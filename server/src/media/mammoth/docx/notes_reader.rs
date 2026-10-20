//! mammoth `lib/docx/notes-reader.js`.

use super::super::Failure;
use super::super::documents::{Note, NoteType};
use super::super::xml::XmlElement;
use super::body_reader::{BodyReaderOptions, read_xml_elements};

/// `createFootnotesReader` / `createEndnotesReader`, applied.
pub fn read_notes_xml(
    note_type: NoteType,
    options: &BodyReaderOptions<'_>,
    element: &XmlElement,
) -> Result<Vec<Note>, Failure> {
    let tag = format!("w:{}", note_type.as_str());
    let mut notes = Vec::new();
    for footnote_element in element.get_elements_by_tag_name(&tag) {
        let ty = footnote_element.attr("w:type");
        if ty == Some("continuationSeparator") || ty == Some("separator") {
            continue;
        }
        let id = footnote_element.attr("w:id").map(str::to_owned);
        let body = read_xml_elements(options, &footnote_element.children)?;
        // `.map(...)` keeps only the value: extras hoisted out of a note's
        // top level are lost.
        notes.push(Note { note_type, note_id: id, body: body.value });
    }
    Ok(notes)
}
