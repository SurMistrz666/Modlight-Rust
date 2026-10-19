//! mammoth `lib/images.js`: `imgElement(func)`, the wrapper docx.ts hands
//! its `convertImage` to.

use super::html::ast::{HtmlNode, fresh_element};
use super::styles::html_paths::AttrValue;

/// The `img` element for what `func` resolved to: `alt` first when the
/// image has alternative text, then the result's keys (`_.extend`, so an
/// `alt` in the result keeps the first position).
pub fn img_element(alt_text: Option<&str>, result: Vec<(String, String)>) -> HtmlNode {
    let mut attributes: Vec<(String, AttrValue)> = Vec::new();
    if let Some(alt) = alt_text.filter(|a| !a.is_empty()) {
        attributes.push(("alt".into(), AttrValue::Str(alt.to_owned())));
    }
    for (key, value) in result {
        match attributes.iter_mut().find(|(k, _)| *k == key) {
            Some(slot) => slot.1 = AttrValue::Str(value),
            None => attributes.push((key, AttrValue::Str(value))),
        }
    }
    fresh_element("img", attributes, Vec::new())
}
