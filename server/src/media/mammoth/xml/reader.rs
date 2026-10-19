//! mammoth `lib/xml/reader.js`: xmldom's document turned into mammoth's
//! element tree. Only elements and text survive (comments, CDATA sections and
//! processing instructions are dropped); names become `prefix:local` through
//! the namespace map, `{uri}local` for an unmapped namespace, and the bare
//! local name when there is no namespace at all.

use super::super::Failure;
use super::super::js;
use super::nodes::{Attributes, XmlElement, XmlNode};
use super::xmldom::{self, DomChild, DomElement, NsValue};

/// A namespace map: URI -> prefix (a plain object, looked up by the URI).
pub type NamespaceMap = &'static [(&'static str, &'static str)];

fn convert_name(ns: &NsValue, local_name: &str, map: NamespaceMap) -> String {
    let uri = match ns {
        NsValue::Undefined => return local_name.to_owned(),
        NsValue::Str(s) if s.is_empty() => return local_name.to_owned(),
        NsValue::Str(s) => s.clone(),
        // `namespaceMap[fn]` looks up the function's source text.
        NsValue::Builtin(name) => js::builtin_to_string(name),
    };
    if let Some((_, prefix)) = map.iter().find(|(u, _)| *u == uri) {
        return format!("{prefix}:{local_name}");
    }
    if let Some(builtin) = js::builtin(&uri) {
        // An inherited member is a truthy "prefix": its string form is used.
        return format!("{}:{local_name}", js::builtin_to_string(builtin));
    }
    format!("{{{uri}}}{local_name}")
}

fn convert_element(element: DomElement, map: NamespaceMap) -> XmlElement {
    let name = convert_name(&element.ns, &element.local_name, map);
    let mut children = Vec::with_capacity(element.children.len());
    for child in element.children {
        match child {
            DomChild::Element(e) => children.push(XmlNode::Element(convert_element(e, map))),
            DomChild::Text(t) => children.push(XmlNode::Text(t)),
            DomChild::Other => {}
        }
    }
    let mut attributes = Attributes::new();
    for a in element.attrs {
        attributes.set(convert_name(&a.ns, &a.local_name, map), a.value);
    }
    XmlElement { name, attributes, children }
}

/// `readString(xmlString, namespaceMap)`: the document element.
pub fn read_string(xml_string: &str, map: NamespaceMap, max_depth: usize) -> Result<XmlElement, Failure> {
    let root = xmldom::parse_from_string(xml_string, max_depth).map_err(|e| Failure::new(e.0))?;
    Ok(convert_element(root, map))
}
