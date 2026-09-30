//! Checks `META-INF/container.xml`, the file that tells a reading system
//! where the package document (the `.opf`) lives.

use crate::paths::{case_hint, resolve_href};
use crate::report::Report;
use crate::xml::{children_named, is_element, parse};
use std::collections::BTreeSet;

pub const CONTAINER_FILE: &str = "META-INF/container.xml";

const CONTAINER_NAMESPACE: &str = "urn:oasis:names:tc:opendocument:xmlns:container";
const PACKAGE_MEDIA_TYPE: &str = "application/oebps-package+xml";

/// Validate the text of container.xml.
/// Returns the paths of the package documents it points to (usually just one).
pub fn check_container(
    xml_text: &str,
    all_files: &BTreeSet<String>,
    report: &mut Report,
) -> Vec<String> {
    let document = match parse(xml_text) {
        Ok(document) => document,
        Err(error) => {
            report.add_error(format!(
                "`{CONTAINER_FILE}` is not well-formed XML: {error}"
            ));
            return Vec::new();
        }
    };

    let container = document.root_element();
    if !is_element(container, "container", CONTAINER_NAMESPACE) {
        report.add_error(format!(
            "`{CONTAINER_FILE}` root element must be <container> in namespace `{CONTAINER_NAMESPACE}`"
        ));
        return Vec::new();
    }
    if container.attribute("version") != Some("1.0") {
        report.add_error(format!(
            "<container> in `{CONTAINER_FILE}` must have version=\"1.0\""
        ));
    }

    // Rust note: `let ... else { ... };` unpacks a value, and if it doesn't
    // match (here: no <rootfiles> element found) runs the `else` block, which
    // must leave the function (return, continue, break, ...).
    let Some(rootfiles) = children_named(container, "rootfiles", CONTAINER_NAMESPACE).next() else {
        report.add_error(format!("`{CONTAINER_FILE}` has no <rootfiles> element"));
        return Vec::new();
    };

    let rootfile_elements: Vec<_> =
        children_named(rootfiles, "rootfile", CONTAINER_NAMESPACE).collect();
    if rootfile_elements.is_empty() {
        report.add_error(format!(
            "`{CONTAINER_FILE}` must contain at least one <rootfile>"
        ));
    }

    let mut package_paths = Vec::new();
    for rootfile in rootfile_elements {
        let media_type = rootfile.attribute("media-type");
        if media_type != Some(PACKAGE_MEDIA_TYPE) {
            report.add_error(format!(
                "<rootfile> media-type must be `{PACKAGE_MEDIA_TYPE}`, found `{}`",
                media_type.unwrap_or("(missing)")
            ));
        }

        let Some(full_path) = rootfile.attribute("full-path") else {
            report.add_error("<rootfile> is missing the full-path attribute");
            continue; // move on to the next <rootfile>
        };

        // container.xml paths are relative to the container root, so the
        // base directory is "" (nothing).
        match resolve_href("", full_path) {
            Err(problem) => {
                report.add_error(format!("<rootfile full-path=\"{full_path}\"> {problem}"));
            }
            Ok(package_path) if all_files.contains(&package_path) => {
                package_paths.push(package_path);
            }
            Ok(package_path) => {
                report.add_error(format!(
                    "<rootfile full-path=\"{full_path}\"> points to a file that does not exist.{}",
                    case_hint(all_files, &package_path)
                ));
            }
        }
    }
    package_paths
}
