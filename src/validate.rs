//! Looks at the folder and decides (a) whether it is a valid EPUB and
//! (b) exactly which files should go into the zip.
//!
//! Nothing in here writes files. It only produces a `Validation`.

use crate::container::{CONTAINER_FILE, check_container};
use crate::package::check_package;
use crate::paths::{case_hint, check_file_names, list_files};
use crate::report::Report;
use crate::xml::read_text_file;
use std::collections::BTreeSet;
use std::io;
use std::path::Path;

pub const MIMETYPE_FILE: &str = "mimetype";
pub const MIMETYPE_CONTENT: &str = "application/epub+zip";

pub struct Validation {
    pub report: Report,
    /// Paths (relative to the folder, using `/`) in the order they must be
    /// written into the zip. `mimetype` is always first.
    pub files_to_pack: Vec<String>,
}

/// Inspect `folder`.
///
/// `output_file` is the path of the .epub we are about to create, relative to
/// `folder` (if it lives inside it), so we never mistake our own output for
/// a stray file.
pub fn validate(folder: &Path, output_file: Option<&str>) -> io::Result<Validation> {
    let mut report = Report::default();
    let all_files = list_files(folder, &mut report)?;

    check_mimetype(folder, &all_files, &mut report);

    let package_paths = check_container_file(folder, &all_files, &mut report);
    let publication_files = check_package_files(folder, &package_paths, &all_files, &mut report);

    let files_to_pack = choose_files_to_pack(&all_files, &publication_files);

    // If we couldn't even find a package document, a list of "skipped" files
    // would just be noise on top of the real error.
    if !package_paths.is_empty() {
        warn_about_skipped_files(&all_files, &files_to_pack, output_file, &mut report);
    }

    check_file_names(&files_to_pack, &mut report);

    Ok(Validation {
        report,
        files_to_pack: order_for_zip(files_to_pack),
    })
}

// ---------------------------------------------------------------------------
// The mimetype file
// ---------------------------------------------------------------------------

fn check_mimetype(folder: &Path, all_files: &BTreeSet<String>, report: &mut Report) {
    if !all_files.contains(MIMETYPE_FILE) {
        report.add_error(format!(
            "`{MIMETYPE_FILE}` is missing from the folder root (REQUIRED).{}",
            case_hint(all_files, MIMETYPE_FILE)
        ));
        return;
    }

    let bytes = match std::fs::read(folder.join(MIMETYPE_FILE)) {
        Ok(bytes) => bytes,
        Err(error) => {
            report.add_error(format!("could not read `{MIMETYPE_FILE}`: {error}"));
            return;
        }
    };

    if bytes == MIMETYPE_CONTENT.as_bytes() {
        return; // exactly right
    }

    // It's wrong, so work out *how* it is wrong to give a helpful message.
    let text = String::from_utf8_lossy(&bytes);
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        report.add_error("`mimetype` starts with a UTF-8 byte order mark; it must not");
    } else if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        report.add_error("`mimetype` is UTF-16; it must be plain US-ASCII");
    } else if text.trim() == MIMETYPE_CONTENT {
        report.add_error(
            "`mimetype` has leading/trailing whitespace or a trailing newline. \
             It must be exactly `application/epub+zip` (20 bytes) with nothing else",
        );
    } else {
        report.add_error(format!(
            "`mimetype` must contain exactly `{MIMETYPE_CONTENT}` but contains `{}`",
            text.trim()
        ));
    }
}

// ---------------------------------------------------------------------------
// container.xml and the package documents
// ---------------------------------------------------------------------------

/// Reads and checks META-INF/container.xml. Returns the package document paths.
fn check_container_file(
    folder: &Path,
    all_files: &BTreeSet<String>,
    report: &mut Report,
) -> Vec<String> {
    if !all_files.contains(CONTAINER_FILE) {
        report.add_error(format!(
            "`{CONTAINER_FILE}` is missing (REQUIRED by the OCF spec).{}",
            case_hint(all_files, CONTAINER_FILE)
        ));
        return Vec::new();
    }

    match read_text_file(folder, CONTAINER_FILE) {
        Ok(xml_text) => check_container(&xml_text, all_files, report),
        Err(problem) => {
            report.add_error(format!("`{CONTAINER_FILE}` {problem}"));
            Vec::new()
        }
    }
}

/// Checks every package document. Returns the package documents themselves
/// plus every file their manifests list: the "publication files".
fn check_package_files(
    folder: &Path,
    package_paths: &[String],
    all_files: &BTreeSet<String>,
    report: &mut Report,
) -> BTreeSet<String> {
    let mut publication_files = BTreeSet::new();

    for package_path in package_paths {
        publication_files.insert(package_path.clone());

        match read_text_file(folder, package_path) {
            Ok(xml_text) => {
                let manifest_files =
                    check_package(folder, package_path, &xml_text, all_files, report);
                publication_files.extend(manifest_files);
            }
            Err(problem) => report.add_error(format!("`{package_path}` {problem}")),
        }
    }
    publication_files
}

// ---------------------------------------------------------------------------
// Deciding what goes in the zip
// ---------------------------------------------------------------------------

/// The zip gets: mimetype, everything in META-INF, and the publication files.
/// Anything else in the folder (build scripts, tree.txt, notes...) is not part
/// of the book and is left out.
fn choose_files_to_pack(
    all_files: &BTreeSet<String>,
    publication_files: &BTreeSet<String>,
) -> BTreeSet<String> {
    all_files
        .iter()
        .filter(|path| {
            *path == MIMETYPE_FILE
                || path.starts_with("META-INF/")
                || publication_files.contains(*path)
        })
        .cloned()
        .collect()
}

fn warn_about_skipped_files(
    all_files: &BTreeSet<String>,
    files_to_pack: &BTreeSet<String>,
    output_file: Option<&str>,
    report: &mut Report,
) {
    let skipped: Vec<&String> = all_files
        .iter()
        .filter(|path| !files_to_pack.contains(*path))
        .filter(|path| Some(path.as_str()) != output_file)
        .filter(|path| !path.ends_with(".epub.tmp"))
        .collect();

    if skipped.is_empty() {
        return;
    }

    const HOW_MANY_TO_SHOW: usize = 8;
    let shown: Vec<&str> = skipped
        .iter()
        .take(HOW_MANY_TO_SHOW)
        .map(|path| path.as_str())
        .collect();

    let mut message = format!(
        "{} file(s) are not in the manifest and will NOT be packed: {}",
        skipped.len(),
        shown.join(", ")
    );
    if skipped.len() > HOW_MANY_TO_SHOW {
        message.push_str(&format!(" (+{} more)", skipped.len() - HOW_MANY_TO_SHOW));
    }
    report.add_warning(message);
}

/// The spec requires `mimetype` to be the first entry in the zip. We put
/// container.xml second and sort the rest, so the same input always produces
/// exactly the same output file.
fn order_for_zip(mut files: BTreeSet<String>) -> Vec<String> {
    let mut ordered = Vec::with_capacity(files.len());

    for first_file in [MIMETYPE_FILE, CONTAINER_FILE] {
        if files.remove(first_file) {
            ordered.push(first_file.to_string());
        }
    }
    ordered.extend(files); // a BTreeSet hands its items out in sorted order
    ordered
}
