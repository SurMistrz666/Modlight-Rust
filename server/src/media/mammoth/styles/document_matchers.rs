//! mammoth `lib/styles/document-matchers.js`.

use super::super::documents::{BreakType, Numbering, Paragraph, Run, Table};
use super::super::js::string_to_number;

#[derive(Debug, Clone)]
pub enum StyleName {
    EqualTo(String),
    StartsWith(String),
}

impl StyleName {
    fn matches(&self, style_name: &str) -> bool {
        match self {
            // first.toUpperCase() === second.toUpperCase()
            StyleName::EqualTo(operand) => operand.to_uppercase() == style_name.to_uppercase(),
            // second.toUpperCase().indexOf(first.toUpperCase()) === 0
            StyleName::StartsWith(operand) => style_name.to_uppercase().starts_with(&operand.to_uppercase()),
        }
    }
}

#[derive(Debug, Clone)]
pub enum DocumentMatcher {
    /// `Matcher(elementType, options)`.
    Element {
        element_type: &'static str,
        style_id: Option<String>,
        style_name: Option<StyleName>,
        /// (levelIndex, isOrdered)
        list: Option<(f64, bool)>,
    },
    Highlight {
        color: Option<String>,
    },
    Break {
        break_type: Option<BreakType>,
    },
}

/// What `findStyle(element)` is asked about.
pub enum MatchTarget<'a> {
    Paragraph(&'a Paragraph),
    Run(&'a Run),
    Table(&'a Table),
    Highlight(&'a str),
    /// `{type: "bold"}` and the other run properties.
    Property(&'static str),
    Break(BreakType),
    CommentReference,
}

impl MatchTarget<'_> {
    fn type_name(&self) -> &'static str {
        match self {
            MatchTarget::Paragraph(_) => "paragraph",
            MatchTarget::Run(_) => "run",
            MatchTarget::Table(_) => "table",
            MatchTarget::Highlight(_) => "highlight",
            MatchTarget::Property(p) => p,
            MatchTarget::Break(_) => "break",
            MatchTarget::CommentReference => "commentReference",
        }
    }

    fn style_id(&self) -> Option<&str> {
        match self {
            MatchTarget::Paragraph(p) => p.style_id.as_deref(),
            MatchTarget::Run(r) => r.style_id.as_deref(),
            MatchTarget::Table(t) => t.style_id.as_deref(),
            _ => None,
        }
    }

    fn style_name(&self) -> Option<&str> {
        match self {
            MatchTarget::Paragraph(p) => p.style_name.as_deref(),
            MatchTarget::Run(r) => r.style_name.as_deref(),
            MatchTarget::Table(t) => t.style_name.as_deref(),
            _ => None,
        }
    }
}

impl DocumentMatcher {
    pub fn matches(&self, element: &MatchTarget<'_>) -> bool {
        match self {
            DocumentMatcher::Element { element_type, style_id, style_name, list } => {
                element.type_name() == *element_type
                    && style_id.as_ref().is_none_or(|id| element.style_id() == Some(id.as_str()))
                    && style_name
                        .as_ref()
                        .is_none_or(|m| element.style_name().filter(|n| !n.is_empty()).is_some_and(|n| m.matches(n)))
                    && list.is_none_or(|(level_index, is_ordered)| is_list(element, level_index, is_ordered))
            }
            DocumentMatcher::Highlight { color } => match element {
                MatchTarget::Highlight(c) => color.as_ref().is_none_or(|want| want == c),
                _ => false,
            },
            DocumentMatcher::Break { break_type } => match element {
                MatchTarget::Break(b) => break_type.is_none_or(|want| want == *b),
                _ => false,
            },
        }
    }
}

/// `isList(element, levelIndex, isOrdered)`: `level == levelIndex` is a loose
/// comparison of the `w:ilvl` string with a number.
fn is_list(element: &MatchTarget<'_>, level_index: f64, is_ordered: bool) -> bool {
    match element {
        MatchTarget::Paragraph(Paragraph { numbering: Some(Numbering::Level { is_ordered: o, level }), .. }) => {
            string_to_number(level) == level_index && *o == is_ordered
        }
        _ => false,
    }
}
