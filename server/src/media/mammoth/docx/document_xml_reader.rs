//! mammoth `lib/docx/document-xml-reader.js`.

use super::super::Failure;
use super::super::documents::{Comment, Document, Note};
use super::super::xml::XmlElement;
use super::body_reader::{BodyReaderOptions, read_xml_elements};

/// `convertXmlToDocument(element)`.
pub fn convert_xml_to_document(
    options: &BodyReaderOptions<'_>,
    element: &XmlElement,
    notes: Vec<Note>,
    comments: Vec<Comment>,
) -> Result<Document, Failure> {
    let Some(body) = element.first("w:body") else {
        return Err(Failure::new("Could not find the body element: are you sure this is a docx file?"));
    };
    let result = read_xml_elements(options, &body.children)?;
    Ok(Document { children: result.value, notes, comments })
}
