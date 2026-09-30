//! Checks a package document (the `.opf` file): its metadata, its manifest
//! (the list of every file in the book) and its spine (the reading order).

use crate::paths::{case_hint, has_url_scheme, resolve_href};
use crate::report::Report;
use crate::xml::{children_named, is_element, parse, read_text_file};
use roxmltree::Node;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const PACKAGE_NAMESPACE: &str = "http://www.idpf.org/2007/opf";
const DUBLIN_CORE_NAMESPACE: &str = "http://purl.org/dc/elements/1.1/";
const EPUB_OPS_NAMESPACE: &str = "http://www.idpf.org/2007/ops";

const XHTML: &str = "application/xhtml+xml";
const SVG: &str = "image/svg+xml";
const NCX: &str = "application/x-dtbncx+xml";

/// Manifest files with these media types are XML, so they must be well-formed.
const XML_MEDIA_TYPES: [&str; 4] = [XHTML, SVG, NCX, "application/smil+xml"];

#[derive(Clone, Copy, PartialEq)]
enum EpubVersion {
    Two,
    Three,
}

/// One `<item>` from the manifest.
struct ManifestItem {
    /// Path inside the container, or `None` if the item is remote or invalid.
    local_path: Option<String>,
    media_type: String,
    properties: Vec<String>,
    has_fallback: bool,
}

/// The whole manifest. A `BTreeMap` (sorted by item id) keeps our error
/// messages in a stable order from run to run.
#[derive(Default)]
struct Manifest {
    items: BTreeMap<String, ManifestItem>,
    /// Paths of all local files the manifest lists.
    local_files: Vec<String>,
}

/// Validate one package document.
/// Returns the paths of the local files listed in its manifest.
pub fn check_package(
    folder: &Path,
    package_path: &str,
    xml_text: &str,
    all_files: &BTreeSet<String>,
    report: &mut Report,
) -> Vec<String> {
    let document = match parse(xml_text) {
        Ok(document) => document,
        Err(error) => {
            report.add_error(format!("`{package_path}` is not well-formed XML: {error}"));
            return Vec::new();
        }
    };

    let package = document.root_element();
    if !is_element(package, "package", PACKAGE_NAMESPACE) {
        report.add_error(format!(
            "`{package_path}` root element must be <package> in namespace `{PACKAGE_NAMESPACE}`"
        ));
        return Vec::new();
    }

    let Some(version) = read_version(package, package_path, report) else {
        return Vec::new();
    };

    // ---- metadata ----
    match opf_child(package, "metadata") {
        Some(metadata) => check_metadata(metadata, package, version, package_path, report),
        None => report.add_error(format!("`{package_path}` has no <metadata> element")),
    }

    // ---- manifest ----
    let manifest = match opf_child(package, "manifest") {
        Some(manifest_element) => read_manifest(manifest_element, package_path, all_files, report),
        None => {
            report.add_error(format!("`{package_path}` has no <manifest> element"));
            Manifest::default()
        }
    };
    check_xml_files_are_well_formed(folder, &manifest, report);
    if version == EpubVersion::Three {
        check_navigation_item(folder, &manifest, package_path, report);
    }

    // ---- spine ----
    match opf_child(package, "spine") {
        Some(spine) => check_spine(spine, &manifest, version, package_path, report),
        None => report.add_error(format!("`{package_path}` has no <spine> element")),
    }

    manifest.local_files
}

/// The first child of `parent` with this name, in the OPF namespace.
fn opf_child<'a, 'input: 'a>(parent: Node<'a, 'input>, name: &'a str) -> Option<Node<'a, 'input>> {
    children_named(parent, name, PACKAGE_NAMESPACE).next()
}

fn read_version(package: Node, package_path: &str, report: &mut Report) -> Option<EpubVersion> {
    let version_text = package.attribute("version").unwrap_or("");

    if version_text.starts_with("3.") {
        Some(EpubVersion::Three)
    } else if version_text == "2.0" || version_text == "2.0.1" {
        Some(EpubVersion::Two)
    } else {
        report.add_error(format!(
            "`{package_path}`: <package version=\"{version_text}\"> is not supported (expected 2.0 or 3.x)"
        ));
        None
    }
}

// ---------------------------------------------------------------------------
// Metadata
// ---------------------------------------------------------------------------

fn check_metadata(
    metadata: Node,
    package: Node,
    version: EpubVersion,
    package_path: &str,
    report: &mut Report,
) {
    let identifiers: Vec<Node> =
        children_named(metadata, "identifier", DUBLIN_CORE_NAMESPACE).collect();
    if identifiers.is_empty() {
        report.add_error(format!(
            "`{package_path}`: <dc:identifier> is REQUIRED in metadata"
        ));
    }
    check_unique_identifier(package, &identifiers, package_path, report);

    for (element_name, label) in [("title", "dc:title"), ("language", "dc:language")] {
        let has_non_empty_value = children_named(metadata, element_name, DUBLIN_CORE_NAMESPACE)
            .any(|element| !element.text().unwrap_or("").trim().is_empty());
        if !has_non_empty_value {
            report.add_error(format!(
                "`{package_path}`: <{label}> is REQUIRED in metadata (and must not be empty)"
            ));
        }
    }

    if version == EpubVersion::Three {
        check_modified_date(metadata, package_path, report);
    }
}

/// `<package unique-identifier="X">` must name the id of one <dc:identifier>.
fn check_unique_identifier(
    package: Node,
    identifiers: &[Node],
    package_path: &str,
    report: &mut Report,
) {
    let Some(unique_id) = package.attribute("unique-identifier") else {
        report.add_error(format!(
            "`{package_path}`: <package> needs a unique-identifier attribute"
        ));
        return;
    };

    let matching_identifier = identifiers
        .iter()
        .find(|identifier| identifier.attribute("id") == Some(unique_id));

    match matching_identifier {
        None => report.add_error(format!(
            "`{package_path}`: unique-identifier=\"{unique_id}\" does not match the id of any <dc:identifier>"
        )),
        Some(identifier) if identifier.text().unwrap_or("").trim().is_empty() => {
            report.add_error(format!("`{package_path}`: the unique <dc:identifier> is empty"));
        }
        Some(_) => {} // all good
    }
}

/// EPUB 3 requires exactly one `<meta property="dcterms:modified">`
/// whose value looks like 2026-09-30T12:00:00Z.
fn check_modified_date(metadata: Node, package_path: &str, report: &mut Report) {
    let modified_values: Vec<&str> = children_named(metadata, "meta", PACKAGE_NAMESPACE)
        .filter(|meta| meta.attribute("property") == Some("dcterms:modified"))
        .map(|meta| meta.text().unwrap_or("").trim())
        .collect();

    match modified_values.as_slice() {
        [] => report.add_error(format!(
            "`{package_path}`: EPUB 3 REQUIRES <meta property=\"dcterms:modified\">"
        )),
        [value] if !is_valid_modified_date(value) => report.add_error(format!(
            "`{package_path}`: dcterms:modified `{value}` must look like 2026-09-30T12:00:00Z"
        )),
        [_] => {} // exactly one, and valid
        _ => report.add_error(format!(
            "`{package_path}`: dcterms:modified must appear exactly once"
        )),
    }
}

/// Checks the shape CCYY-MM-DDThh:mm:ssZ (it does not check the date is real).
fn is_valid_modified_date(text: &str) -> bool {
    // '9' = a digit, anything else must match that exact character.
    let pattern = "9999-99-99T99:99:99Z";
    text.len() == pattern.len()
        && text.chars().zip(pattern.chars()).all(|(actual, expected)| {
            if expected == '9' {
                actual.is_ascii_digit()
            } else {
                actual == expected
            }
        })
}

// ---------------------------------------------------------------------------
// Manifest
// ---------------------------------------------------------------------------

fn read_manifest(
    manifest_element: Node,
    package_path: &str,
    all_files: &BTreeSet<String>,
    report: &mut Report,
) -> Manifest {
    let mut manifest = Manifest::default();

    for item in children_named(manifest_element, "item", PACKAGE_NAMESPACE) {
        let (Some(id), Some(href), Some(media_type)) = (
            item.attribute("id"),
            item.attribute("href"),
            item.attribute("media-type"),
        ) else {
            report.add_error(format!(
                "`{package_path}`: every manifest <item> needs id, href and media-type (bad item near byte {})",
                item.range().start
            ));
            continue;
        };

        if manifest.items.contains_key(id) {
            report.add_error(format!(
                "`{package_path}`: duplicate manifest item id `{id}`"
            ));
            continue;
        }

        let local_path = locate_item_file(id, href, media_type, package_path, all_files, report);
        if let Some(path) = &local_path {
            manifest.local_files.push(path.clone());
        }

        let properties = item
            .attribute("properties")
            .unwrap_or("")
            .split_whitespace()
            .map(String::from)
            .collect();

        manifest.items.insert(
            id.to_string(),
            ManifestItem {
                local_path,
                media_type: media_type.to_string(),
                properties,
                has_fallback: item.attribute("fallback").is_some(),
            },
        );
    }

    if manifest.items.is_empty() {
        report.add_error(format!(
            "`{package_path}`: <manifest> has no <item> elements"
        ));
    }
    manifest
}

/// Work out where a manifest item's file is, and check it is allowed to be there.
/// Returns its path in the container, or `None` for remote or invalid items
/// (after reporting any problem).
fn locate_item_file(
    id: &str,
    href: &str,
    media_type: &str,
    package_path: &str,
    all_files: &BTreeSet<String>,
    report: &mut Report,
) -> Option<String> {
    if has_url_scheme(href) {
        check_remote_item(id, href, media_type, package_path, report);
        return None;
    }

    // hrefs in the package document are relative to the package document's folder.
    let package_directory = package_path
        .rsplit_once('/')
        .map_or("", |(directory, _)| directory);

    let path = match resolve_href(package_directory, href) {
        Ok(path) => path,
        Err(problem) => {
            report.add_error(format!(
                "`{package_path}`: item `{id}` href `{href}` {problem}"
            ));
            return None;
        }
    };

    if path == "mimetype" || path.starts_with("META-INF/") {
        report.add_error(format!(
            "`{package_path}`: item `{id}` lists `{path}`; mimetype and META-INF files must NOT be in the manifest"
        ));
        return None;
    }
    if !all_files.contains(&path) {
        report.add_error(format!(
            "`{package_path}`: item `{id}` points to `{path}`, which does not exist.{}",
            case_hint(all_files, &path)
        ));
        return None;
    }
    Some(path)
}

/// Only audio, video and fonts may be hosted outside the EPUB.
fn check_remote_item(
    id: &str,
    href: &str,
    media_type: &str,
    package_path: &str,
    report: &mut Report,
) {
    if href.starts_with("data:") || href.starts_with("file:") {
        report.add_error(format!(
            "`{package_path}`: item `{id}` uses a data:/file: URL in href, which is forbidden"
        ));
        return;
    }

    let may_be_remote = media_type.starts_with("audio/")
        || media_type.starts_with("video/")
        || media_type.starts_with("font/")
        || media_type.contains("font")
        || media_type == "application/vnd.ms-opentype";
    if !may_be_remote {
        report.add_error(format!(
            "`{package_path}`: item `{id}` is remote (`{href}`) but only audio, video and font resources may live outside the container"
        ));
    }
}

/// XML-based files in the manifest must be well-formed XML.
fn check_xml_files_are_well_formed(folder: &Path, manifest: &Manifest, report: &mut Report) {
    for (id, item) in &manifest.items {
        let Some(path) = &item.local_path else {
            continue;
        };
        if !XML_MEDIA_TYPES.contains(&item.media_type.as_str()) {
            continue;
        }

        match read_text_file(folder, path) {
            Ok(text) => {
                if let Err(error) = parse(&text) {
                    report.add_error(format!(
                        "`{path}` (manifest id `{id}`) is not well-formed XML: {error}"
                    ));
                }
            }
            // UTF-16 XML is legal but this tool can't read it, so don't fail the build.
            Err(problem) => {
                report.add_warning(format!(
                    "`{path}` {problem}; skipped the well-formedness check"
                ));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// EPUB 3 navigation document
// ---------------------------------------------------------------------------

/// EPUB 3 needs exactly one manifest item marked `properties="nav"`, and that
/// file must contain a `<nav epub:type="toc">` table of contents.
fn check_navigation_item(
    folder: &Path,
    manifest: &Manifest,
    package_path: &str,
    report: &mut Report,
) {
    let nav_items: Vec<(&String, &ManifestItem)> = manifest
        .items
        .iter()
        .filter(|(_, item)| item.properties.iter().any(|property| property == "nav"))
        .collect();

    match nav_items.as_slice() {
        [] => report.add_error(format!(
            "`{package_path}`: EPUB 3 REQUIRES exactly one manifest item with properties=\"nav\""
        )),
        [(id, nav_item)] => {
            if nav_item.media_type != XHTML {
                report.add_error(format!(
                    "`{package_path}`: nav item `{id}` must have media-type {XHTML}"
                ));
            } else if let Some(path) = &nav_item.local_path {
                check_nav_has_toc(folder, path, report);
            }
        }
        _ => report.add_error(format!(
            "`{package_path}`: more than one manifest item has properties=\"nav\""
        )),
    }
}

fn check_nav_has_toc(folder: &Path, nav_path: &str, report: &mut Report) {
    let Ok(text) = read_text_file(folder, nav_path) else {
        return;
    };
    let Ok(document) = parse(&text) else { return }; // already reported elsewhere

    let has_toc = document.descendants().any(|node| {
        let is_nav_element = node.is_element() && node.tag_name().name() == "nav";
        let epub_type = node.attribute((EPUB_OPS_NAMESPACE, "type")).unwrap_or("");
        is_nav_element && epub_type.split_whitespace().any(|word| word == "toc")
    });

    if !has_toc {
        report.add_error(format!(
            "navigation document `{nav_path}` has no <nav epub:type=\"toc\"> element"
        ));
    }
}

// ---------------------------------------------------------------------------
// Spine
// ---------------------------------------------------------------------------

fn check_spine(
    spine: Node,
    manifest: &Manifest,
    version: EpubVersion,
    package_path: &str,
    report: &mut Report,
) {
    let itemrefs: Vec<Node> = children_named(spine, "itemref", PACKAGE_NAMESPACE).collect();
    if itemrefs.is_empty() {
        report.add_error(format!(
            "`{package_path}`: <spine> needs at least one <itemref>"
        ));
    }

    for itemref in itemrefs {
        let Some(idref) = itemref.attribute("idref") else {
            report.add_error(format!("`{package_path}`: <itemref> is missing idref"));
            continue;
        };

        let Some(item) = manifest.items.get(idref) else {
            report.add_error(format!(
                "`{package_path}`: spine idref `{idref}` is not in the manifest"
            ));
            continue;
        };

        let is_content_document = item.media_type == XHTML || item.media_type == SVG;
        if !is_content_document && !item.has_fallback {
            report.add_error(format!(
                "`{package_path}`: spine item `{idref}` has media-type `{}`; only XHTML/SVG may be in the spine without a manifest fallback",
                item.media_type
            ));
        }
    }

    check_spine_toc_attribute(spine, manifest, version, package_path, report);
}

/// `<spine toc="ncx-id">` points at the legacy NCX table of contents.
/// EPUB 2 requires it; in EPUB 3 it is optional but must be right if present.
fn check_spine_toc_attribute(
    spine: Node,
    manifest: &Manifest,
    version: EpubVersion,
    package_path: &str,
    report: &mut Report,
) {
    let Some(toc_id) = spine.attribute("toc") else {
        if version == EpubVersion::Two {
            report.add_error(format!(
                "`{package_path}`: EPUB 2 REQUIRES <spine toc=\"...\"> pointing at the NCX"
            ));
        }
        return;
    };

    match manifest.items.get(toc_id) {
        Some(item) if item.media_type == NCX => {} // correct
        Some(_) => report.add_error(format!(
            "`{package_path}`: spine toc=\"{toc_id}\" must reference an item with media-type {NCX}"
        )),
        None => report.add_error(format!(
            "`{package_path}`: spine toc=\"{toc_id}\" is not in the manifest"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modified_date_accepts_the_required_shape() {
        assert!(is_valid_modified_date("2026-09-30T12:00:00Z"));
    }

    #[test]
    fn modified_date_rejects_other_shapes() {
        for bad_date in [
            "",
            "2026-09-30",
            "2026-09-30T12:00:00",       // missing the Z
            "2026-09-30T12:00:00+01:00", // offsets are not allowed
            "2026-09-30 12:00:00Z",      // space instead of T
            "26-09-30T12:00:00Z",
            "2026-09-30T12:00:0aZ",
        ] {
            assert!(
                !is_valid_modified_date(bad_date),
                "{bad_date:?} should fail"
            );
        }
    }
}
