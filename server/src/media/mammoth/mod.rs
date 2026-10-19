//! mammoth 1.12.1 (`convertToHtml` with default options plus a
//! `convertImage`), with the parts of JSZip 3.10.1, @xmldom/xmldom 0.8.13,
//! lop 0.4.2 and dingbat-to-unicode 1.0.1 it runs, ported so the same .docx
//! gives the same HTML. One Rust file per upstream file, same names.
//!
//! Fidelity notes, all decided by what the upstream code does rather than by
//! what would be nicer:
//!
//! - **Every exception is the same answer.** docx.ts turns any rejection of
//!   `convertToHtml` into `not-a-document`, so the port only has to fail
//!   where JavaScript would throw ([`Failure`]; its message is for
//!   debugging). That includes the `TypeError`s of mammoth itself: a footnote
//!   reference to a missing footnote, a hyperlink whose relationship is
//!   missing when its text is written, a `w:num` without `w:abstractNumId`,
//!   a table style looked up in a document without a styles part, a complex
//!   field ended or separated before it began, a `w:sym` without `w:font`.
//! - **Plain objects are not maps.** Style ids, numbering ids, relationship
//!   ids, content types and element names are looked up in `{}`: a name that
//!   is an `Object.prototype` member (`constructor`, `toString`,
//!   `__proto__`, ...) finds an inherited function. [`js::JsObject`] models
//!   that; the effects are reproduced where they change the output (see the
//!   readers), except an object assigned to `__proto__` becoming a
//!   prototype for OTHER keys, and the global prototype pollution of a style
//!   whose `w:type` is `__proto__` (an upstream vulnerability: it adds
//!   properties to every object in the Node process).
//! - **Warnings are counted, not kept** ([`Ctx::warn`]). They never reach the
//!   HTML, with one exception: an element named `constructor` or `toString`
//!   (no namespace) leaves an `undefined` in mammoth's message list, and
//!   combining it with any other message is a `TypeError`.
//! - **Deep nesting.** Upstream recurses on the V8 stack, which runs out
//!   somewhere between 400 nested tables and 2343 nested unknown elements
//!   (measured on the reference, node 24.22; the exact point depends on what
//!   nests and on V8's tiering). Here XML nesting beyond
//!   [`MAX_XML_DEPTH`] elements fails, matching the most common case (1952
//!   nested unknown elements inside a paragraph).
//! - **Images run one at a time, in document order** (`promises.mapSeries`
//!   over the deferred nodes in tree order: body, then notes in reference
//!   order, then comments), which makes docx.ts's shared budget
//!   deterministic.

pub mod dingbat_to_unicode;
pub mod document_to_html;
pub mod documents;
pub mod docx;
pub mod html;
pub mod images;
pub mod js;
pub mod jszip;
pub mod lop;
pub mod options_reader;
pub mod style_reader;
pub mod styles;
pub mod writers;
pub mod xml;
pub mod zipfile;

use std::cell::Cell;
use std::collections::HashMap;
use std::sync::Arc;

use document_to_html::{DocumentConversion, deferred_in_order, replace_deferred};
use documents::Image;
use html::ast::HtmlNode;
use writers::html_writer::HtmlWriter;

/// Element nesting a part may have (the root element is depth 1).
pub const MAX_XML_DEPTH: usize = 1955;

/// A JavaScript exception somewhere in the conversion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure(pub String);

impl Failure {
    pub fn new(message: impl Into<String>) -> Failure {
        Failure(message.into())
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Per-conversion state the readers share.
pub struct Ctx {
    messages: Cell<usize>,
    poisons: Cell<usize>,
    pub max_depth: usize,
}

impl Ctx {
    pub fn new(max_depth: usize) -> Ctx {
        Ctx { messages: Cell::new(0), poisons: Cell::new(0), max_depth }
    }

    /// A warning or error message mammoth records.
    pub fn warn(&self) {
        self.messages.set(self.messages.get() + 1);
    }

    /// An `undefined` message (see the module docs).
    pub fn poison(&self) {
        self.poisons.set(self.poisons.get() + 1);
    }

    /// Whether combining the messages throws: an `undefined` entry meets any
    /// other entry in `combineMessages`.
    fn messages_throw(&self) -> bool {
        let poisons = self.poisons.get();
        poisons > 0 && poisons + self.messages.get() >= 2
    }
}

/// What `convertImage` resolved to: the `img` attributes, or why it
/// rejected (mammoth then records an error and drops the image).
pub type ConvertImageResult = Result<Vec<(String, String)>, String>;

/// `mammoth.convertToHtml({ buffer }, { convertImage: mammoth.images.imgElement(func) })`
/// where `func` is `convert_image`, called once per image, in order, each
/// call finished before the next starts. `max_entry` bounds what one archive
/// entry may expand to (see [`jszip`]).
pub fn convert_to_html(
    buffer: bytes::Bytes,
    max_entry: u64,
    convert_image: &mut dyn FnMut(&Image) -> ConvertImageResult,
) -> Result<String, Failure> {
    let ctx = Ctx::new(MAX_XML_DEPTH);
    let zip = Arc::new(jszip::ZipFile::open(buffer, max_entry).map_err(|e| Failure::new(e.0))?);
    let embedded_style_map = docx::style_map::read_style_map(&zip)?;
    let document = docx::docx_reader::read(&zip, &ctx)?;

    let mut style_map = Vec::new();
    for line in options_reader::style_map_lines(embedded_style_map.as_deref()) {
        match style_reader::read_style(&line) {
            Some(mapping) => style_map.push(mapping),
            None => ctx.warn(), // Did not understand this style mapping, so ignored it
        }
    }

    let html = {
        let mut conversion = DocumentConversion::new(&style_map, &ctx, &document);
        conversion.convert_document(&document)?
    };

    let mut deferred = Vec::new();
    deferred_in_order(&html, &mut deferred);
    let mut values: HashMap<usize, Vec<HtmlNode>> = HashMap::new();
    for (id, image) in deferred {
        let nodes = match convert_image(&image) {
            Ok(attributes) => vec![images::img_element(image.alt_text.as_deref(), attributes)],
            Err(_) => {
                ctx.warn(); // results.error(error)
                Vec::new()
            }
        };
        values.insert(id, nodes);
    }

    let mut writer = HtmlWriter::default();
    html::write(&mut writer, &html::simplify::simplify(replace_deferred(html, &values)))?;
    if ctx.messages_throw() {
        return Err(Failure::new("Cannot read properties of undefined (reading 'type')"));
    }
    Ok(writer.as_string())
}
