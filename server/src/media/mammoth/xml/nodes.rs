//! mammoth `lib/xml/nodes.js`: the element tree the readers walk.

use super::super::Failure;

#[derive(Debug, Clone)]
pub enum XmlNode {
    Element(XmlElement),
    Text(String),
}

impl XmlNode {
    pub fn as_element(&self) -> Option<&XmlElement> {
        match self {
            XmlNode::Element(e) => Some(e),
            XmlNode::Text(_) => None,
        }
    }
}

/// An element's attributes (a plain object keyed by the converted name: a
/// later attribute with the same name replaces the value, `__proto__` is
/// never stored).
#[derive(Debug, Clone, Default)]
pub struct Attributes {
    entries: Vec<(String, String)>,
}

impl Attributes {
    pub const fn new() -> Self {
        Attributes { entries: Vec::new() }
    }

    pub fn set(&mut self, name: String, value: String) {
        if name == "__proto__" {
            return;
        }
        match self.entries.iter_mut().find(|(n, _)| *n == name) {
            Some(slot) => slot.1 = value,
            None => self.entries.push((name, value)),
        }
    }

    /// `element.attributes[name]` (`None` is `undefined`).
    pub fn get(&self, name: &str) -> Option<&str> {
        self.entries.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }
}

/// `Element`.
#[derive(Debug, Clone, Default)]
pub struct XmlElement {
    pub name: String,
    pub attributes: Attributes,
    pub children: Vec<XmlNode>,
}

/// `emptyElement`: what `firstOrEmpty` returns when nothing matches.
pub static EMPTY_ELEMENT: XmlElement =
    XmlElement { name: String::new(), attributes: Attributes::new(), children: Vec::new() };

impl XmlElement {
    /// `element.attributes[name]`.
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attributes.get(name)
    }

    /// `first(name)`: the first child element with that name.
    pub fn first(&self, name: &str) -> Option<&XmlElement> {
        self.children.iter().filter_map(XmlNode::as_element).find(|e| e.name == name)
    }

    /// `firstOrEmpty(name)`.
    pub fn first_or_empty(&self, name: &str) -> &XmlElement {
        self.first(name).unwrap_or(&EMPTY_ELEMENT)
    }

    /// `getElementsByTagName(name)`: the CHILDREN with that name (not all
    /// descendants).
    pub fn get_elements_by_tag_name(&self, name: &str) -> Vec<&XmlElement> {
        self.children.iter().filter_map(XmlNode::as_element).filter(|e| e.name == name).collect()
    }

    /// `text()`: "" without children, the text of a single text child,
    /// otherwise "Not implemented".
    pub fn text(&self) -> Result<String, Failure> {
        match self.children.as_slice() {
            [] => Ok(String::new()),
            [XmlNode::Text(t)] => Ok(t.clone()),
            _ => Err(Failure::new("Not implemented")),
        }
    }
}

/// `elementList.getElementsByTagName(name)` on a list.
pub fn elements_named<'a>(list: &[&'a XmlElement], name: &str) -> Vec<&'a XmlElement> {
    list.iter().flat_map(|e| e.get_elements_by_tag_name(name)).collect()
}
