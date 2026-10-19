//! mammoth `lib/html/`: the HTML tree, `simplify`, and `write`.

pub mod ast;
pub mod simplify;

use super::Failure;
use super::writers::html_writer::HtmlWriter;
use ast::{HtmlNode, is_void_element};

/// `write(writer, nodes)`.
pub fn write(writer: &mut HtmlWriter, nodes: &[HtmlNode]) -> Result<(), Failure> {
    for node in nodes {
        match node {
            HtmlNode::Element { tag, children } => {
                if is_void_element(tag, children) {
                    writer.self_closing(&tag.tag_name, &tag.attributes)?;
                } else {
                    writer.open(&tag.tag_name, &tag.attributes)?;
                    write(writer, children)?;
                    writer.close(&tag.tag_name);
                }
            }
            HtmlNode::Text(value) => writer.text(value),
            HtmlNode::ForceWrite => {}
            // Replaced before writing; `toStrings["deferred"]` would throw.
            HtmlNode::Deferred { .. } => return Err(Failure::new("toStrings[node.type] is not a function")),
        }
    }
    Ok(())
}
