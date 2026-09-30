//! Helpers for the paths *inside* the EPUB.
//!
//! Inside an EPUB, paths always use `/`, are relative to the folder root and
//! are case sensitive. That is why this module uses plain `String`s such as
//! "OEBPS/text/chapter1.xhtml" rather than `std::path::Path`.

use crate::report::Report;
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::io;
use std::path::Path;

// ---------------------------------------------------------------------------
// Finding the files
// ---------------------------------------------------------------------------

/// Every file under `folder`, as `/`-separated paths relative to `folder`.
/// A `BTreeSet` keeps them unique *and* sorted, which makes output predictable.
///
/// A name that is not valid Unicode is added to `report` as an error and then
/// skipped, so the user still sees every other problem in the same run.
pub fn list_files(folder: &Path, report: &mut Report) -> io::Result<BTreeSet<String>> {
    let mut files = BTreeSet::new();
    collect_files(folder, "", &mut files, report)?;
    Ok(files)
}

fn collect_files(
    directory: &Path,
    path_prefix: &str,
    files: &mut BTreeSet<String>,
    report: &mut Report,
) -> io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;

        // `into_string` gives the name back as an `Err` if it isn't valid Unicode.
        let name = match entry.file_name().into_string() {
            Ok(name) => name,
            Err(bad_name) => {
                let location = if path_prefix.is_empty() {
                    "the folder root"
                } else {
                    path_prefix
                };
                report.add_error(format!(
                    "a name in `{location}` is not valid Unicode (EPUB requires UTF-8 names): {bad_name:?}"
                ));
                continue; // skip this entry and carry on with the next one
            }
        };

        let relative_path = if path_prefix.is_empty() {
            name.clone()
        } else {
            format!("{path_prefix}/{name}")
        };

        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            if name != ".git" {
                collect_files(&entry.path(), &relative_path, files, report)?;
            }
        } else if file_type.is_file() {
            files.insert(relative_path);
        }
    }
    Ok(())
}

/// EPUB paths are case sensitive but Windows is not, so a wrongly-cased name
/// "works" on your PC and then breaks in reading systems. If a file exists
/// that differs only by case, return a sentence pointing it out.
pub fn case_hint(all_files: &BTreeSet<String>, wanted_path: &str) -> String {
    let wanted_lowercase = wanted_path.to_lowercase();
    let similar_file = all_files
        .iter()
        .find(|path| path.to_lowercase() == wanted_lowercase);

    match similar_file {
        Some(actual_path) if actual_path != wanted_path => {
            format!(" A file named `{actual_path}` exists, but EPUB paths are case sensitive.")
        }
        _ => String::new(),
    }
}

// ---------------------------------------------------------------------------
// Turning an `href` from the XML into a path in the container
// ---------------------------------------------------------------------------

/// Convert an `href` found in a file inside `base_directory` into a clean path
/// relative to the container root. It:
///   * drops any `#fragment`,
///   * decodes `%XX` escapes (`%27` becomes `'`),
///   * resolves `.` and `..`,
///   * refuses anything that would climb out of the container.
///
/// The `Err` text is a fragment like "is empty" so callers can prefix it with
/// context ("item `c1` href `x` is empty").
pub fn resolve_href(base_directory: &str, href: &str) -> Result<String, String> {
    let href_without_fragment = href.split('#').next().unwrap_or("");

    if href_without_fragment.is_empty() {
        return Err("is empty".into());
    }
    if href_without_fragment.starts_with('/') {
        return Err("must be a relative path (no leading `/`)".into());
    }

    let decoded = percent_decode(href_without_fragment)?;
    let full_path = if base_directory.is_empty() {
        decoded
    } else {
        format!("{base_directory}/{decoded}")
    };

    let mut segments: Vec<&str> = Vec::new();
    for segment in full_path.split('/') {
        match segment {
            "" | "." => {} // nothing to do
            ".." => {
                // Go up one directory; if there is none, we'd leave the container.
                if segments.pop().is_none() {
                    return Err("climbs out of the container with `..`".into());
                }
            }
            normal_segment => segments.push(normal_segment),
        }
    }

    if segments.is_empty() {
        return Err("does not name a file".into());
    }
    Ok(segments.join("/"))
}

/// Decode `%XX` escapes, e.g. "dai%27er.xhtml" becomes "dai'er.xhtml".
fn percent_decode(text: &str) -> Result<String, String> {
    let bytes = text.as_bytes();
    let mut decoded_bytes = Vec::with_capacity(bytes.len());
    let mut position = 0;

    while position < bytes.len() {
        if bytes[position] == b'%' {
            // The two characters after the `%` must be hexadecimal digits.
            let byte = text
                .get(position + 1..position + 3)
                .filter(|hex| hex.chars().all(|c| c.is_ascii_hexdigit()))
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
                .ok_or_else(|| format!("contains an invalid %-escape (`{text}`)"))?;
            decoded_bytes.push(byte);
            position += 3;
        } else {
            decoded_bytes.push(bytes[position]);
            position += 1;
        }
    }

    String::from_utf8(decoded_bytes).map_err(|_| "decodes to invalid UTF-8".to_string())
}

/// Does this href start with a URL scheme such as `http:` or `data:`?
pub fn has_url_scheme(href: &str) -> bool {
    let Some((scheme, _rest)) = href.split_once(':') else {
        return false;
    };
    let mut characters = scheme.chars();
    let starts_with_letter = characters.next().is_some_and(|c| c.is_ascii_alphabetic());
    starts_with_letter
        && characters.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

// ---------------------------------------------------------------------------
// The spec's rules for file names
// ---------------------------------------------------------------------------

pub fn check_file_names(paths: &BTreeSet<String>, report: &mut Report) {
    // Maps each path (lowercased) to the first real path seen with that
    // spelling, so we can spot two files that differ only by upper/lower case.
    // We lowercase the WHOLE path, directories included: on a case-insensitive
    // system `OEBPS/Text/a.xhtml` and `OEBPS/text/a.xhtml` are the same file.
    let mut seen_paths: HashMap<String, String> = HashMap::new();

    for path in paths {
        if path.len() > 65535 {
            report.add_error(format!("path is longer than 65535 bytes: `{path}`"));
        }

        for name in path.split('/') {
            check_one_file_name(name, path, report);
        }

        if let Some(other_path) = seen_paths.insert(path.to_lowercase(), path.clone()) {
            report.add_error(format!(
                "`{path}` and `{other_path}` differ only by case; names must be unique after case folding"
            ));
        }
    }
}

fn check_one_file_name(name: &str, full_path: &str, report: &mut Report) {
    if name.len() > 255 {
        report.add_error(format!(
            "file name is longer than 255 bytes in `{full_path}`"
        ));
    }
    if name.ends_with('.') {
        report.add_error(format!(
            "file name must not end with a full stop: `{full_path}`"
        ));
    }
    if let Some(bad_character) = name.chars().find(|c| is_forbidden_in_file_name(*c)) {
        report.add_error(format!(
            "file name contains the forbidden character U+{:04X} in `{full_path}`",
            bad_character as u32
        ));
    }
    if name.contains(' ') {
        report.add_warning(format!(
            "`{full_path}` contains a space; the spec says file names SHOULD NOT"
        ));
    }
}

/// The characters the spec forbids in file names (section 4.2.3).
fn is_forbidden_in_file_name(character: char) -> bool {
    let code_point = character as u32;

    let is_reserved_symbol = matches!(character, '"' | '*' | ':' | '<' | '>' | '?' | '\\' | '|');
    let is_control_character = code_point <= 0x1F || (0x7F..=0x9F).contains(&code_point);
    let is_private_use =
        (0xE000..=0xF8FF).contains(&code_point) || (0xF0000..=0x10FFFF).contains(&code_point);
    let is_non_character = (0xFDD0..=0xFDEF).contains(&code_point)
        || (0xFFF0..=0xFFFF).contains(&code_point)
        // The last two code points of every Unicode plane (U+1FFFE, U+1FFFF, ...).
        || (code_point & 0xFFFE) == 0xFFFE;

    is_reserved_symbol || is_control_character || is_private_use || is_non_character
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a set of paths from string literals.
    fn path_set(paths: &[&str]) -> BTreeSet<String> {
        paths.iter().map(|path| path.to_string()).collect()
    }

    #[test]
    fn resolve_href_is_relative_to_the_base_directory() {
        assert_eq!(
            resolve_href("OEBPS", "text/ch1.xhtml").unwrap(),
            "OEBPS/text/ch1.xhtml"
        );
    }

    #[test]
    fn resolve_href_follows_parent_segments() {
        assert_eq!(
            resolve_href("OEBPS/text", "../images/cover.jpg").unwrap(),
            "OEBPS/images/cover.jpg"
        );
    }

    #[test]
    fn resolve_href_refuses_to_leave_the_container() {
        assert!(resolve_href("OEBPS", "../../etc/passwd").is_err());
        assert!(resolve_href("", "../secret").is_err());
        // Escapes hidden inside percent-encoding must be caught too.
        assert!(resolve_href("OEBPS", "%2E%2E/%2E%2E/secret").is_err());
    }

    #[test]
    fn resolve_href_decodes_escapes_and_drops_fragments() {
        assert_eq!(
            resolve_href("OEBPS", "dai%27er.xhtml#top").unwrap(),
            "OEBPS/dai'er.xhtml"
        );
    }

    #[test]
    fn resolve_href_rejects_absolute_and_empty_paths() {
        assert!(resolve_href("OEBPS", "/etc/passwd").is_err());
        assert!(resolve_href("OEBPS", "").is_err());
        assert!(resolve_href("OEBPS", "#only-a-fragment").is_err());
    }

    #[test]
    fn resolve_href_rejects_malformed_percent_escapes() {
        for bad_href in ["%zz.xhtml", "%+f.xhtml", "%4", "%"] {
            assert!(
                resolve_href("", bad_href).is_err(),
                "{bad_href} should fail"
            );
        }
    }

    #[test]
    fn url_schemes_are_detected() {
        assert!(has_url_scheme("http://example.com/font.woff"));
        assert!(has_url_scheme("data:text/plain,hi"));
        assert!(!has_url_scheme("text/ch1.xhtml"));
        assert!(!has_url_scheme(":starts-with-a-colon"));
        assert!(!has_url_scheme("1abc:starts-with-a-digit"));
    }

    #[test]
    fn names_that_differ_only_by_case_in_the_file_name_are_an_error() {
        let mut report = Report::default();
        check_file_names(&path_set(&["OEBPS/A.xhtml", "OEBPS/a.xhtml"]), &mut report);
        assert_eq!(report.errors.len(), 1);
    }

    #[test]
    fn names_that_differ_only_by_case_in_a_directory_are_an_error() {
        let mut report = Report::default();
        let paths = path_set(&["OEBPS/Text/ch1.xhtml", "OEBPS/text/ch1.xhtml"]);
        check_file_names(&paths, &mut report);
        assert_eq!(report.errors.len(), 1);
    }

    #[test]
    fn ordinary_names_pass_without_errors() {
        let mut report = Report::default();
        let paths = path_set(&["OEBPS/text/ch1.xhtml", "OEBPS/text/ch2.xhtml", "mimetype"]);
        check_file_names(&paths, &mut report);
        assert!(report.errors.is_empty());
        assert!(report.warnings.is_empty());
    }

    #[test]
    fn forbidden_characters_and_trailing_dots_are_errors() {
        for bad_path in [
            "OEBPS/a:b.xhtml",
            "OEBPS/what?.xhtml",
            "OEBPS/ends-with-dot.",
        ] {
            let mut report = Report::default();
            check_file_names(&path_set(&[bad_path]), &mut report);
            assert!(report.has_errors(), "{bad_path} should be rejected");
        }
    }

    #[test]
    fn a_space_in_a_name_is_only_a_warning() {
        let mut report = Report::default();
        check_file_names(&path_set(&["OEBPS/my chapter.xhtml"]), &mut report);
        assert!(report.errors.is_empty());
        assert_eq!(report.warnings.len(), 1);
    }
}
