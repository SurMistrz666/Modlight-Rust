//! mammoth `lib/docx/office-xml-reader.js`: reading one XML part with the
//! OOXML namespace prefixes, and collapsing `mc:AlternateContent` to its
//! fallback.

use super::super::Failure;
use super::super::jszip::ZipFile;
use super::super::xml::reader::{NamespaceMap, read_string};
use super::super::xml::{XmlElement, XmlNode};

pub static XML_NAMESPACE_MAP: NamespaceMap = &[
    // Transitional format
    ("http://schemas.openxmlformats.org/wordprocessingml/2006/main", "w"),
    ("http://schemas.openxmlformats.org/officeDocument/2006/relationships", "r"),
    ("http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing", "wp"),
    ("http://schemas.openxmlformats.org/drawingml/2006/main", "a"),
    ("http://schemas.openxmlformats.org/drawingml/2006/picture", "pic"),
    // Strict format
    ("http://purl.oclc.org/ooxml/wordprocessingml/main", "w"),
    ("http://purl.oclc.org/ooxml/officeDocument/relationships", "r"),
    ("http://purl.oclc.org/ooxml/drawingml/wordprocessingDrawing", "wp"),
    ("http://purl.oclc.org/ooxml/drawingml/main", "a"),
    ("http://purl.oclc.org/ooxml/drawingml/picture", "pic"),
    // Common
    ("http://schemas.openxmlformats.org/package/2006/content-types", "content-types"),
    ("http://schemas.openxmlformats.org/package/2006/relationships", "relationships"),
    ("http://schemas.openxmlformats.org/markup-compatibility/2006", "mc"),
    ("urn:schemas-microsoft-com:vml", "v"),
    ("urn:schemas-microsoft-com:office:word", "office-word"),
    // [MS-DOCX]: Word Extensions to the Office Open XML (.docx) File Format
    // https://learn.microsoft.com/en-us/openspecs/office_standards/ms-docx/b839fe1f-e1ca-4fa6-8c26-5954d0abbccd
    ("http://schemas.microsoft.com/office/word/2010/wordml", "wordml"),
];

/// `TextDecoder("utf-8").decode(bytes)`: a leading BOM consumed, invalid
/// sequences replaced.
pub fn decode_utf8(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    String::from_utf8_lossy(bytes).into_owned()
}

fn collapse_alternate_content(node: XmlNode) -> Vec<XmlNode> {
    match node {
        XmlNode::Element(mut element) => {
            if element.name == "mc:AlternateContent" {
                // The fallback's children are NOT collapsed in turn.
                return match element
                    .children
                    .into_iter()
                    .filter_map(|c| match c {
                        XmlNode::Element(e) if e.name == "mc:Fallback" => Some(e),
                        _ => None,
                    })
                    .next()
                {
                    Some(fallback) => fallback.children,
                    None => Vec::new(),
                };
            }
            let children = std::mem::take(&mut element.children);
            element.children = children.into_iter().flat_map(collapse_alternate_content).collect();
            vec![XmlNode::Element(element)]
        }
        text => vec![text],
    }
}

/// `read(xmlString)`: the root node after collapsing (`None` is the
/// `undefined` an `mc:AlternateContent` root with an empty fallback gives).
pub fn read(xml_string: &str, max_depth: usize) -> Result<Option<XmlNode>, Failure> {
    let root = read_string(xml_string, XML_NAMESPACE_MAP, max_depth)?;
    Ok(collapse_alternate_content(XmlNode::Element(root)).into_iter().next())
}

/// `readXmlFromZipFile(docxFile, path)`: `None` when the part does not
/// exist; `Some(None)` for the `undefined` root above.
pub fn read_xml_from_zip_file(
    docx_file: &ZipFile,
    path: &str,
    max_depth: usize,
) -> Result<Option<Option<XmlNode>>, Failure> {
    if !docx_file.exists(path) {
        return Ok(None);
    }
    let bytes = docx_file.read(path).map_err(|e| Failure::new(e.0))?;
    let text = decode_utf8(&bytes);
    // stripUtf8Bom: one more U+FEFF.
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(&text);
    read(text, max_depth).map(Some)
}

/// The element a reader is handed: `undefined` (no element) or a text node
/// make every reader throw, except where the caller tests truthiness first.
pub fn require_element(node: &XmlNode) -> Result<&XmlElement, Failure> {
    node.as_element().ok_or_else(|| Failure::new("expected an element, got a text node"))
}
