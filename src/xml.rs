//! Small helpers around the `roxmltree` XML parser.

use roxmltree::{Document, Node, ParsingOptions};
use std::fs;
use std::path::Path;

/// Parse XML text into a document, or return the parser's error message.
pub fn parse(xml_text: &str) -> Result<Document<'_>, roxmltree::Error> {
    let options = ParsingOptions {
        // XHTML files very often start with a <!DOCTYPE ...> line, so allow it.
        allow_dtd: true,
        ..ParsingOptions::default()
    };
    Document::parse_with_options(xml_text, options)
}

/// Read a file as UTF-8 text, ignoring a leading byte order mark (BOM).
///
/// On failure the error is a *fragment* such as "is not valid UTF-8", written
/// so the caller can put the file name in front of it.
pub fn read_text_file(folder: &Path, relative_path: &str) -> Result<String, String> {
    let bytes = fs::read(folder.join(relative_path))
        .map_err(|error| format!("could not be read: {error}"))?;

    let utf8_bom = [0xEF, 0xBB, 0xBF];
    let bytes_without_bom = bytes.strip_prefix(&utf8_bom).unwrap_or(&bytes);

    String::from_utf8(bytes_without_bom.to_vec()).map_err(|_| "is not valid UTF-8".to_string())
}

/// Is this node an element with exactly this local name *and* namespace?
///
/// XML namespaces are what tell an OPF `<package>` apart from any other
/// element called "package", so we always compare both.
pub fn is_element(node: Node, local_name: &str, namespace: &str) -> bool {
    node.is_element()
        && node.tag_name().name() == local_name
        && node.tag_name().namespace() == Some(namespace)
}

/// All direct children of `parent` with this local name and namespace.
///
/// Rust note: the `'a` and `'input` markers are *lifetimes*. They tell the
/// compiler that the nodes we hand back borrow from the parsed document, so
/// the document must stay alive for as long as you are using them.
pub fn children_named<'a, 'input: 'a>(
    parent: Node<'a, 'input>,
    local_name: &'a str,
    namespace: &'a str,
) -> impl Iterator<Item = Node<'a, 'input>> + 'a {
    parent
        .children()
        .filter(move |child| is_element(*child, local_name, namespace))
}
