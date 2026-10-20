//! mammoth `lib/docx/comments-reader.js`.

use super::super::Failure;
use super::super::documents::Comment;
use super::super::js::js_trim;
use super::super::xml::XmlElement;
use super::body_reader::{BodyReaderOptions, read_xml_elements};

pub fn read_comments_xml(options: &BodyReaderOptions<'_>, element: &XmlElement) -> Result<Vec<Comment>, Failure> {
    let mut comments = Vec::new();
    for comment_element in element.get_elements_by_tag_name("w:comment") {
        let id = comment_element.attr("w:id").map(str::to_owned);
        let read_optional_attribute = |name: &str| -> Option<String> {
            let v = js_trim(comment_element.attr(name).unwrap_or(""));
            (!v.is_empty()).then(|| v.to_owned())
        };
        let body = read_xml_elements(options, &comment_element.children)?;
        comments.push(Comment {
            comment_id: id,
            body: body.value,
            author_name: read_optional_attribute("w:author"),
            author_initials: read_optional_attribute("w:initials"),
        });
    }
    Ok(comments)
}
