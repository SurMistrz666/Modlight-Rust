//! mammoth `lib/document-to-html.js`: the document model into an HTML tree,
//! with each image left as a deferred node for `convertImage`.
//!
//! The order things happen in matters and is kept: a run's wrapping paths are
//! all looked up before its content is converted, an `ignore` path (`!`)
//! never converts what it wraps (no note numbers, no images), notes are
//! converted after the body in the order they were referenced, and comments
//! after the notes.

use std::rc::Rc;

use super::documents::{
    BreakType, Comment, DocElement, Document, JsVal, Note, NoteType, Paragraph, Run, Table, TableCell, id_string,
    note_key,
};
use super::html::ast::{HtmlNode, element_with_tag, fresh_element, non_fresh_element, text};
use super::js::{self, JsObject, Lookup};
use super::style_reader::{ParsedPath, StyleMapping};
use super::styles::document_matchers::MatchTarget;
use super::styles::html_paths::{AttrValue, HtmlTag};
use super::{Ctx, Failure};

type Nodes = Result<Vec<HtmlNode>, Failure>;

/// One referenced comment: its label, and the comment it found.
enum FoundComment<'d> {
    Comment(&'d Comment),
    /// An inherited `Object.prototype` member: converting it throws.
    Builtin,
}

pub struct DocumentConversion<'s, 'd> {
    style_map: &'s [StyleMapping],
    ctx: &'s Ctx,
    note_number: usize,
    note_references: Vec<(NoteType, Option<String>)>,
    referenced_comments: Vec<(String, FoundComment<'d>)>,
    comments: JsObject<&'d Comment>,
    next_deferred: usize,
}

fn attr(key: &str, value: impl Into<String>) -> (String, AttrValue) {
    (key.to_owned(), AttrValue::Str(value.into()))
}

/// `htmlId(suffix)` with the default empty `idPrefix`.
fn html_id(suffix: &str) -> String {
    suffix.to_owned()
}

fn referent_html_id(reference_type: &str, reference_id: &str) -> String {
    html_id(&format!("{reference_type}-{reference_id}"))
}

fn reference_html_id(reference_type: &str, reference_id: &str) -> String {
    html_id(&format!("{reference_type}-ref-{reference_id}"))
}

impl<'s, 'd> DocumentConversion<'s, 'd> {
    pub fn new(style_map: &'s [StyleMapping], ctx: &'s Ctx, document: &'d Document) -> Self {
        // _.indexBy(document.comments, "commentId")
        let mut comments = JsObject::new();
        for comment in &document.comments {
            comments.set(&id_string(&comment.comment_id), comment);
        }
        DocumentConversion {
            style_map,
            ctx,
            note_number: 1,
            note_references: Vec::new(),
            referenced_comments: Vec::new(),
            comments,
            next_deferred: 1,
        }
    }

    /// `findStyle(element)`.
    fn find_style(&self, element: &MatchTarget<'_>) -> Result<Option<&'s StyleMapping>, Failure> {
        for mapping in self.style_map {
            let Some(from) = &mapping.from else {
                return Err(Failure::new("Cannot read properties of undefined (reading 'matches')"));
            };
            if from.matches(element) {
                return Ok(Some(mapping));
            }
        }
        Ok(None)
    }

    /// `findHtmlPath(element, defaultPath)`.
    fn find_html_path(&self, element: &MatchTarget<'_>, default: ParsedPath) -> Result<ParsedPath, Failure> {
        Ok(match self.find_style(element)? {
            Some(style) => style.to.clone(),
            None => default,
        })
    }

    /// `path.wrap(children)`.
    fn wrap(&mut self, path: &ParsedPath, children: impl FnOnce(&mut Self) -> Nodes) -> Nodes {
        let ParsedPath::Elements(elements) = path else {
            return Ok(Vec::new());
        };
        let mut result = children(self)?;
        for element in elements.iter().rev() {
            let Some(tag) = element else {
                return Err(Failure::new("Cannot read properties of undefined (reading 'wrapNodes')"));
            };
            result = vec![element_with_tag(Rc::clone(tag), result)];
        }
        Ok(result)
    }

    fn convert_elements(&mut self, elements: &'d [DocElement], is_table_header: bool) -> Nodes {
        let mut out = Vec::new();
        for element in elements {
            out.extend(self.element_to_html(element, is_table_header)?);
        }
        Ok(out)
    }

    /// The `document` converter.
    pub fn convert_document(&mut self, document: &'d Document) -> Nodes {
        let mut children = self.convert_elements(&document.children, false)?;
        // document.notes.resolve(reference) for each reference so far.
        let mut by_key: std::collections::HashMap<String, &'d Note> = std::collections::HashMap::new();
        for note in &document.notes {
            by_key.insert(note_key(note.note_type, &note.note_id), note);
        }
        let notes: Vec<Option<&'d Note>> =
            self.note_references.iter().map(|(note_type, id)| by_key.get(&note_key(*note_type, id)).copied()).collect();
        let mut notes_nodes = Vec::new();
        for note in notes {
            let Some(note) = note else {
                return Err(Failure::new("Cannot read properties of null (reading 'type')"));
            };
            notes_nodes.push(self.convert_note(note)?);
        }
        let mut comment_nodes = Vec::new();
        let count = self.referenced_comments.len();
        for index in 0..count {
            comment_nodes.extend(self.convert_comment(index)?);
        }
        children.push(fresh_element("ol", Vec::new(), notes_nodes));
        children.push(fresh_element("dl", Vec::new(), comment_nodes));
        Ok(children)
    }

    fn element_to_html(&mut self, element: &'d DocElement, is_table_header: bool) -> Nodes {
        match element {
            DocElement::Paragraph(p) => self.convert_paragraph(p, is_table_header),
            DocElement::Run(r) => self.convert_run(r, is_table_header),
            DocElement::Text(value) => Ok(vec![text(value.clone())]),
            DocElement::Tab => Ok(vec![text("\t")]),
            DocElement::Hyperlink(h) => {
                let href = match &h.anchor {
                    Some(anchor) if !anchor.is_empty() => AttrValue::Str(format!("#{}", html_id(anchor))),
                    _ => match &h.href {
                        JsVal::Str(s) => AttrValue::Str(s.clone()),
                        JsVal::Undefined => AttrValue::Undefined,
                        JsVal::Builtin(b) => AttrValue::Builtin(b),
                    },
                };
                let mut attributes = vec![("href".to_owned(), href)];
                if let Some(target) = &h.target_frame {
                    attributes.push(attr("target", target.clone()));
                }
                let children = self.convert_elements(&h.children, is_table_header)?;
                Ok(vec![non_fresh_element("a", attributes, children)])
            }
            DocElement::Checkbox { checked } => {
                let mut attributes = vec![attr("type", "checkbox")];
                if *checked {
                    attributes.push(attr("checked", "checked"));
                }
                Ok(vec![fresh_element("input", attributes, Vec::new())])
            }
            DocElement::BookmarkStart { name } => {
                let anchor =
                    fresh_element("a", vec![attr("id", html_id(&id_string(name)))], vec![HtmlNode::ForceWrite]);
                Ok(vec![anchor])
            }
            DocElement::NoteReference { note_type, note_id } => {
                self.note_references.push((*note_type, note_id.clone()));
                let id = id_string(note_id);
                let number = self.note_number;
                self.note_number += 1;
                let anchor = fresh_element(
                    "a",
                    vec![
                        attr("href", format!("#{}", referent_html_id(note_type.as_str(), &id))),
                        attr("id", reference_html_id(note_type.as_str(), &id)),
                    ],
                    vec![text(format!("[{number}]"))],
                );
                Ok(vec![fresh_element("sup", Vec::new(), vec![anchor])])
            }
            DocElement::CommentReference { comment_id } => self.convert_comment_reference(comment_id),
            DocElement::Image(image) => {
                let id = self.next_deferred;
                self.next_deferred += 1;
                Ok(vec![HtmlNode::Deferred { id, image: image.clone() }])
            }
            DocElement::Table(t) => self.convert_table(t, is_table_header),
            DocElement::TableRow(r) => {
                let mut children = vec![HtmlNode::ForceWrite];
                children.extend(self.convert_elements(&r.children, is_table_header)?);
                Ok(vec![fresh_element("tr", Vec::new(), children)])
            }
            DocElement::TableCell(c) => self.convert_table_cell(c, is_table_header),
            DocElement::Break(b) => {
                let path = match self.find_style(&MatchTarget::Break(*b))? {
                    Some(style) => style.to.clone(),
                    None if *b == BreakType::Line => {
                        ParsedPath::Elements(vec![Some(Rc::new(HtmlTag::new("br", Vec::new(), true)))])
                    }
                    None => ParsedPath::Elements(Vec::new()),
                };
                self.wrap(&path, |_| Ok(Vec::new()))
            }
            DocElement::Poison => Err(Failure::new("Cannot read properties of undefined (reading 'type')")),
        }
    }

    fn convert_paragraph(&mut self, element: &'d Paragraph, is_table_header: bool) -> Nodes {
        let path = match self.find_style(&MatchTarget::Paragraph(element))? {
            Some(style) => style.to.clone(),
            None => {
                if element.style_id.is_some() {
                    self.ctx.warn(); // Unrecognised paragraph style
                }
                ParsedPath::Elements(vec![Some(Rc::new(HtmlTag::new("p", Vec::new(), true)))])
            }
        };
        self.wrap(&path, |s| s.convert_elements(&element.children, is_table_header))
    }

    /// `findHtmlPathForRunProperty(elementType, defaultTagName)`.
    fn run_property_path(
        &self,
        element_type: &'static str,
        default_tag_name: Option<&str>,
    ) -> Result<ParsedPath, Failure> {
        Ok(match self.find_style(&MatchTarget::Property(element_type))? {
            Some(style) => style.to.clone(),
            None => match default_tag_name {
                Some(tag) => ParsedPath::Elements(vec![Some(Rc::new(HtmlTag::new(tag, Vec::new(), false)))]),
                None => ParsedPath::Elements(Vec::new()),
            },
        })
    }

    fn convert_run(&mut self, run: &'d Run, is_table_header: bool) -> Nodes {
        let mut paths: Vec<ParsedPath> = Vec::new();
        if let Some(color) = &run.highlight
            && let Some(style) = self.find_style(&MatchTarget::Highlight(color))?
        {
            paths.push(style.to.clone());
        }
        if run.is_small_caps {
            paths.push(self.run_property_path("smallCaps", None)?);
        }
        if run.is_all_caps {
            paths.push(self.run_property_path("allCaps", None)?);
        }
        if run.is_strikethrough {
            paths.push(self.run_property_path("strikethrough", Some("s"))?);
        }
        if run.is_underline {
            paths.push(self.run_property_path("underline", None)?);
        }
        let plain = |tag: &str| ParsedPath::Elements(vec![Some(Rc::new(HtmlTag::new(tag, Vec::new(), false)))]);
        if run.vertical_alignment == "subscript" {
            paths.push(plain("sub"));
        }
        if run.vertical_alignment == "superscript" {
            paths.push(plain("sup"));
        }
        if run.is_italic {
            paths.push(self.run_property_path("italic", Some("em"))?);
        }
        if run.is_bold {
            paths.push(self.run_property_path("bold", Some("strong"))?);
        }
        let style_path = match self.find_style(&MatchTarget::Run(run))? {
            Some(style) => style.to.clone(),
            None => {
                if run.style_id.is_some() {
                    self.ctx.warn(); // Unrecognised run style
                }
                ParsedPath::Elements(Vec::new())
            }
        };
        paths.push(style_path);
        self.apply_paths(&paths, run, is_table_header)
    }

    /// The `nodes = path.wrap.bind(path, nodes)` chain: the last path is the
    /// outermost and is called first.
    fn apply_paths(&mut self, paths: &[ParsedPath], run: &'d Run, is_table_header: bool) -> Nodes {
        match paths.split_last() {
            None => self.convert_elements(&run.children, is_table_header),
            Some((outer, rest)) => self.wrap(outer, |s| s.apply_paths(rest, run, is_table_header)),
        }
    }

    fn convert_table(&mut self, element: &'d Table, _is_table_header: bool) -> Nodes {
        let default = ParsedPath::Elements(vec![Some(Rc::new(HtmlTag::new("table", Vec::new(), true)))]);
        let path = self.find_html_path(&MatchTarget::Table(element), default)?;
        self.wrap(&path, |s| s.convert_table_children(element))
    }

    fn convert_table_children(&mut self, element: &'d Table) -> Nodes {
        // `!child.type === tableRow || !child.isHeader`: the first half is
        // always false, so the body starts at the first non-header child.
        let body_index = element
            .children
            .iter()
            .position(|child| !matches!(child, DocElement::TableRow(r) if r.is_header))
            .unwrap_or(element.children.len());
        let children = if body_index == 0 {
            self.convert_elements(&element.children, false)?
        } else {
            let head_rows = self.convert_elements(&element.children[..body_index], true)?;
            let body_rows = self.convert_elements(&element.children[body_index..], false)?;
            vec![fresh_element("thead", Vec::new(), head_rows), fresh_element("tbody", Vec::new(), body_rows)]
        };
        let mut out = vec![HtmlNode::ForceWrite];
        out.extend(children);
        Ok(out)
    }

    fn convert_table_cell(&mut self, element: &'d TableCell, is_table_header: bool) -> Nodes {
        let tag_name = if is_table_header { "th" } else { "td" };
        let children = self.convert_elements(&element.children, is_table_header)?;
        let mut attributes = Vec::new();
        if element.col_span != 1.0 {
            attributes.push(attr("colspan", js::number_to_string(element.col_span)));
        }
        if element.row_span != 1.0 {
            attributes.push(attr("rowspan", js::number_to_string(element.row_span)));
        }
        let mut body = vec![HtmlNode::ForceWrite];
        body.extend(children);
        Ok(vec![fresh_element(tag_name, attributes, body)])
    }

    fn convert_note(&mut self, note: &'d Note) -> Result<HtmlNode, Failure> {
        let mut children = self.convert_elements(&note.body, false)?;
        let id = id_string(&note.note_id);
        let back_link = element_with_tag(
            Rc::new(HtmlTag::new("p", Vec::new(), false)),
            vec![
                text(" "),
                fresh_element(
                    "a",
                    vec![attr("href", format!("#{}", reference_html_id(note.note_type.as_str(), &id)))],
                    vec![text("↑")],
                ),
            ],
        );
        children.push(back_link);
        Ok(fresh_element("li", vec![attr("id", referent_html_id(note.note_type.as_str(), &id))], children))
    }

    fn convert_comment_reference(&mut self, comment_id: &Option<String>) -> Nodes {
        let path = self.find_html_path(&MatchTarget::CommentReference, ParsedPath::Ignore)?;
        let id = id_string(comment_id);
        self.wrap(&path, |s| {
            let found = match s.comments.get(&id) {
                Lookup::Own(c) => FoundComment::Comment(c),
                Lookup::Builtin(_) => FoundComment::Builtin,
                // commentAuthorLabel(undefined)
                Lookup::Missing => {
                    return Err(Failure::new("Cannot read properties of undefined (reading 'authorInitials')"));
                }
            };
            let count = s.referenced_comments.len() + 1;
            let initials = match &found {
                FoundComment::Comment(c) => c.author_initials.clone().unwrap_or_default(),
                FoundComment::Builtin => String::new(),
            };
            let label = format!("[{initials}{count}]");
            s.referenced_comments.push((label.clone(), found));
            // TODO: remove duplication with note references
            Ok(vec![fresh_element(
                "a",
                vec![
                    attr("href", format!("#{}", referent_html_id("comment", &id))),
                    attr("id", reference_html_id("comment", &id)),
                ],
                vec![text(label)],
            )])
        })
    }

    fn convert_comment(&mut self, index: usize) -> Nodes {
        // TODO: remove duplication with note references
        let (label, comment) = match &self.referenced_comments[index] {
            (label, FoundComment::Comment(c)) => (label.clone(), *c),
            // `comment.body` of a function: convertElements(undefined) throws.
            (_, FoundComment::Builtin) => {
                return Err(Failure::new("Cannot read properties of undefined (reading 'map')"));
            }
        };
        let id = id_string(&comment.comment_id);
        let mut body = self.convert_elements(&comment.body, false)?;
        body.push(non_fresh_element(
            "p",
            Vec::new(),
            vec![
                text(" "),
                fresh_element(
                    "a",
                    vec![attr("href", format!("#{}", reference_html_id("comment", &id)))],
                    vec![text("↑")],
                ),
            ],
        ));
        Ok(vec![
            fresh_element(
                "dt",
                vec![attr("id", referent_html_id("comment", &id))],
                vec![text(format!("Comment {label}"))],
            ),
            fresh_element("dd", Vec::new(), body),
        ])
    }
}

/// `walkHtml`: the deferred nodes in tree order (the order `mapSeries` runs
/// them in).
pub fn deferred_in_order(nodes: &[HtmlNode], out: &mut Vec<(usize, super::documents::Image)>) {
    for node in nodes {
        match node {
            HtmlNode::Deferred { id, image } => out.push((*id, image.clone())),
            HtmlNode::Element { children, .. } => deferred_in_order(children, out),
            _ => {}
        }
    }
}

/// `replaceDeferred(nodes)`.
pub fn replace_deferred(
    nodes: Vec<HtmlNode>,
    values: &std::collections::HashMap<usize, Vec<HtmlNode>>,
) -> Vec<HtmlNode> {
    let mut out = Vec::with_capacity(nodes.len());
    for node in nodes {
        match node {
            HtmlNode::Deferred { id, .. } => out.extend(values.get(&id).cloned().unwrap_or_default()),
            HtmlNode::Element { tag, children } => {
                out.push(HtmlNode::Element { tag, children: replace_deferred(children, values) })
            }
            other => out.push(other),
        }
    }
    out
}
