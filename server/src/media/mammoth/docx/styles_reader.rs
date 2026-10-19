//! mammoth `lib/docx/styles-reader.js`.
//!
//! The style sets are plain objects keyed by style id. A style whose
//! `w:type` names an `Object.prototype` member is not stored here; upstream
//! writes it onto that member (for `w:type="__proto__"` onto
//! `Object.prototype` itself, which changes every object in the process for
//! the rest of its life: prototype pollution, not reproduced).

use super::super::Failure;
use super::super::js::{self, JsObject, Lookup};
use super::super::xml::XmlElement;

#[derive(Debug, Clone)]
pub struct Style {
    pub name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NumberingStyle {
    pub num_id: Option<String>,
}

/// What `find*StyleById(id)` returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StyleLookup {
    /// A style (or an inherited member, whose `.name` is its own name): its
    /// name.
    Found(Option<String>),
    Missing,
}

/// `Styles`. `None` is a set the default (no styles part) does not have:
/// looking anything up in it is a `TypeError`.
#[derive(Debug, Clone)]
pub struct Styles {
    paragraph: Option<JsObject<Style>>,
    character: Option<JsObject<Style>>,
    table: Option<JsObject<Style>>,
    numbering: Option<JsObject<NumberingStyle>>,
}

fn find(set: &Option<JsObject<Style>>, id: &str) -> Result<StyleLookup, Failure> {
    let set = set.as_ref().ok_or_else(|| Failure::new("Cannot read properties of undefined (reading styleId)"))?;
    Ok(match set.get(id) {
        Lookup::Own(style) => StyleLookup::Found(style.name.clone()),
        Lookup::Builtin(b) => StyleLookup::Found(js::builtin_name(b)),
        Lookup::Missing => StyleLookup::Missing,
    })
}

/// `findNumberingStyleById(id).numId`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NumberingStyleLookup {
    NumId(Option<String>),
    Missing,
}

impl Styles {
    pub fn find_paragraph_style_by_id(&self, id: &str) -> Result<StyleLookup, Failure> {
        find(&self.paragraph, id)
    }

    pub fn find_character_style_by_id(&self, id: &str) -> Result<StyleLookup, Failure> {
        find(&self.character, id)
    }

    pub fn find_table_style_by_id(&self, id: &str) -> Result<StyleLookup, Failure> {
        find(&self.table, id)
    }

    pub fn find_numbering_style_by_id(&self, id: &str) -> Result<NumberingStyleLookup, Failure> {
        let set = self
            .numbering
            .as_ref()
            .ok_or_else(|| Failure::new("Cannot read properties of undefined (reading styleId)"))?;
        Ok(match set.get(id) {
            Lookup::Own(style) => NumberingStyleLookup::NumId(style.num_id.clone()),
            // A function has no `numId`.
            Lookup::Builtin(_) => NumberingStyleLookup::NumId(None),
            Lookup::Missing => NumberingStyleLookup::Missing,
        })
    }
}

/// `defaultStyles`: `new Styles({}, {})`.
pub fn default_styles() -> Styles {
    Styles { paragraph: Some(JsObject::new()), character: Some(JsObject::new()), table: None, numbering: None }
}

/// `readStylesXml(root)`.
pub fn read_styles_xml(root: &XmlElement) -> Styles {
    let mut paragraph = JsObject::new();
    let mut character = JsObject::new();
    let mut table = JsObject::new();
    let mut numbering: JsObject<NumberingStyle> = JsObject::new();

    for style_element in root.get_elements_by_tag_name("w:style") {
        let ty = style_element.attr("w:type");
        let style_id = style_element.attr("w:styleId").unwrap_or("undefined");

        // Per 17.7.4.17 style (Style Definition) of ECMA-376 4th edition Part 1:
        //
        // > If multiple style definitions each declare the same value for their
        // > styleId, then the first such instance shall keep its current
        // > identifier with all other instances being reassigned in any manner
        // > desired.
        //
        // For the purpose of conversion, there's no point holding onto styles
        // with reassigned style IDs, so we ignore such style definitions.
        if ty == Some("numbering") {
            let num_id = style_element
                .first_or_empty("w:pPr")
                .first_or_empty("w:numPr")
                .first_or_empty("w:numId")
                .attr("w:val")
                .map(str::to_owned);
            if numbering.get(style_id).is_missing() {
                numbering.set(style_id, NumberingStyle { num_id });
            }
            continue;
        }
        let set = match ty {
            Some("paragraph") => &mut paragraph,
            Some("character") => &mut character,
            Some("table") => &mut table,
            _ => continue,
        };
        let name = style_element.first("w:name").and_then(|n| n.attr("w:val")).map(str::to_owned);
        if set.get(style_id).is_missing() {
            set.set(style_id, Style { name });
        }
    }

    Styles { paragraph: Some(paragraph), character: Some(character), table: Some(table), numbering: Some(numbering) }
}
