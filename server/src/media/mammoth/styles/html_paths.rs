//! mammoth `lib/styles/html-paths.js`.

use std::rc::Rc;

use super::super::js;

/// An HTML attribute value: normally a string; `undefined` or an inherited
/// `Object.prototype` member when a hyperlink's relationship is missing (the
/// writer throws on those).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttrValue {
    Str(String),
    Undefined,
    Builtin(&'static str),
}

/// `Element` of html-paths.js: what an HTML node is tagged with.
#[derive(Debug, Clone)]
pub struct HtmlTag {
    pub tag_name: String,
    /// `tagNames` (a plain object of the alternatives, `ul|ol`).
    pub tag_names: Vec<String>,
    pub attributes: Vec<(String, AttrValue)>,
    pub fresh: bool,
    pub separator: Option<String>,
}

impl HtmlTag {
    /// `element(tagName, attributes, { fresh })`.
    pub fn new(tag_name: &str, attributes: Vec<(String, AttrValue)>, fresh: bool) -> HtmlTag {
        HtmlTag {
            tag_name: tag_name.to_owned(),
            tag_names: vec![tag_name.to_owned()],
            attributes,
            fresh,
            separator: None,
        }
    }

    /// `matchesElement(element)`: `this.tagNames[element.tagName]` (an
    /// inherited member counts) and equal attribute objects.
    pub fn matches_element(&self, other: &HtmlTag) -> bool {
        let name_matches = self.tag_names.contains(&other.tag_name) || js::builtin(&other.tag_name).is_some();
        name_matches && attributes_equal(&self.attributes, &other.attributes)
    }
}

/// `_.isEqual(a, b)` of two attribute objects.
fn attributes_equal(a: &[(String, AttrValue)], b: &[(String, AttrValue)]) -> bool {
    a.len() == b.len() && a.iter().all(|(k, v)| b.iter().any(|(k2, v2)| k == k2 && v == v2))
}

/// `HtmlPath` (or `ignore`).
#[derive(Debug, Clone)]
pub enum HtmlPath {
    Elements(Vec<Rc<HtmlTag>>),
    /// `!`: the content is dropped without being converted.
    Ignore,
}

impl HtmlPath {
    /// `empty`.
    pub fn empty() -> HtmlPath {
        HtmlPath::Elements(Vec::new())
    }

    /// `topLevelElement(tagName)`.
    pub fn top_level_element(tag_name: &str) -> HtmlPath {
        HtmlPath::Elements(vec![Rc::new(HtmlTag::new(tag_name, Vec::new(), true))])
    }
}
