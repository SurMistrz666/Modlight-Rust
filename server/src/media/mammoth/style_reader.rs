//! mammoth `lib/style-reader.js`: one style-map line, `matcher => html path`,
//! parsed with the same lop grammar (see [`super::lop`] for the quirks that
//! decide which lines are accepted).

use std::rc::Rc;

use super::documents::BreakType;
use super::js::string_to_number;
use super::lop::{self, Rule, TokenIterator, Value};
use super::styles::document_matchers::{DocumentMatcher, StyleName};
use super::styles::html_paths::{AttrValue, HtmlTag};
use super::styles::parser::tokeniser::tokenise;

/// An HTML path as parsed: `ignore` (`!`) or elements, one of which may be
/// `undefined` (a separated repetition that ended in an error); wrapping with
/// that one is a `TypeError`.
#[derive(Debug, Clone)]
pub enum ParsedPath {
    Ignore,
    Elements(Vec<Option<Rc<HtmlTag>>>),
}

/// `{ from, to }`; `from` is `None` for `br[type='...']` with an unknown
/// type (`undefined`: consulting it is a `TypeError`).
#[derive(Debug, Clone)]
pub struct StyleMapping {
    pub from: Option<DocumentMatcher>,
    pub to: ParsedPath,
}

#[derive(Debug, Clone, Default)]
struct MatcherOptions {
    style_id: Option<String>,
    style_name: Option<StyleName>,
    list: Option<(f64, bool)>,
}

/// The grammar's own values.
#[derive(Debug, Clone)]
enum Dom {
    /// `documentMatchers.paragraph` / `documentMatchers.run`.
    Factory(&'static str),
    Matcher(DocumentMatcher),
    ListType(bool),
    Options(MatcherOptions),
    Bool(bool),
    Path(ParsedPath),
    PathElement(Rc<HtmlTag>),
    Attribute {
        name: String,
        value: String,
        append: bool,
    },
    Mapping(StyleMapping),
}

type V = Value<Dom>;
type R = Rule<Dom>;

fn tok(name: &'static str) -> R {
    lop::token_of_type(name)
}

fn str_of(v: &V) -> String {
    match v {
        Value::Str(s) => s.clone(),
        _ => "undefined".into(),
    }
}

/// `decodeEscapeSequences(value)`.
fn decode_escape_sequences(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\'
            && let Some(&next) = chars.peek()
            && !matches!(next, '\n' | '\r' | '\u{2028}' | '\u{2029}')
        {
            chars.next();
            match next {
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                other => out.push(other),
            }
            continue;
        }
        out.push(c);
    }
    out
}

fn identifier_rule() -> R {
    lop::then(tok("identifier"), |v| Value::Str(decode_escape_sequences(&str_of(&v))))
}

fn string_rule() -> R {
    lop::then(tok("string"), |v| Value::Str(decode_escape_sequences(&str_of(&v))))
}

fn integer_rule() -> R {
    tok("integer")
}

fn identifier_to_constant(identifier: &'static str, constant: Dom) -> R {
    lop::then(lop::token("identifier", Some(identifier)), move |_| Value::Dom(constant.clone()))
}

fn capture(rule: R) -> R {
    lop::capture(rule, None)
}

fn options_of(v: &V) -> MatcherOptions {
    match v {
        Value::Dom(Dom::Options(o)) => o.clone(),
        _ => MatcherOptions::default(),
    }
}

fn create_matcher_suffixes_rule(rules: Vec<R>) -> R {
    let matcher_suffix = lop::first_of(rules);
    lop::then(lop::zero_or_more(matcher_suffix), |suffixes| {
        let mut merged = MatcherOptions::default();
        if let Value::Array(list) = suffixes {
            for suffix in list {
                // _.extend(matcherOptions, suffix): each suffix sets one key.
                let o = options_of(&suffix);
                if o.style_id.is_some() {
                    merged.style_id = o.style_id;
                }
                if o.style_name.is_some() {
                    merged.style_name = o.style_name;
                }
                if o.list.is_some() {
                    merged.list = o.list;
                }
            }
        }
        Value::Dom(Dom::Options(merged))
    })
}

fn element_matcher(element_type: &'static str, o: MatcherOptions) -> DocumentMatcher {
    DocumentMatcher::Element { element_type, style_id: o.style_id, style_name: o.style_name, list: o.list }
}

fn type_matcher(element_type: &'static str) -> Dom {
    Dom::Matcher(element_matcher(element_type, MatcherOptions::default()))
}

fn document_matcher_rule() -> R {
    let paragraph_rule = identifier_to_constant("p", Dom::Factory("paragraph"));
    let run_rule = identifier_to_constant("r", Dom::Factory("run"));
    let element_type_rule = lop::first_of(vec![paragraph_rule, run_rule]);

    let style_id_rule = || {
        lop::map(vec![tok("dot"), lop::cut(), capture(identifier_rule())], |v| {
            Value::Dom(Dom::Options(MatcherOptions { style_id: Some(str_of(&v[0])), ..Default::default() }))
        })
    };

    let style_name_rule = || {
        let style_name_matcher_rule = lop::first_of(vec![
            lop::then(lop::head(vec![tok("equals"), lop::cut(), capture(string_rule())]), |s| {
                Value::Dom(Dom::Options(MatcherOptions {
                    style_name: Some(StyleName::EqualTo(str_of(&s))),
                    ..Default::default()
                }))
            }),
            lop::then(lop::head(vec![tok("startsWith"), lop::cut(), capture(string_rule())]), |s| {
                Value::Dom(Dom::Options(MatcherOptions {
                    style_name: Some(StyleName::StartsWith(str_of(&s))),
                    ..Default::default()
                }))
            }),
        ]);
        lop::head(vec![
            tok("open-square-bracket"),
            lop::cut(),
            lop::token("identifier", Some("style-name")),
            capture(style_name_matcher_rule),
            tok("close-square-bracket"),
        ])
    };

    let list_type_rule = lop::first_of(vec![
        identifier_to_constant("ordered-list", Dom::ListType(true)),
        identifier_to_constant("unordered-list", Dom::ListType(false)),
    ]);
    let list_rule = lop::map(
        vec![
            tok("colon"),
            capture(list_type_rule),
            lop::cut(),
            tok("open-paren"),
            capture(integer_rule()),
            tok("close-paren"),
        ],
        |v| {
            let is_ordered = matches!(v[0], Value::Dom(Dom::ListType(true)));
            // levelNumber - 1 (the integer token is a string)
            let level_index = string_to_number(&str_of(&v[1])) - 1.0;
            Value::Dom(Dom::Options(MatcherOptions { list: Some((level_index, is_ordered)), ..Default::default() }))
        },
    );

    let paragraph_or_run = lop::map(
        vec![
            capture(element_type_rule),
            capture(create_matcher_suffixes_rule(vec![style_id_rule(), style_name_rule(), list_rule])),
        ],
        |v| match &v[0] {
            Value::Dom(Dom::Factory(t)) => Value::Dom(Dom::Matcher(element_matcher(t, options_of(&v[1])))),
            _ => Value::Undefined,
        },
    );

    let table = lop::map(
        vec![
            lop::token("identifier", Some("table")),
            capture(create_matcher_suffixes_rule(vec![style_id_rule(), style_name_rule()])),
        ],
        |v| Value::Dom(Dom::Matcher(element_matcher("table", options_of(&v[0])))),
    );

    let highlight = lop::map(
        vec![
            lop::token("identifier", Some("highlight")),
            capture(lop::optional(lop::head(vec![
                tok("open-square-bracket"),
                lop::cut(),
                lop::token("identifier", Some("color")),
                tok("equals"),
                capture(string_rule()),
                tok("close-square-bracket"),
            ]))),
        ],
        |v| {
            let color = match &v[0] {
                Value::Opt(Some(c)) => match c.as_ref() {
                    Value::Str(s) => Some(s.clone()),
                    _ => None,
                },
                _ => None,
            };
            Value::Dom(Dom::Matcher(DocumentMatcher::Highlight { color }))
        },
    );

    let break_matcher = lop::map(
        vec![
            lop::token("identifier", Some("br")),
            lop::cut(),
            tok("open-square-bracket"),
            lop::token("identifier", Some("type")),
            tok("equals"),
            capture(string_rule()),
            tok("close-square-bracket"),
        ],
        |v| {
            let break_type = match str_of(&v[0]).as_str() {
                "line" => BreakType::Line,
                "page" => BreakType::Page,
                "column" => BreakType::Column,
                // TODO: handle unknown document matchers
                _ => return Value::Undefined,
            };
            Value::Dom(Dom::Matcher(DocumentMatcher::Break { break_type: Some(break_type) }))
        },
    );

    lop::first_of(vec![
        paragraph_or_run,
        table,
        identifier_to_constant("b", type_matcher("bold")),
        identifier_to_constant("i", type_matcher("italic")),
        identifier_to_constant("u", type_matcher("underline")),
        identifier_to_constant("strike", type_matcher("strikethrough")),
        identifier_to_constant("all-caps", type_matcher("allCaps")),
        identifier_to_constant("small-caps", type_matcher("smallCaps")),
        highlight,
        identifier_to_constant("comment-reference", type_matcher("commentReference")),
        break_matcher,
    ])
}

fn attribute_or_class_rule() -> R {
    let attribute_rule = lop::map(
        vec![
            tok("open-square-bracket"),
            lop::cut(),
            capture(identifier_rule()),
            tok("equals"),
            capture(string_rule()),
            tok("close-square-bracket"),
        ],
        |v| Value::Dom(Dom::Attribute { name: str_of(&v[0]), value: str_of(&v[1]), append: false }),
    );
    let class_rule = lop::map(vec![tok("dot"), lop::cut(), capture(identifier_rule())], |v| {
        Value::Dom(Dom::Attribute { name: "class".into(), value: str_of(&v[0]), append: true })
    });
    lop::first_of(vec![attribute_rule, class_rule])
}

fn html_path_rule() -> R {
    let fresh_rule = lop::then(
        lop::optional(lop::sequence(vec![tok("colon"), lop::token("identifier", Some("fresh"))])),
        |option| Value::Dom(Dom::Bool(matches!(option, Value::Opt(Some(_))))),
    );
    let separator_rule = lop::then(
        lop::optional(lop::head(vec![
            tok("colon"),
            lop::token("identifier", Some("separator")),
            tok("open-paren"),
            capture(string_rule()),
            tok("close-paren"),
        ])),
        |option| match option {
            Value::Opt(Some(v)) => *v,
            _ => Value::Str(String::new()),
        },
    );
    let tag_names_rule = lop::one_or_more_with_separator(identifier_rule(), tok("choice"));
    let style_element_rule = lop::map(
        vec![
            capture(tag_names_rule),
            capture(lop::zero_or_more(attribute_or_class_rule())),
            capture(fresh_rule),
            capture(separator_rule),
        ],
        |v| {
            let names: Vec<String> = match &v[0] {
                Value::Array(list) => list.iter().map(str_of).collect(),
                other => vec![str_of(other)],
            };
            let mut attributes: Vec<(String, AttrValue)> = Vec::new();
            if let Value::Array(list) = &v[1] {
                for attribute in list {
                    let Value::Dom(Dom::Attribute { name, value, append }) = attribute else { continue };
                    if name == "__proto__" {
                        continue;
                    }
                    let existing = attributes.iter_mut().find(|(n, _)| n == name);
                    match existing {
                        Some((_, AttrValue::Str(current))) if *append && !current.is_empty() => {
                            current.push(' ');
                            current.push_str(value);
                        }
                        Some((_, slot)) => *slot = AttrValue::Str(value.clone()),
                        None => attributes.push((name.clone(), AttrValue::Str(value.clone()))),
                    }
                }
            }
            let fresh = matches!(v[2], Value::Dom(Dom::Bool(true)));
            let separator = match &v[3] {
                Value::Str(s) if !s.is_empty() => Some(s.clone()),
                _ => None,
            };
            let tag_name = names.first().cloned().unwrap_or_default();
            Value::Dom(Dom::PathElement(Rc::new(HtmlTag { tag_name, tag_names: names, attributes, fresh, separator })))
        },
    );

    lop::first_of(vec![
        lop::then(tok("bang"), |_| Value::Dom(Dom::Path(ParsedPath::Ignore))),
        lop::then(
            lop::zero_or_more_with_separator(
                style_element_rule,
                lop::sequence(vec![tok("whitespace"), tok("gt"), tok("whitespace")]),
            ),
            |elements| {
                let list = match elements {
                    Value::Array(list) => list,
                    other => vec![other],
                };
                Value::Dom(Dom::Path(ParsedPath::Elements(
                    list.into_iter()
                        .map(|e| match e {
                            Value::Dom(Dom::PathElement(el)) => Some(el),
                            _ => None,
                        })
                        .collect(),
                )))
            },
        ),
    ])
}

fn style_rule() -> R {
    lop::map(
        vec![
            capture(document_matcher_rule()),
            tok("whitespace"),
            tok("arrow"),
            capture(lop::optional(lop::head(vec![tok("whitespace"), capture(html_path_rule())]))),
            tok("end"),
        ],
        |v| {
            let from = match &v[0] {
                Value::Dom(Dom::Matcher(m)) => Some(m.clone()),
                _ => None,
            };
            // htmlPath.valueOrElse(htmlPaths.empty)
            let to = match &v[1] {
                Value::Opt(Some(p)) => match p.as_ref() {
                    Value::Dom(Dom::Path(path)) => path.clone(),
                    _ => ParsedPath::Elements(Vec::new()),
                },
                _ => ParsedPath::Elements(Vec::new()),
            };
            Value::Dom(Dom::Mapping(StyleMapping { from, to }))
        },
    )
}

/// `readStyle(string)`: the mapping, or `None` when lop rejects the line
/// (upstream records a warning and skips it).
pub fn read_style(line: &str) -> Option<StyleMapping> {
    let tokens = tokenise(line);
    let result = style_rule().parse(&TokenIterator::new(tokens));
    if !result.is_success() {
        return None;
    }
    match result.value {
        Value::Dom(Dom::Mapping(m)) => Some(m),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path_tags(p: &ParsedPath) -> Vec<(String, bool)> {
        match p {
            ParsedPath::Ignore => vec![("!".into(), false)],
            ParsedPath::Elements(e) => e
                .iter()
                .map(|t| t.as_ref().map_or(("undefined".into(), false), |t| (t.tag_names.join("|"), t.fresh)))
                .collect(),
        }
    }

    #[test]
    fn reads_the_default_map() {
        let m = read_style("p[style-name='Heading 1'] => h1:fresh").unwrap();
        assert!(matches!(
            m.from,
            Some(DocumentMatcher::Element { element_type: "paragraph", style_name: Some(StyleName::EqualTo(ref n)), .. }) if n == "Heading 1"
        ));
        assert_eq!(path_tags(&m.to), vec![("h1".into(), true)]);
        let m = read_style("p:ordered-list(2) => ul|ol > li > ol > li:fresh").unwrap();
        assert!(matches!(m.from, Some(DocumentMatcher::Element { list: Some((l, true)), .. }) if l == 1.0));
        assert_eq!(
            path_tags(&m.to),
            vec![("ul|ol".into(), false), ("li".into(), false), ("ol".into(), false), ("li".into(), true)]
        );
        let m = read_style("r[style-name='Hyperlink'] =>").unwrap();
        assert_eq!(path_tags(&m.to), vec![]);
        assert!(read_style("p.Heading1 => h1:fresh").is_some());
    }

    #[test]
    fn keeps_lop_quirks() {
        assert!(read_style("p =>").is_some());
        assert!(read_style("p => !").is_some());
        assert!(read_style("x => p").is_none());
        assert!(read_style("p[style-name='a' => p").is_none());
        // br with an unknown type parses into an undefined matcher.
        let m = read_style("br[type='other'] => hr").unwrap();
        assert!(m.from.is_none());
        // An error in the repeated part of a path becomes an undefined element.
        let m = read_style("p => p > li[foo").unwrap();
        assert_eq!(path_tags(&m.to), vec![("p".into(), false), ("undefined".into(), false)]);
        let m = read_style("p => span.a.b[x='1']:separator(', ')").unwrap();
        let ParsedPath::Elements(e) = &m.to else { panic!() };
        let tag = e[0].as_ref().unwrap();
        assert_eq!(
            tag.attributes,
            vec![("class".into(), AttrValue::Str("a b".into())), ("x".into(), AttrValue::Str("1".into()))]
        );
        assert_eq!(tag.separator.as_deref(), Some(", "));
        assert_eq!(decode_escape_sequences(r"a\nb\'c\"), "a\nb'c\\");
    }
}
