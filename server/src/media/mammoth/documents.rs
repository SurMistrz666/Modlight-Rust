//! mammoth `lib/documents.js`: the document model the body reader builds and
//! the HTML converter walks. Fields the converter never reads (paragraph
//! alignment and indent, run font and size, image content type) are not
//! carried.

use std::sync::Arc;

use super::jszip::ZipFile;

/// A value that may be a string, `undefined`, or (from a plain-object lookup)
/// an inherited `Object.prototype` member.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsVal {
    Str(String),
    Undefined,
    Builtin(&'static str),
}

/// What a paragraph's numbering resolved to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Numbering {
    Level {
        is_ordered: bool,
        level: String,
    },
    /// A truthy value that is not a level (an inherited member, a boolean
    /// from a level installed as a prototype): never a list.
    Opaque,
}

#[derive(Debug, Clone)]
pub struct Paragraph {
    pub children: Vec<DocElement>,
    pub style_id: Option<String>,
    pub style_name: Option<String>,
    pub numbering: Option<Numbering>,
}

#[derive(Debug, Clone)]
pub struct Run {
    pub children: Vec<DocElement>,
    pub style_id: Option<String>,
    pub style_name: Option<String>,
    pub is_bold: bool,
    pub is_underline: bool,
    pub is_italic: bool,
    pub is_strikethrough: bool,
    pub is_all_caps: bool,
    pub is_small_caps: bool,
    pub vertical_alignment: String,
    pub highlight: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Hyperlink {
    pub children: Vec<DocElement>,
    pub href: JsVal,
    pub anchor: Option<String>,
    pub target_frame: Option<String>,
}

/// Where an image's bytes come from.
#[derive(Debug, Clone)]
pub enum ImageSource {
    /// `docxFile.read(path)`.
    Embedded { zip: Arc<ZipFile>, path: String },
    /// A linked (`r:link`) picture: external file access is off, so reading
    /// always fails.
    External,
}

#[derive(Debug, Clone)]
pub struct Image {
    pub alt_text: Option<String>,
    pub source: ImageSource,
}

impl Image {
    /// `image.read()`: the bytes, or why the read rejected.
    pub fn read(&self) -> Result<Vec<u8>, String> {
        match &self.source {
            ImageSource::Embedded { zip, path } => zip.read(path).map_err(|e| e.0),
            ImageSource::External => Err("could not read external image, external file access is disabled".into()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Table {
    pub children: Vec<DocElement>,
    pub style_id: Option<String>,
    pub style_name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TableRow {
    pub children: Vec<DocElement>,
    pub is_header: bool,
}

#[derive(Debug, Clone)]
pub struct TableCell {
    pub children: Vec<DocElement>,
    pub col_span: f64,
    pub row_span: f64,
    /// `_vMerge`, only while the table is being read.
    pub v_merge: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakType {
    Line,
    Page,
    Column,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteType {
    Footnote,
    Endnote,
}

impl NoteType {
    pub fn as_str(self) -> &'static str {
        match self {
            NoteType::Footnote => "footnote",
            NoteType::Endnote => "endnote",
        }
    }
}

/// `String(id)` of an attribute that may be missing.
pub fn id_string(id: &Option<String>) -> String {
    id.clone().unwrap_or_else(|| "undefined".into())
}

#[derive(Debug, Clone)]
pub enum DocElement {
    Paragraph(Paragraph),
    Run(Run),
    Text(String),
    Tab,
    Checkbox {
        checked: bool,
    },
    Hyperlink(Hyperlink),
    NoteReference {
        note_type: NoteType,
        note_id: Option<String>,
    },
    Image(Image),
    Table(Table),
    TableRow(TableRow),
    TableCell(TableCell),
    Break(BreakType),
    BookmarkStart {
        name: Option<String>,
    },
    CommentReference {
        comment_id: Option<String>,
    },
    /// `undefined` in an element list: what an element named after an
    /// `Object.prototype` member (`<constructor/>`, `<toString/>`) leaves
    /// behind. Touching it is a `TypeError`.
    Poison,
}

impl DocElement {
    pub fn children(&self) -> Option<&Vec<DocElement>> {
        match self {
            DocElement::Paragraph(p) => Some(&p.children),
            DocElement::Run(r) => Some(&r.children),
            DocElement::Hyperlink(h) => Some(&h.children),
            DocElement::Table(t) => Some(&t.children),
            DocElement::TableRow(r) => Some(&r.children),
            DocElement::TableCell(c) => Some(&c.children),
            _ => None,
        }
    }

    pub fn children_mut(&mut self) -> Option<&mut Vec<DocElement>> {
        match self {
            DocElement::Paragraph(p) => Some(&mut p.children),
            DocElement::Run(r) => Some(&mut r.children),
            DocElement::Hyperlink(h) => Some(&mut h.children),
            DocElement::Table(t) => Some(&mut t.children),
            DocElement::TableRow(r) => Some(&mut r.children),
            DocElement::TableCell(c) => Some(&mut c.children),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Note {
    pub note_type: NoteType,
    pub note_id: Option<String>,
    pub body: Vec<DocElement>,
}

/// `noteKey(noteType, id)`.
pub fn note_key(note_type: NoteType, id: &Option<String>) -> String {
    format!("{}-{}", note_type.as_str(), id_string(id))
}

#[derive(Debug, Clone)]
pub struct Comment {
    pub comment_id: Option<String>,
    pub body: Vec<DocElement>,
    pub author_name: Option<String>,
    pub author_initials: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Document {
    pub children: Vec<DocElement>,
    pub notes: Vec<Note>,
    pub comments: Vec<Comment>,
}
