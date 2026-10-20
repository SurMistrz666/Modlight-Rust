//! mammoth `lib/docx/body-reader.js`: OOXML body elements into the document
//! model.
//!
//! `ReadResult` keeps two lists: the elements, and the "extra" elements a
//! `w:pict` hoists out (they surface after the enclosing paragraph, and are
//! lost when there is none). Warnings are counted, not kept (see
//! `Ctx::warn`): they never reach the HTML, but an element named after an
//! `Object.prototype` member turns every warning into a `TypeError`.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use regex::Regex;

use super::super::documents::{
    BreakType, DocElement, Hyperlink, Image, ImageSource, JsVal, NoteType, Paragraph, Run, Table, TableCell, TableRow,
};
use super::super::js::{self, JS_DOT_CLASS, JS_WHITESPACE_CLASS};
use super::super::jszip::ZipFile;
use super::super::xml::{EMPTY_ELEMENT, XmlElement, XmlNode};
use super::super::{Ctx, Failure, dingbat_to_unicode};
use super::content_types_reader::ContentTypes;
use super::numbering_xml::{LevelResult, Numbering};
use super::relationships_reader::Relationships;
use super::styles_reader::{StyleLookup, Styles};
use super::uris;

/// What every reader gets (`createBodyReader(options)`).
pub struct BodyReaderOptions<'a> {
    pub relationships: &'a Relationships,
    pub content_types: &'a ContentTypes,
    pub docx_file: &'a Arc<ZipFile>,
    pub numbering: &'a Numbering,
    pub styles: &'a Styles,
    pub ctx: &'a Ctx,
}

/// `ReadResult`.
#[derive(Debug, Default)]
pub struct ReadResult {
    pub value: Vec<DocElement>,
    pub extra: Vec<DocElement>,
}

impl ReadResult {
    fn empty() -> Self {
        ReadResult::default()
    }

    fn element(e: DocElement) -> Self {
        ReadResult { value: vec![e], extra: Vec::new() }
    }

    /// `toExtra()`. An `undefined` among them is dropped by the next
    /// `combineResults` (`_.filter(..., identity)`), so it is dropped here.
    fn into_extra(self) -> Self {
        let mut extra = self.extra;
        extra.extend(self.value.into_iter().filter(|e| !matches!(e, DocElement::Poison)));
        ReadResult { value: Vec::new(), extra }
    }

    /// `insertExtra()`.
    fn insert_extra(self) -> Self {
        if self.extra.is_empty() {
            return self;
        }
        let mut value = self.value;
        value.extend(self.extra);
        ReadResult { value, extra: Vec::new() }
    }
}

/// `combineResults(results)`.
fn combine(results: Vec<ReadResult>) -> ReadResult {
    let mut out = ReadResult::empty();
    for r in results {
        out.value.extend(r.value);
        out.extra.extend(r.extra.into_iter().filter(|e| !matches!(e, DocElement::Poison)));
    }
    out
}

/// `{styleId, name}` of `readStyle`.
struct StyleRef {
    style_id: Option<String>,
    name: Option<String>,
}

/// One entry of `complexFieldStack`.
#[derive(Clone)]
enum ComplexField<'x> {
    Begin(&'x XmlElement),
    Hyperlink { href: Option<String>, anchor: Option<String> },
    Checkbox { checked: bool },
    Unknown,
}

static HYPERLINK_INSTR: LazyLock<Regex> = LazyLock::new(|| {
    let s = format!("[{JS_WHITESPACE_CLASS}]");
    let not_s = format!("[^{JS_WHITESPACE_CLASS}]");
    Regex::new(&format!(r#"^{s}*HYPERLINK{s}+(\\l{s}+)?(?:"({JS_DOT_CLASS}*)"|([^\\]{not_s}*))"#))
        .expect("HYPERLINK regex")
});

/// `new BodyReader(options)`: one per `readXmlElements` call from outside,
/// so complex fields and deleted paragraphs never leak between parts.
pub struct BodyReader<'a, 'x> {
    options: &'a BodyReaderOptions<'a>,
    complex_field_stack: Vec<ComplexField<'x>>,
    current_instr_text: Vec<String>,
    // When a paragraph is marked as deleted, its contents should be combined
    // with the following paragraph. See 17.13.5.15 del (Deleted Paragraph) of
    // ECMA-376 4th edition Part 1.
    deleted_paragraph_contents: Vec<&'x XmlNode>,
}

/// `createBodyReader(options).readXmlElements(elements)`.
pub fn read_xml_elements<'a>(options: &'a BodyReaderOptions<'a>, elements: &[XmlNode]) -> Result<ReadResult, Failure> {
    let mut reader = BodyReader {
        options,
        complex_field_stack: Vec::new(),
        current_instr_text: Vec::new(),
        deleted_paragraph_contents: Vec::new(),
    };
    reader.read_xml_elements(elements.iter())
}

fn read_boolean_attribute_value(value: Option<&str>) -> bool {
    value != Some("false") && value != Some("0")
}

/// `readBooleanElement(element)`.
fn read_boolean_element(element: Option<&XmlElement>) -> bool {
    match element {
        Some(e) => read_boolean_attribute_value(e.attr("w:val")),
        None => false,
    }
}

/// `readUnderline(element)`.
fn read_underline(element: Option<&XmlElement>) -> bool {
    match element.and_then(|e| e.attr("w:val")) {
        Some(v) => v != "false" && v != "0" && v != "none",
        None => false,
    }
}

const IGNORE_ELEMENTS: [&str; 19] = [
    "office-word:wrap",
    "v:shadow",
    "v:shapetype",
    "w:annotationRef",
    "w:bookmarkEnd",
    "w:sectPr",
    "w:proofErr",
    "w:lastRenderedPageBreak",
    "w:commentRangeStart",
    "w:commentRangeEnd",
    "w:del",
    "w:footnoteRef",
    "w:endnoteRef",
    "w:pPr",
    "w:rPr",
    "w:tblPr",
    "w:tblGrid",
    "w:trPr",
    "w:tcPr",
];

impl<'a, 'x> BodyReader<'a, 'x> {
    fn ctx(&self) -> &'a Ctx {
        self.options.ctx
    }

    fn read_xml_elements(&mut self, elements: impl Iterator<Item = &'x XmlNode>) -> Result<ReadResult, Failure> {
        let mut results = Vec::new();
        for element in elements {
            results.push(self.read_xml_element(element)?);
        }
        Ok(combine(results))
    }

    fn read_child_elements(&mut self, element: &'x XmlElement) -> Result<ReadResult, Failure> {
        self.read_xml_elements(element.children.iter())
    }

    fn read_xml_element(&mut self, node: &'x XmlNode) -> Result<ReadResult, Failure> {
        let XmlNode::Element(element) = node else {
            return Ok(ReadResult::empty());
        };
        match element.name.as_str() {
            "w:p" => self.read_paragraph(element),
            "w:r" => self.read_run(element),
            "w:fldChar" => self.read_fld_char(element),
            "w:instrText" => {
                self.current_instr_text.push(element.text()?);
                Ok(ReadResult::empty())
            }
            "w:t" => Ok(ReadResult::element(DocElement::Text(element.text()?))),
            "w:tab" => Ok(ReadResult::element(DocElement::Tab)),
            "w:noBreakHyphen" => Ok(ReadResult::element(DocElement::Text("\u{2011}".into()))),
            "w:softHyphen" => Ok(ReadResult::element(DocElement::Text("\u{AD}".into()))),
            "w:sym" => self.read_symbol(element),
            "w:hyperlink" => self.read_hyperlink(element),
            "w:tbl" => self.read_table(element),
            "w:tr" => self.read_table_row(element),
            "w:tc" => self.read_table_cell(element),
            "w:footnoteReference" => Ok(ReadResult::element(DocElement::NoteReference {
                note_type: NoteType::Footnote,
                note_id: element.attr("w:id").map(str::to_owned),
            })),
            "w:endnoteReference" => Ok(ReadResult::element(DocElement::NoteReference {
                note_type: NoteType::Endnote,
                note_id: element.attr("w:id").map(str::to_owned),
            })),
            "w:commentReference" => Ok(ReadResult::element(DocElement::CommentReference {
                comment_id: element.attr("w:id").map(str::to_owned),
            })),
            "w:br" => {
                let break_type = element.attr("w:type");
                Ok(match break_type {
                    None | Some("textWrapping") => ReadResult::element(DocElement::Break(BreakType::Line)),
                    Some("page") => ReadResult::element(DocElement::Break(BreakType::Page)),
                    Some("column") => ReadResult::element(DocElement::Break(BreakType::Column)),
                    Some(_) => {
                        self.ctx().warn(); // Unsupported break type
                        ReadResult::empty()
                    }
                })
            }
            "w:bookmarkStart" => {
                let name = element.attr("w:name");
                if name == Some("_GoBack") {
                    Ok(ReadResult::empty())
                } else {
                    Ok(ReadResult::element(DocElement::BookmarkStart { name: name.map(str::to_owned) }))
                }
            }
            "mc:AlternateContent" => self.read_child_elements(element.first_or_empty("mc:Fallback")),
            "w:sdt" => self.read_sdt(element),
            "w:ins" | "w:object" | "w:smartTag" | "w:drawing" | "v:roundrect" | "v:shape" | "v:textbox"
            | "w:txbxContent" | "v:group" | "v:rect" => self.read_child_elements(element),
            "w:pict" => Ok(self.read_child_elements(element)?.into_extra()),
            "wp:inline" | "wp:anchor" => self.read_drawing_element(element),
            "v:imagedata" => self.read_image_data(element),
            name => {
                // `xmlElementReaders[name]` is a plain object: an inherited
                // member is "found" and called.
                match js::builtin(name) {
                    // Object(element) / toString() return no ReadResult: an
                    // `undefined` element and an `undefined` message.
                    Some("constructor") | Some("toString") => {
                        self.ctx().poison();
                        Ok(ReadResult::element(DocElement::Poison))
                    }
                    // Called with `this` undefined, or not a function at all.
                    Some(_) => Err(Failure::new(format!("handler for <{name}> is not callable"))),
                    None => {
                        if !IGNORE_ELEMENTS.contains(&name) {
                            self.ctx().warn(); // An unrecognised element was ignored
                        }
                        Ok(ReadResult::empty())
                    }
                }
            }
        }
    }

    // --- styles and properties --------------------------------------------------------------

    fn read_style(
        &self,
        element: &XmlElement,
        style_tag_name: &str,
        find: impl Fn(&Styles, &str) -> Result<StyleLookup, Failure>,
    ) -> Result<StyleRef, Failure> {
        let mut style_id = None;
        let mut name = None;
        if let Some(style_element) = element.first(style_tag_name) {
            let id = style_element.attr("w:val");
            style_id = id.map(str::to_owned);
            if let Some(id) = id.filter(|s| !s.is_empty()) {
                match find(self.options.styles, id)? {
                    StyleLookup::Found(n) => name = n,
                    StyleLookup::Missing => self.ctx().warn(), // style referenced but not defined
                }
            }
        }
        Ok(StyleRef { style_id, name })
    }

    fn read_paragraph(&mut self, element: &'x XmlElement) -> Result<ReadResult, Failure> {
        let properties_element = element.first_or_empty("w:pPr");

        let is_deleted = properties_element.first_or_empty("w:rPr").first("w:del").is_some();

        if is_deleted {
            self.deleted_paragraph_contents.extend(element.children.iter());
            return Ok(ReadResult::empty());
        }

        let mut children_xml: Vec<&'x XmlNode> = Vec::new();
        if !self.deleted_paragraph_contents.is_empty() {
            children_xml.append(&mut self.deleted_paragraph_contents);
        }
        children_xml.extend(element.children.iter());

        // readParagraphProperties(paragraphPropertiesElement)
        let style = self.read_style(properties_element, "w:pStyle", Styles::find_paragraph_style_by_id)?;
        let numbering = read_numbering_properties(
            style.style_id.as_deref(),
            properties_element.first_or_empty("w:numPr"),
            self.options.numbering,
        )?;

        let children = self.read_xml_elements(children_xml.into_iter())?;
        let paragraph = DocElement::Paragraph(Paragraph {
            children: children.value,
            style_id: style.style_id.filter(|s| !s.is_empty()),
            style_name: style.name.filter(|s| !s.is_empty()),
            numbering: numbering.into_option(),
        });
        Ok(ReadResult { value: vec![paragraph], extra: children.extra }.insert_extra())
    }

    fn read_run(&mut self, element: &'x XmlElement) -> Result<ReadResult, Failure> {
        let properties = element.first_or_empty("w:rPr");
        let style = self.read_style(properties, "w:rStyle", Styles::find_character_style_by_id)?;
        let highlight = properties.first_or_empty("w:highlight").attr("w:val");
        let run_properties = (
            style,
            properties.first_or_empty("w:vertAlign").attr("w:val").filter(|s| !s.is_empty()).map(str::to_owned),
            read_boolean_element(properties.first("w:b")),
            read_underline(properties.first("w:u")),
            read_boolean_element(properties.first("w:i")),
            read_boolean_element(properties.first("w:strike")),
            read_boolean_element(properties.first("w:caps")),
            read_boolean_element(properties.first("w:smallCaps")),
            highlight.filter(|v| !v.is_empty() && *v != "none").map(str::to_owned),
        );
        let children = self.read_child_elements(element)?;
        let (
            style,
            vertical_alignment,
            is_bold,
            is_underline,
            is_italic,
            is_strikethrough,
            is_all_caps,
            is_small_caps,
            highlight,
        ) = run_properties;

        let mut run_children = children.value;
        if let Some((href, anchor)) = self.current_hyperlink_options() {
            run_children = vec![DocElement::Hyperlink(Hyperlink {
                children: run_children,
                href: href.map_or(JsVal::Undefined, JsVal::Str),
                anchor,
                target_frame: None,
            })];
        }
        let run = DocElement::Run(Run {
            children: run_children,
            style_id: style.style_id.filter(|s| !s.is_empty()),
            style_name: style.name.filter(|s| !s.is_empty()),
            is_bold,
            is_underline,
            is_italic,
            is_strikethrough,
            is_all_caps,
            is_small_caps,
            vertical_alignment: vertical_alignment.unwrap_or_else(|| "baseline".into()),
            highlight,
        });
        Ok(ReadResult { value: vec![run], extra: children.extra })
    }

    // --- fields ----------------------------------------------------------------------------------

    fn read_fld_char(&mut self, element: &'x XmlElement) -> Result<ReadResult, Failure> {
        match element.attr("w:fldCharType") {
            Some("begin") => {
                self.complex_field_stack.push(ComplexField::Begin(element));
                self.current_instr_text.clear();
            }
            Some("end") => {
                let Some(mut end) = self.complex_field_stack.pop() else {
                    return Err(Failure::new("Cannot read properties of undefined (reading 'type')"));
                };
                if let ComplexField::Begin(fld_char) = end {
                    end = self.parse_instr_text(fld_char);
                }
                if let ComplexField::Checkbox { checked } = end {
                    return Ok(ReadResult::element(DocElement::Checkbox { checked }));
                }
            }
            Some("separate") => {
                let Some(separate) = self.complex_field_stack.pop() else {
                    return Err(Failure::new("Cannot read properties of undefined (reading 'type')"));
                };
                let fld_char = match separate {
                    ComplexField::Begin(f) => f,
                    _ => &EMPTY_ELEMENT,
                };
                let field = self.parse_instr_text(fld_char);
                self.complex_field_stack.push(field);
            }
            _ => {}
        }
        Ok(ReadResult::empty())
    }

    fn current_hyperlink_options(&self) -> Option<(Option<String>, Option<String>)> {
        self.complex_field_stack.iter().rev().find_map(|f| match f {
            ComplexField::Hyperlink { href, anchor } => Some((href.clone(), anchor.clone())),
            _ => None,
        })
    }

    fn parse_instr_text(&self, fld_char: &'x XmlElement) -> ComplexField<'x> {
        let instr_text = self.current_instr_text.concat();
        if let Some(caps) = HYPERLINK_INSTR.captures(&instr_text) {
            let location = caps.get(2).or_else(|| caps.get(3)).map(|m| m.as_str().to_owned()).unwrap_or_default();
            return if caps.get(1).is_none() {
                ComplexField::Hyperlink { href: Some(location), anchor: None }
            } else {
                ComplexField::Hyperlink { href: None, anchor: Some(location) }
            };
        }

        if instr_text.contains("FORMCHECKBOX") {
            let checkbox_element = fld_char.first_or_empty("w:ffData").first_or_empty("w:checkBox");
            let checked = match checkbox_element.first("w:checked") {
                None => read_boolean_element(checkbox_element.first("w:default")),
                Some(checked_element) => read_boolean_element(Some(checked_element)),
            };
            return ComplexField::Checkbox { checked };
        }

        ComplexField::Unknown
    }

    fn read_symbol(&mut self, element: &XmlElement) -> Result<ReadResult, Failure> {
        // See 17.3.3.30 sym (Symbol Character) of ECMA-376 4th edition Part 1
        let Some(font) = element.attr("w:font") else {
            // dingbatToUnicode.hex(undefined, ...): `typeface.toUpperCase()`.
            return Err(Failure::new("Cannot read properties of undefined (reading 'toUpperCase')"));
        };
        let ch = element.attr("w:char");
        let mut unicode = ch.and_then(|c| dingbat_to_unicode::hex(font, c));
        if unicode.is_none()
            && let Some(c) = ch
            && c.starts_with("F0")
            && js::utf16_len(c) == 4
            && !c.chars().any(|x| matches!(x, '\n' | '\r' | '\u{2028}' | '\u{2029}'))
        {
            let rest: String = js::from_utf16(&js::to_utf16(c)[2..]);
            unicode = dingbat_to_unicode::hex(font, &rest);
        }
        match unicode {
            Some(s) => Ok(ReadResult::element(DocElement::Text(s))),
            None => {
                self.ctx().warn(); // A w:sym element with an unsupported character was ignored
                Ok(ReadResult::empty())
            }
        }
    }

    fn read_hyperlink(&mut self, element: &'x XmlElement) -> Result<ReadResult, Failure> {
        let relationship_id = element.attr("r:id").filter(|s| !s.is_empty());
        let anchor = element.attr("w:anchor").filter(|s| !s.is_empty());
        let children = self.read_child_elements(element)?;
        let target_frame = element.attr("w:tgtFrame").filter(|s| !s.is_empty()).map(str::to_owned);
        let value = if let Some(rid) = relationship_id {
            let mut href = self.options.relationships.find_target_by_relationship_id(rid);
            if let Some(anchor) = anchor {
                let JsVal::Str(h) = &href else {
                    return Err(Failure::new("Cannot read properties of undefined (reading 'indexOf')"));
                };
                href = JsVal::Str(uris::replace_fragment(h, anchor));
            }
            vec![DocElement::Hyperlink(Hyperlink { children: children.value, href, anchor: None, target_frame })]
        } else if let Some(anchor) = anchor {
            vec![DocElement::Hyperlink(Hyperlink {
                children: children.value,
                href: JsVal::Undefined,
                anchor: Some(anchor.to_owned()),
                target_frame,
            })]
        } else {
            children.value
        };
        Ok(ReadResult { value, extra: children.extra })
    }

    fn read_sdt(&mut self, element: &'x XmlElement) -> Result<ReadResult, Failure> {
        let content_result = self.read_child_elements(element.first_or_empty("w:sdtContent"))?;
        // From the WordML standard: https://learn.microsoft.com/en-us/openspecs/office_standards/ms-docx/3350cb64-931f-41f7-8824-f18b2568ce66
        //
        // > A CT_SdtCheckbox element that specifies that the parent
        // > structured document tag is a checkbox when displayed in the
        // > document. The parent structured document tag contents MUST
        // > contain a single character and optionally an additional
        // > character in a deleted run.
        let Some(checkbox) = element.first_or_empty("w:sdtPr").first("wordml:checkbox") else {
            return Ok(content_result);
        };
        let checked_element = checkbox.first("wordml:checked");
        let is_checked = checked_element.is_some_and(|c| read_boolean_attribute_value(c.attr("wordml:val")));

        let mut has_checkbox = false;
        let mut content = content_result.value;
        for e in content.iter_mut() {
            replace_first_text(e, is_checked, &mut has_checkbox)?;
        }
        let value = if has_checkbox { content } else { vec![DocElement::Checkbox { checked: is_checked }] };
        Ok(ReadResult { value, extra: content_result.extra })
    }

    // --- tables ----------------------------------------------------------------------------------

    fn read_table(&mut self, element: &'x XmlElement) -> Result<ReadResult, Failure> {
        let properties =
            self.read_style(element.first_or_empty("w:tblPr"), "w:tblStyle", Styles::find_table_style_by_id)?;
        let children = self.read_child_elements(element)?;
        let rows = self.calculate_row_spans(children.value)?;
        Ok(ReadResult {
            value: vec![DocElement::Table(Table {
                children: rows,
                style_id: properties.style_id.filter(|s| !s.is_empty()),
                style_name: properties.name.filter(|s| !s.is_empty()),
            })],
            extra: children.extra,
        })
    }

    fn read_table_row(&mut self, element: &'x XmlElement) -> Result<ReadResult, Failure> {
        let properties = element.first_or_empty("w:trPr");

        // See 17.13.5.12 del (Deleted Table Row) of ECMA-376 4th edition Part 1
        if properties.first("w:del").is_some() {
            return Ok(ReadResult::empty());
        }

        let is_header = properties.first("w:tblHeader").is_some();
        let children = self.read_child_elements(element)?;
        Ok(ReadResult {
            value: vec![DocElement::TableRow(TableRow { children: children.value, is_header })],
            extra: children.extra,
        })
    }

    fn read_table_cell(&mut self, element: &'x XmlElement) -> Result<ReadResult, Failure> {
        let children = self.read_child_elements(element)?;
        let properties = element.first_or_empty("w:tcPr");

        let grid_span = properties.first_or_empty("w:gridSpan").attr("w:val");
        let col_span = match grid_span.filter(|s| !s.is_empty()) {
            Some(g) => js::parse_int(g, Some(10)),
            None => 1.0,
        };
        let v_merge = properties.first("w:vMerge").map(|e| match e.attr("w:val") {
            Some(v) => v == "continue" || v.is_empty(),
            None => true,
        });
        Ok(ReadResult {
            value: vec![DocElement::TableCell(TableCell {
                children: children.value,
                col_span,
                row_span: 1.0,
                v_merge,
            })],
            extra: children.extra,
        })
    }

    fn calculate_row_spans(&self, mut rows: Vec<DocElement>) -> Result<Vec<DocElement>, Failure> {
        // _.any(rows, row => row.type !== tableRow)
        let mut non_row = false;
        for row in &rows {
            match row {
                DocElement::Poison => return Err(Failure::new("Cannot read properties of undefined (reading 'type')")),
                DocElement::TableRow(_) => {}
                _ => {
                    non_row = true;
                    break;
                }
            }
        }
        if non_row {
            remove_v_merge_properties(&mut rows)?;
            self.ctx().warn(); // unexpected non-row element in table, cell merging may be incorrect
            return Ok(rows);
        }
        // _.any(rows, row => _.any(row.children, cell => cell.type !== tableCell))
        let mut non_cell = false;
        'rows: for row in &rows {
            if let DocElement::TableRow(r) = row {
                for cell in &r.children {
                    match cell {
                        DocElement::Poison => {
                            return Err(Failure::new("Cannot read properties of undefined (reading 'type')"));
                        }
                        DocElement::TableCell(_) => {}
                        _ => {
                            non_cell = true;
                            break 'rows;
                        }
                    }
                }
            }
        }
        if non_cell {
            remove_v_merge_properties(&mut rows)?;
            self.ctx().warn(); // unexpected non-cell element in table row, cell merging may be incorrect
            return Ok(rows);
        }

        // `columns` is a plain object keyed by String(cellIndex) (a colSpan of
        // 0, a negative one or NaN makes cells of one row share a key). The
        // decisions only depend on which keys are set, so the rowSpan
        // increments are applied after the walk.
        let mut columns: HashMap<String, (usize, usize)> = HashMap::new();
        let mut increments: Vec<(usize, usize)> = Vec::new();
        for (row_index, row) in rows.iter_mut().enumerate() {
            let DocElement::TableRow(r) = row else { continue };
            let mut cell_index: f64 = 0.0;
            for (cell_pos, cell) in r.children.iter_mut().enumerate() {
                let DocElement::TableCell(c) = cell else { continue };
                let key = js::number_to_string(cell_index);
                match columns.get(&key) {
                    Some(start) if c.v_merge == Some(true) => increments.push(*start),
                    _ => {
                        columns.insert(key, (row_index, cell_pos));
                        c.v_merge = Some(false);
                    }
                }
                cell_index += c.col_span;
            }
        }
        for (row_index, cell_pos) in increments {
            if let DocElement::TableRow(r) = &mut rows[row_index]
                && let DocElement::TableCell(c) = &mut r.children[cell_pos]
            {
                c.row_span += 1.0;
            }
        }

        for row in rows.iter_mut() {
            if let DocElement::TableRow(r) = row {
                r.children.retain(|c| !matches!(c, DocElement::TableCell(TableCell { v_merge: Some(true), .. })));
                for c in r.children.iter_mut() {
                    if let DocElement::TableCell(cell) = c {
                        cell.v_merge = None;
                    }
                }
            }
        }
        Ok(rows)
    }

    // --- drawings and images ------------------------------------------------------------------------

    fn read_drawing_element(&mut self, element: &'x XmlElement) -> Result<ReadResult, Failure> {
        use super::super::xml::nodes::elements_named;
        let graphic = element.get_elements_by_tag_name("a:graphic");
        let data = elements_named(&graphic, "a:graphicData");
        let pic = elements_named(&data, "pic:pic");
        let fill = elements_named(&pic, "pic:blipFill");
        let blips = elements_named(&fill, "a:blip");
        let mut results = Vec::new();
        for blip in blips {
            results.push(self.read_blip(element, blip)?);
        }
        Ok(combine(results))
    }

    fn read_blip(&mut self, element: &'x XmlElement, blip: &'x XmlElement) -> Result<ReadResult, Failure> {
        let properties_element = element.first_or_empty("wp:docPr");
        let descr = properties_element.attr("descr");
        let alt_text = if descr.is_none_or(js::js_is_blank) { properties_element.attr("title") } else { descr };

        let Some((path, source)) = self.find_blip_image_file(blip)? else {
            self.ctx().warn(); // Could not find image file for a:blip element
            return Ok(ReadResult::empty());
        };

        let image = self.read_image(path, source, alt_text.map(str::to_owned))?;
        let hlink_click = properties_element.first_or_empty("a:hlinkClick");
        let relationship_id = hlink_click.attr("r:id").filter(|s| !s.is_empty());
        let value = match relationship_id {
            Some(rid) => {
                let href = self.options.relationships.find_target_by_relationship_id(rid);
                DocElement::Hyperlink(Hyperlink { children: vec![image], href, anchor: None, target_frame: None })
            }
            None => image,
        };
        Ok(ReadResult::element(value))
    }

    fn find_blip_image_file(&self, blip: &XmlElement) -> Result<Option<(JsVal, ImageSource)>, Failure> {
        let embed = blip.attr("r:embed").filter(|s| !s.is_empty());
        let link = blip.attr("r:link").filter(|s| !s.is_empty());
        if let Some(embed) = embed {
            return self.find_embedded_image_file(embed).map(Some);
        }
        if let Some(link) = link {
            let image_path = self.options.relationships.find_target_by_relationship_id(link);
            return Ok(Some((image_path, ImageSource::External)));
        }
        Ok(None)
    }

    fn read_image_data(&mut self, element: &XmlElement) -> Result<ReadResult, Failure> {
        let relationship_id = element.attr("r:id").filter(|s| !s.is_empty());
        let Some(rid) = relationship_id else {
            self.ctx().warn(); // A v:imagedata element without a relationship ID was ignored
            return Ok(ReadResult::empty());
        };
        let (path, source) = self.find_embedded_image_file(rid)?;
        // `element.attributes["o:title"]`: the "o" namespace is not in the
        // map, so no attribute is ever named that way.
        Ok(ReadResult::element(self.read_image(path, source, element.attr("o:title").map(str::to_owned))?))
    }

    fn find_embedded_image_file(&self, relationship_id: &str) -> Result<(JsVal, ImageSource), Failure> {
        let target = self.options.relationships.find_target_by_relationship_id(relationship_id);
        let JsVal::Str(target) = target else {
            return Err(Failure::new("Cannot read properties of undefined (reading 'charAt')"));
        };
        let path = uris::uri_to_zip_entry_name("word", &target);
        let source = ImageSource::Embedded { zip: Arc::clone(self.options.docx_file), path: path.clone() };
        Ok((JsVal::Str(path), source))
    }

    fn read_image(&self, path: JsVal, source: ImageSource, alt_text: Option<String>) -> Result<DocElement, Failure> {
        let content_type = self.options.content_types.find_content_type(&path)?;
        if !content_type.is_supported() {
            self.ctx().warn(); // Image of type ... is unlikely to display in web browsers
        }
        Ok(DocElement::Image(Image { alt_text, source }))
    }
}

/// `removeVMergeProperties(rows)`: the cells keep their row span of 1.
/// (`getDescendants` reads `.children` of every element: an `undefined`
/// one throws.)
fn remove_v_merge_properties(rows: &mut [DocElement]) -> Result<(), Failure> {
    fn visit(e: &mut DocElement) -> Result<(), Failure> {
        if matches!(e, DocElement::Poison) {
            return Err(Failure::new("Cannot read properties of undefined (reading 'children')"));
        }
        if let DocElement::TableCell(c) = e {
            c.v_merge = None;
        }
        if let Some(children) = e.children_mut() {
            for c in children.iter_mut() {
                visit(c)?;
            }
        }
        Ok(())
    }
    for row in rows.iter_mut() {
        visit(row)?;
    }
    Ok(())
}

/// The `transforms._elementsOfType("text", ...)` pass of a checkbox
/// `w:sdt`: the first non-empty text, in document order, becomes the
/// checkbox.
fn replace_first_text(element: &mut DocElement, checked: bool, has_checkbox: &mut bool) -> Result<(), Failure> {
    match element {
        DocElement::Poison => return Err(Failure::new("Cannot read properties of undefined (reading 'children')")),
        DocElement::Text(value) => {
            if !value.is_empty() && !*has_checkbox {
                *has_checkbox = true;
                *element = DocElement::Checkbox { checked };
            }
            return Ok(());
        }
        _ => {}
    }
    if let Some(children) = element.children_mut() {
        for child in children.iter_mut() {
            replace_first_text(child, checked, has_checkbox)?;
        }
    }
    Ok(())
}

/// `readNumberingProperties(styleId, element, numbering)`.
pub fn read_numbering_properties(
    style_id: Option<&str>,
    element: &XmlElement,
    numbering: &Numbering,
) -> Result<LevelResult, Failure> {
    let level = element.first_or_empty("w:ilvl").attr("w:val");
    let num_id = element.first_or_empty("w:numId").attr("w:val");
    if let (Some(level), Some(num_id)) = (level, num_id) {
        return numbering.find_level(num_id, level);
    }

    if let Some(style_id) = style_id {
        let level_by_style_id = numbering.find_level_by_paragraph_style_id(style_id);
        if level_by_style_id != LevelResult::Null {
            return Ok(level_by_style_id);
        }
    }

    // Some malformed documents define numbering levels without an index, and
    // reference the numbering using a w:numPr element without a w:ilvl child.
    // To handle such cases, we assume a level of 0 as a fallback.
    if let Some(num_id) = num_id {
        return numbering.find_level(num_id, "0");
    }

    Ok(LevelResult::Null)
}
