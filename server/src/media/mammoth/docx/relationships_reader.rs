//! mammoth `lib/docx/relationships-reader.js`.

use std::collections::HashMap;

use super::super::Failure;
use super::super::documents::JsVal;
use super::super::js::{self, JsObject, Lookup};
use super::super::xml::XmlElement;

/// `Relationships`.
#[derive(Debug, Clone, Default)]
pub struct Relationships {
    /// `targetsByRelationshipId` (a plain object; a missing `Target` is
    /// `undefined`).
    by_id: JsObject<Option<String>>,
    /// `targetsByType`, in document order.
    by_type: HashMap<String, Vec<Option<String>>>,
}

impl Relationships {
    fn new(relationships: Vec<(Option<String>, Option<String>, Option<String>)>) -> Result<Self, Failure> {
        let mut by_id = JsObject::new();
        for (id, target, _) in &relationships {
            by_id.set_primitive(id.as_deref().unwrap_or("undefined"), target.clone());
        }
        let mut by_type: HashMap<String, Vec<Option<String>>> = HashMap::new();
        for (_, target, ty) in relationships {
            let key = ty.unwrap_or_else(|| "undefined".into());
            // `targetsByType[type]` finds an inherited member (truthy), so no
            // array is created and `.push` is not a function.
            if js::builtin(&key).is_some() {
                return Err(Failure::new("targetsByType[relationship.type].push is not a function"));
            }
            by_type.entry(key).or_default().push(target);
        }
        Ok(Relationships { by_id, by_type })
    }

    /// `findTargetByRelationshipId(id)`.
    pub fn find_target_by_relationship_id(&self, id: &str) -> JsVal {
        match self.by_id.get(id) {
            Lookup::Own(Some(target)) => JsVal::Str(target.clone()),
            Lookup::Own(None) | Lookup::Missing => JsVal::Undefined,
            Lookup::Builtin(b) => JsVal::Builtin(b),
        }
    }

    /// `findTargetsByType(type)`.
    pub fn find_targets_by_type(&self, ty: &str) -> Vec<Option<String>> {
        self.by_type.get(ty).cloned().unwrap_or_default()
    }
}

/// `readRelationships(element)`.
pub fn read_relationships(element: &XmlElement) -> Result<Relationships, Failure> {
    let mut relationships = Vec::new();
    for child in element.children.iter().filter_map(|c| c.as_element()) {
        if child.name == "relationships:Relationship" {
            relationships.push((
                child.attr("Id").map(str::to_owned),
                child.attr("Target").map(str::to_owned),
                child.attr("Type").map(str::to_owned),
            ));
        }
    }
    Relationships::new(relationships)
}

/// `defaultValue`.
pub fn default_value() -> Relationships {
    Relationships::default()
}
