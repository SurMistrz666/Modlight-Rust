//! mammoth `lib/html/ast.js`.

use std::rc::Rc;

use super::super::documents::Image;
use super::super::js;
use super::super::styles::html_paths::{AttrValue, HtmlTag};

#[derive(Debug, Clone)]
pub enum HtmlNode {
    Element {
        tag: Rc<HtmlTag>,
        children: Vec<HtmlNode>,
    },
    Text(String),
    /// `forceWrite`: keeps an otherwise empty element.
    ForceWrite,
    /// An image waiting for `convertImage` (`deferredConversion`).
    Deferred {
        id: usize,
        image: Image,
    },
}

/// `freshElement(tagName, attributes, children)`.
pub fn fresh_element(tag_name: &str, attributes: Vec<(String, AttrValue)>, children: Vec<HtmlNode>) -> HtmlNode {
    HtmlNode::Element { tag: Rc::new(HtmlTag::new(tag_name, attributes, true)), children }
}

/// `nonFreshElement(tagName, attributes, children)`.
pub fn non_fresh_element(tag_name: &str, attributes: Vec<(String, AttrValue)>, children: Vec<HtmlNode>) -> HtmlNode {
    HtmlNode::Element { tag: Rc::new(HtmlTag::new(tag_name, attributes, false)), children }
}

/// `elementWithTag(tag, children)`.
pub fn element_with_tag(tag: Rc<HtmlTag>, children: Vec<HtmlNode>) -> HtmlNode {
    HtmlNode::Element { tag, children }
}

/// `text(value)`.
pub fn text(value: impl Into<String>) -> HtmlNode {
    HtmlNode::Text(value.into())
}

/// `isVoidElement(node)`: no children and a void tag name (`voidTagNames`
/// is a plain object: an inherited member's name counts).
pub fn is_void_element(tag: &HtmlTag, children: &[HtmlNode]) -> bool {
    children.is_empty()
        && (matches!(tag.tag_name.as_str(), "br" | "hr" | "img" | "input") || js::builtin(&tag.tag_name).is_some())
}
