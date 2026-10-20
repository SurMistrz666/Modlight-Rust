//! mammoth `lib/docx/numbering-xml.js`.

use super::super::Failure;
use super::super::documents::Numbering as NumberingValue;
use super::super::js::{JsObject, Lookup};
use super::super::xml::XmlElement;
use super::styles_reader::{NumberingStyleLookup, Styles};

#[derive(Debug, Clone)]
pub struct Level {
    pub is_ordered: bool,
    pub level: String,
    pub paragraph_style_id: Option<String>,
}

#[derive(Debug, Clone)]
struct Num {
    abstract_num_id: Option<String>,
}

#[derive(Debug, Clone)]
struct AbstractNum {
    levels: JsObject<Level>,
    num_style_link: Option<String>,
}

/// What `findLevel` / `findLevelByParagraphStyleId` return.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LevelResult {
    Null,
    Undefined,
    Found(NumberingValue),
}

impl LevelResult {
    fn level(level: &Level) -> LevelResult {
        LevelResult::Found(NumberingValue::Level { is_ordered: level.is_ordered, level: level.level.clone() })
    }

    /// `numbering || null`.
    pub fn into_option(self) -> Option<NumberingValue> {
        match self {
            LevelResult::Found(n) => Some(n),
            _ => None,
        }
    }
}

/// `Numbering`.
#[derive(Debug, Clone)]
pub struct Numbering {
    nums: JsObject<Num>,
    abstract_nums: JsObject<AbstractNum>,
    /// `None` for `defaultNumbering`, which is built without styles.
    styles: Option<Styles>,
    levels_by_paragraph_style_id: JsObject<Level>,
}

impl Numbering {
    fn new(nums: JsObject<Num>, abstract_nums: JsObject<AbstractNum>, styles: Option<Styles>) -> Numbering {
        let mut levels_by_paragraph_style_id = JsObject::new();
        for abstract_num in abstract_nums.values() {
            for level in abstract_num.levels.values() {
                if let Some(id) = &level.paragraph_style_id {
                    levels_by_paragraph_style_id.set(id, level.clone());
                }
            }
        }
        Numbering { nums, abstract_nums, styles, levels_by_paragraph_style_id }
    }

    /// `findLevel(numId, level)`.
    pub fn find_level(&self, num_id: &str, level: &str) -> Result<LevelResult, Failure> {
        let mut seen: JsObject<bool> = JsObject::new();
        self.find_level_with_seen_num_ids(num_id.to_owned(), level, &mut seen)
    }

    fn find_level_with_seen_num_ids(
        &self,
        num_id: String,
        level: &str,
        seen: &mut JsObject<bool>,
    ) -> Result<LevelResult, Failure> {
        if !seen.get(&num_id).is_missing() {
            return Ok(LevelResult::Null);
        }
        seen.set_primitive(&num_id, true);

        let abstract_num_id = match self.nums.get(&num_id) {
            Lookup::Own(num) => num.abstract_num_id.clone(),
            Lookup::Builtin(_) => None,
            Lookup::Missing => return Ok(LevelResult::Null),
        };
        let key = abstract_num_id.unwrap_or_else(|| "undefined".into());
        let abstract_num = match self.abstract_nums.get(&key) {
            Lookup::Own(a) => a,
            // A function: `.numStyleLink` is undefined, `.levels[level]`
            // reads a property of undefined.
            Lookup::Builtin(_) => return Err(Failure::new("Cannot read properties of undefined (reading level)")),
            Lookup::Missing => return Ok(LevelResult::Null),
        };
        match &abstract_num.num_style_link {
            None => Ok(match abstract_num.levels.get(level) {
                Lookup::Own(l) => LevelResult::level(l),
                Lookup::Builtin(_) => LevelResult::Found(NumberingValue::Opaque),
                Lookup::Missing => LevelResult::Undefined,
            }),
            Some(link) => {
                let styles = self.styles.as_ref().ok_or_else(|| {
                    Failure::new("Cannot read properties of undefined (reading 'findNumberingStyleById')")
                })?;
                match styles.find_numbering_style_by_id(link)? {
                    NumberingStyleLookup::NumId(id) => {
                        self.find_level_with_seen_num_ids(id.unwrap_or_else(|| "undefined".into()), level, seen)
                    }
                    NumberingStyleLookup::Missing => {
                        Err(Failure::new("Cannot read properties of undefined (reading 'numId')"))
                    }
                }
            }
        }
    }

    /// `findLevelByParagraphStyleId(styleId)`.
    pub fn find_level_by_paragraph_style_id(&self, style_id: &str) -> LevelResult {
        match self.levels_by_paragraph_style_id.get(style_id) {
            Lookup::Own(l) => LevelResult::level(l),
            Lookup::Builtin(_) => LevelResult::Found(NumberingValue::Opaque),
            Lookup::Missing => LevelResult::Null,
        }
    }
}

/// `defaultNumbering`.
pub fn default_numbering() -> Numbering {
    Numbering::new(JsObject::new(), JsObject::new(), None)
}

/// `readNumberingXml(root, { styles })`.
pub fn read_numbering_xml(root: &XmlElement, styles: &Styles) -> Result<Numbering, Failure> {
    let mut abstract_nums = JsObject::new();
    for element in root.get_elements_by_tag_name("w:abstractNum") {
        let id = element.attr("w:abstractNumId").unwrap_or("undefined");
        abstract_nums.set(id, read_abstract_num(element));
    }
    let mut nums = JsObject::new();
    for element in root.get_elements_by_tag_name("w:num") {
        let num_id = element.attr("w:numId").unwrap_or("undefined");
        let Some(abstract_num_id) = element.first("w:abstractNumId") else {
            return Err(Failure::new("Cannot read properties of undefined (reading 'attributes')"));
        };
        nums.set(num_id, Num { abstract_num_id: abstract_num_id.attr("w:val").map(str::to_owned) });
    }
    Ok(Numbering::new(nums, abstract_nums, Some(styles.clone())))
}

fn read_abstract_num(element: &XmlElement) -> AbstractNum {
    let mut levels = JsObject::new();

    // Some malformed documents define numbering levels without an index, and
    // reference the numbering using a w:numPr element without a w:ilvl child.
    // To handle such cases, we assume a level of 0 as a fallback.
    let mut level_without_index: Option<Level> = None;

    for level_element in element.get_elements_by_tag_name("w:lvl") {
        let level_index = level_element.attr("w:ilvl");
        let num_fmt = level_element.first_or_empty("w:numFmt").attr("w:val");
        let is_ordered = num_fmt != Some("bullet");
        let paragraph_style_id = level_element.first_or_empty("w:pStyle").attr("w:val").map(str::to_owned);
        match level_index {
            None => level_without_index = Some(Level { is_ordered, level: "0".into(), paragraph_style_id }),
            Some(index) => levels.set(index, Level { is_ordered, level: index.to_owned(), paragraph_style_id }),
        }
    }

    if let Some(level) = level_without_index
        && levels.get(&level.level.clone()).is_missing()
    {
        levels.set("0", level);
    }

    let num_style_link = element.first_or_empty("w:numStyleLink").attr("w:val").map(str::to_owned);
    AbstractNum { levels, num_style_link }
}
