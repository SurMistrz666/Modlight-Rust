//! `@xmldom/xmldom` 0.8.13 as mammoth's `xml/xmldom.js` drives it:
//! `new DOMParser({ errorHandler }).parseFromString(string)` (no MIME type, so
//! the XML entity table), with an error handler that records every warning,
//! error and fatal error, after which mammoth throws. So any report at all is
//! a failure here, and the first one ends the parse.
//!
//! This is a port of `sax.js` (the lenient, regex-and-indexOf scanner, with
//! its quirks: stray end tags are ignored, a `<` that starts nothing becomes
//! text, an element whose closing tag never appears is reported as
//! "unclosed", character references go through `parseInt` and
//! `String.fromCharCode`, U+0080 inside a tag is a space) and of the DOM rules
//! that decide what survives into mammoth's tree: line endings normalised
//! before parsing, adjacent text nodes merged (`normalize()`), but not across
//! a comment, CDATA section or processing instruction, which mammoth then
//! drops; a second root element, a CDATA section or doctype in the wrong place
//! are DOM exceptions (reported, so failures).
//!
//! Positions are byte offsets into the UTF-8 source where upstream counts
//! UTF-16 units; every delimiter the scanner looks for is ASCII, so the two
//! agree on what is found where.
//!
//! Two departures, neither visible in the result: the parse stops at the
//! first report (upstream keeps going, and its error recovery is quadratic,
//! so a part full of stray `<` never finishes), and the searches sax.js
//! repeats are memoised ([`Scans`]), where upstream's are quadratic for some
//! error-free inputs.

use std::collections::HashMap;

use crate::media::mammoth::js::{self, js_is_blank, js_is_whitespace, parse_int, push_utf16, to_int32, to_uint16};

pub const NS_XML: &str = "http://www.w3.org/XML/1998/namespace";
pub const NS_XMLNS: &str = "http://www.w3.org/2000/xmlns/";
pub const NS_HTML: &str = "http://www.w3.org/1999/xhtml";

/// A namespace URI as xmldom stores it: looked up in a plain object, so an
/// undeclared prefix that happens to be an `Object.prototype` member finds a
/// function (or the prototype object).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NsValue {
    Undefined,
    Str(String),
    Builtin(&'static str),
}

impl NsValue {
    fn is_html(&self) -> bool {
        matches!(self, NsValue::Str(s) if s == NS_HTML)
    }
}

#[derive(Debug, Clone)]
pub struct DomAttr {
    pub ns: NsValue,
    pub local_name: String,
    pub value: String,
}

#[derive(Debug, Clone)]
pub struct DomElement {
    pub ns: NsValue,
    pub local_name: String,
    pub attrs: Vec<DomAttr>,
    pub children: Vec<DomChild>,
}

#[derive(Debug, Clone)]
pub enum DomChild {
    Element(DomElement),
    Text(String),
    /// A comment, CDATA section, processing instruction or doctype: dropped by
    /// mammoth, but it keeps the text on either side apart.
    Other,
}

/// Something xmldom reported (or threw); the message is for debugging.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XmlError(pub String);

type R<T> = Result<T, XmlError>;

fn fail<T>(message: impl Into<String>) -> R<T> {
    Err(XmlError(message.into()))
}

/// `normalizeLineEndings`.
pub fn normalize_line_endings(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if matches!(chars.peek(), Some('\n') | Some('\u{85}')) {
                    chars.next();
                }
                out.push('\n');
            }
            '\u{85}' | '\u{2028}' => out.push('\n'),
            other => out.push(other),
        }
    }
    out
}

fn is_name_start_char(c: char) -> bool {
    matches!(c,
        'A'..='Z' | '_' | 'a'..='z'
        | '\u{C0}'..='\u{D6}' | '\u{D8}'..='\u{F6}' | '\u{F8}'..='\u{2FF}'
        | '\u{370}'..='\u{37D}' | '\u{37F}'..='\u{1FFF}' | '\u{200C}'..='\u{200D}'
        | '\u{2070}'..='\u{218F}' | '\u{2C00}'..='\u{2FEF}' | '\u{3001}'..='\u{D7FF}'
        | '\u{F900}'..='\u{FDCF}' | '\u{FDF0}'..='\u{FFFD}')
}

fn is_name_char(c: char) -> bool {
    is_name_start_char(c)
        || matches!(c, '-' | '.' | '0'..='9' | '\u{B7}' | '\u{300}'..='\u{36F}' | '\u{203F}'..='\u{2040}')
}

/// `tagNamePattern`: Name, optionally `prefix:local` (BMP only, as the
/// non-unicode regex sees surrogates as non-name characters).
fn is_tag_name(name: &str) -> bool {
    fn ncname(part: &str) -> bool {
        let mut chars = part.chars();
        chars.next().is_some_and(is_name_start_char) && chars.all(is_name_char)
    }
    match name.split_once(':') {
        Some((prefix, local)) => ncname(prefix) && ncname(local),
        None => ncname(name),
    }
}

/// `String.fromCharCode` of what `fixedFromCharCode(code)` computes.
fn fixed_from_char_code(code: f64, out: &mut String) {
    if code > 65535.0 {
        let code = code - 65536.0;
        let surrogate1 = 0xD800_i64 + (to_int32(code) >> 10) as i64;
        let surrogate2 = 0xDC00_i64 + (to_int32(code) & 0x3FF) as i64;
        push_utf16(out, &[to_uint16(surrogate1 as f64), to_uint16(surrogate2 as f64)]);
    } else {
        push_utf16(out, &[to_uint16(code)]);
    }
}

fn is_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// `.replace(/&#?\w+;/g, entityReplacer)`.
fn replace_entities(text: &str) -> R<String> {
    if !text.contains('&') {
        return Ok(text.to_owned());
    }
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'&' {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        if j < bytes.len() && bytes[j] == b'#' {
            j += 1;
        }
        let word_start = j;
        while j < bytes.len() && is_word(bytes[j]) {
            j += 1;
        }
        if j == word_start || j >= bytes.len() || bytes[j] != b';' {
            i += 1;
            continue;
        }
        out.push_str(&text[last..i]);
        let k = &text[i + 1..j];
        match k {
            "amp" => out.push('&'),
            "apos" => out.push('\''),
            "gt" => out.push('>'),
            "lt" => out.push('<'),
            "quot" => out.push('"'),
            _ if k.starts_with('#') => {
                let digits = k[1..].replacen('x', "0x", 1);
                fixed_from_char_code(parse_int(&digits, None), &mut out);
            }
            _ => return fail(format!("entity not found:&{k};")),
        }
        i = j + 1;
        last = i;
    }
    out.push_str(&text[last..]);
    Ok(out)
}

/// A namespace map (`currentNSMap`): a plain object copied per element that
/// declares something.
type NsMap = HashMap<String, String>;

fn ns_lookup(map: &NsMap, prefix: &str) -> NsValue {
    if let Some(v) = map.get(prefix) {
        return NsValue::Str(v.clone());
    }
    match js::builtin(prefix) {
        Some(b) => NsValue::Builtin(b),
        None => NsValue::Undefined,
    }
}

struct SaxAttr {
    qname: String,
    value: String,
    prefix: Option<String>,
    local_name: String,
    uri: NsValue,
}

/// `ElementAttributes`.
#[derive(Default)]
struct ElementAttributes {
    tag_name: String,
    attrs: Vec<SaxAttr>,
    /// `attributeNames` (a plain object).
    names: HashMap<String, usize>,
    /// An attribute named `hasOwnProperty` replaced the method on
    /// `attributeNames`: the next `hasOwnProperty(...)` call throws.
    has_own_property_shadowed: bool,
    closed: bool,
}

impl ElementAttributes {
    fn set_tag_name(&mut self, name: &str) -> R<()> {
        if !is_tag_name(name) {
            return fail(format!("invalid tagName:{name}"));
        }
        self.tag_name = name.to_owned();
        Ok(())
    }

    /// `addAttribute(qname, value)` of `parseElementStartPart`.
    fn add_attribute(&mut self, qname: &str, raw_value: &str) -> R<()> {
        if self.has_own_property_shadowed {
            return fail("el.attributeNames.hasOwnProperty is not a function");
        }
        if self.names.contains_key(qname) {
            return fail(format!("Attribute {qname} redefined"));
        }
        let normalised: String =
            raw_value.chars().map(|c| if matches!(c, '\t' | '\n' | '\r') { ' ' } else { c }).collect();
        let value = replace_entities(&normalised)?;
        if !is_tag_name(qname) {
            return fail(format!("invalid attribute:{qname}"));
        }
        // `this.attributeNames[qName] = this.length`: a number assigned to
        // `__proto__` is dropped, so that name never counts as defined.
        match qname {
            "__proto__" => {}
            "hasOwnProperty" => {
                self.has_own_property_shadowed = true;
                self.names.insert(qname.to_owned(), self.attrs.len());
            }
            _ => {
                self.names.insert(qname.to_owned(), self.attrs.len());
            }
        }
        self.attrs.push(SaxAttr {
            qname: qname.to_owned(),
            value,
            prefix: None,
            local_name: String::new(),
            uri: NsValue::Undefined,
        });
        Ok(())
    }
}

const S_TAG: u8 = 0;
const S_ATTR: u8 = 1;
const S_ATTR_SPACE: u8 = 2;
const S_EQ: u8 = 3;
const S_ATTR_NOQUOT_VALUE: u8 = 4;
const S_ATTR_END: u8 = 5;
const S_TAG_SPACE: u8 = 6;
const S_TAG_CLOSE: u8 = 7;

fn char_at(source: &str, p: usize) -> Option<char> {
    source.get(p..).and_then(|s| s.chars().next())
}

fn is_html_boolean_attribute(name: &str) -> bool {
    ["disabled", "checked", "selected"].iter().any(|n| name.eq_ignore_ascii_case(n))
}

/// `parseElementStartPart`: returns the index of the closing `>`.
fn parse_element_start_part(source: &str, tag_start: usize, el: &mut ElementAttributes, ns: &NsMap) -> R<usize> {
    let mut start = tag_start + 1;
    let mut p = start;
    let mut s = S_TAG;
    let mut attr_name = String::new();
    let default_is_html = ns_lookup(ns, "").is_html();
    loop {
        let Some(mut c) = char_at(source, p) else {
            return fail("unexpected end of input");
        };
        let width = c.len_utf8();
        match c {
            '=' => {
                if s == S_ATTR {
                    attr_name = source[start..p].to_owned();
                    s = S_EQ;
                } else if s == S_ATTR_SPACE {
                    s = S_EQ;
                } else {
                    return fail("attribute equal must after attrName");
                }
            }
            '\'' | '"' => {
                if s == S_EQ || s == S_ATTR {
                    if s == S_ATTR {
                        return fail("attribute value must after \"=\"");
                    }
                    start = p + 1;
                    match source[start..].find(c) {
                        Some(i) => {
                            p = start + i;
                            let value = source[start..p].to_owned();
                            el.add_attribute(&attr_name, &value)?;
                            s = S_ATTR_END;
                        }
                        None => return fail(format!("attribute value no end '{c}' match")),
                    }
                } else if s == S_ATTR_NOQUOT_VALUE {
                    return fail("attribute missed start quot");
                } else {
                    return fail("attribute value must after \"=\"");
                }
            }
            '/' => match s {
                S_TAG => {
                    el.set_tag_name(&source[start..p])?;
                    s = S_TAG_CLOSE;
                    el.closed = true;
                }
                S_ATTR_END | S_TAG_SPACE | S_TAG_CLOSE => {
                    s = S_TAG_CLOSE;
                    el.closed = true;
                }
                S_ATTR_NOQUOT_VALUE | S_ATTR => {}
                S_ATTR_SPACE => el.closed = true,
                _ => return fail("attribute invalid close char('/')"),
            },
            '>' => {
                match s {
                    S_TAG => el.set_tag_name(&source[start..p])?,
                    S_ATTR_END | S_TAG_SPACE | S_TAG_CLOSE => {}
                    S_ATTR_NOQUOT_VALUE => return fail("attribute missed quot"),
                    S_ATTR | S_ATTR_SPACE => {
                        let mut value = if s == S_ATTR_SPACE {
                            attr_name.clone()
                        } else {
                            let mut v = source[start..p].to_owned();
                            if v.ends_with('/') {
                                el.closed = true;
                                v.pop();
                            }
                            v
                        };
                        if s == S_ATTR_SPACE {
                            value = attr_name.clone();
                        }
                        if !default_is_html || !is_html_boolean_attribute(&value) {
                            return fail(format!("attribute \"{value}\" missed value!!"));
                        }
                        el.add_attribute(&value.clone(), &value)?;
                    }
                    _ => return fail("attribute value missed!!"),
                }
                return Ok(p);
            }
            _ => {
                if c == '\u{80}' {
                    c = ' ';
                }
                if c <= ' ' {
                    match s {
                        S_TAG => {
                            el.set_tag_name(&source[start..p])?;
                            s = S_TAG_SPACE;
                        }
                        S_ATTR => {
                            attr_name = source[start..p].to_owned();
                            s = S_ATTR_SPACE;
                        }
                        S_ATTR_NOQUOT_VALUE => return fail("attribute missed quot"),
                        S_ATTR_END => s = S_TAG_SPACE,
                        _ => {}
                    }
                } else {
                    match s {
                        S_ATTR_SPACE => {
                            if !default_is_html || !is_html_boolean_attribute(&attr_name) {
                                return fail(format!("attribute \"{attr_name}\" missed value!!"));
                            }
                            let name = attr_name.clone();
                            el.add_attribute(&name, &name)?;
                            start = p;
                            s = S_ATTR;
                        }
                        S_ATTR_END => return fail("attribute space is required"),
                        S_TAG_SPACE => {
                            s = S_ATTR;
                            start = p;
                        }
                        S_EQ => {
                            s = S_ATTR_NOQUOT_VALUE;
                            start = p;
                        }
                        S_TAG_CLOSE => return fail("elements closed character '/' and '>' must be connected to"),
                        _ => {}
                    }
                }
            }
        }
        p += width;
    }
}

/// Where new nodes go (`domBuilder.currentElement`).
struct Builder {
    /// Open elements, innermost last.
    stack: Vec<DomElement>,
    root: Option<DomElement>,
    root_started: bool,
    has_doctype: bool,
    in_cdata: bool,
    max_depth: usize,
}

impl Builder {
    fn append_child_to_top(&mut self, child: DomChild) {
        let top = self.stack.last_mut().expect("open element");
        if let DomChild::Text(text) = &child
            && let Some(DomChild::Text(last)) = top.children.last_mut()
        {
            // normalize(): adjacent text nodes become one.
            last.push_str(text);
            return;
        }
        top.children.push(child);
    }

    /// `characters(chars)`.
    fn characters(&mut self, chars: &str) -> R<()> {
        if chars.is_empty() {
            return Ok(());
        }
        let node = if self.in_cdata { DomChild::Other } else { DomChild::Text(chars.to_owned()) };
        if !self.stack.is_empty() {
            self.append_child_to_top(node);
        } else if self.root_started || js_is_blank(chars) {
            // The document itself takes text, but a CDATA section is not
            // insertable there (HierarchyRequestError).
            if self.in_cdata {
                return fail("Unexpected node type 4 for parent node type 9");
            }
        }
        Ok(())
    }

    /// A comment or processing instruction: anywhere.
    fn other(&mut self) {
        if !self.stack.is_empty() {
            self.append_child_to_top(DomChild::Other);
        }
    }

    fn doctype(&mut self) -> R<()> {
        if !self.stack.is_empty() {
            self.append_child_to_top(DomChild::Other);
            return Ok(());
        }
        if self.has_doctype {
            return fail("Only one doctype is allowed");
        }
        if self.root_started {
            return fail("Doctype can not be appended since element is present");
        }
        self.has_doctype = true;
        Ok(())
    }

    fn start_element(&mut self, el: DomElement) -> R<()> {
        if self.stack.is_empty() {
            if self.root_started {
                return fail("Only one element can be added and only after doctype");
            }
            self.root_started = true;
        }
        if self.stack.len() >= self.max_depth {
            return fail("elements nested too deeply");
        }
        self.stack.push(el);
        Ok(())
    }

    fn end_element(&mut self) {
        let el = self.stack.pop().expect("open element");
        match self.stack.last_mut() {
            Some(parent) => parent.children.push(DomChild::Element(el)),
            None => self.root = Some(el),
        }
    }
}

/// `appendElement(el, domBuilder, currentNSMap)` of sax.js: resolves the
/// namespaces, starts (and for `<x/>` ends) the element; returns the map the
/// element's children see when it stays open.
fn append_element(el: &mut ElementAttributes, builder: &mut Builder, current: &NsMap) -> R<Option<NsMap>> {
    let mut map: Option<NsMap> = None;
    for a in el.attrs.iter_mut().rev() {
        let ns_prefix: Option<String>;
        match a.qname.find(':').filter(|i| *i > 0) {
            Some(nsp) => {
                let prefix = a.qname[..nsp].to_owned();
                a.local_name = a.qname[nsp + 1..].to_owned();
                ns_prefix = (prefix == "xmlns").then(|| a.local_name.clone());
                a.prefix = Some(prefix);
            }
            None => {
                a.local_name = a.qname.clone();
                ns_prefix = (a.qname == "xmlns").then(String::new);
            }
        }
        if let Some(ns_prefix) = ns_prefix {
            let m = map.get_or_insert_with(|| current.clone());
            // A string assigned to `__proto__` is dropped.
            if ns_prefix != "__proto__" {
                m.insert(ns_prefix, a.value.clone());
            }
            a.uri = NsValue::Str(NS_XMLNS.to_owned());
        }
    }
    let effective = map.as_ref().unwrap_or(current);
    for a in el.attrs.iter_mut().rev() {
        if let Some(prefix) = a.prefix.clone().filter(|p| !p.is_empty()) {
            if prefix == "xml" {
                a.uri = NsValue::Str(NS_XML.to_owned());
            }
            if prefix != "xmlns" {
                a.uri = ns_lookup(effective, &prefix);
            }
        }
    }
    let tag_name = el.tag_name.clone();
    let (prefix, local_name) = match tag_name.find(':').filter(|i| *i > 0) {
        Some(nsp) => (tag_name[..nsp].to_owned(), tag_name[nsp + 1..].to_owned()),
        None => (String::new(), tag_name.clone()),
    };
    let ns = ns_lookup(effective, &prefix);

    // startElement: createElementNS + one setAttributeNode per attribute,
    // which replaces an earlier attribute of the same qualified name in place.
    let mut attrs: Vec<(String, DomAttr)> = Vec::new();
    for a in &el.attrs {
        let local = match a.qname.split_once(':') {
            Some((_, l)) => l.to_owned(),
            None => a.qname.clone(),
        };
        let attr = DomAttr { ns: a.uri.clone(), local_name: local, value: a.value.clone() };
        match attrs.iter_mut().find(|(q, _)| *q == a.qname) {
            Some(slot) => slot.1 = attr,
            None => attrs.push((a.qname.clone(), attr)),
        }
    }
    let dom = DomElement {
        ns: ns.clone(),
        local_name,
        attrs: attrs.into_iter().map(|(_, a)| a).collect(),
        children: Vec::new(),
    };
    builder.start_element(dom)?;
    if el.closed {
        builder.end_element();
        Ok(None)
    } else {
        Ok(Some(map.unwrap_or_else(|| current.clone())))
    }
}

/// One entry of sax.js's `parseStack`.
struct StackEntry {
    tag_name: Option<String>,
    ns: NsMap,
}

/// The forward and backward searches sax.js repeats, answered in amortised
/// linear time with the same results. Upstream runs them naively, which is
/// quadratic for some inputs that never produce an error (many unterminated
/// `<?` or CDATA sections, many distinct element names each looked up with
/// `lastIndexOf` over the whole part): it then never finishes.
struct Scans<'s> {
    source: &'s str,
    /// `closeMap` of fixSelfClosed.
    close_map: HashMap<String, isize>,
    close_index: Option<CloseIndex<'s>>,
    /// Per pattern: the last (from, found) of a forward search.
    finds: HashMap<String, (usize, Option<usize>)>,
}

/// Every `</` in the source with the run of name characters after it.
struct CloseIndex<'s> {
    /// Last position of `</run>`.
    exact: HashMap<&'s str, usize>,
    /// Distinct runs, sorted, with the last position of `</run`.
    runs: Vec<(&'s str, usize)>,
}

fn is_tag_name_char(c: char) -> bool {
    is_name_char(c) || c == ':'
}

impl<'s> CloseIndex<'s> {
    fn build(source: &'s str) -> CloseIndex<'s> {
        let mut exact: HashMap<&'s str, usize> = HashMap::new();
        let mut last: HashMap<&'s str, usize> = HashMap::new();
        let bytes = source.as_bytes();
        let mut i = 0;
        while let Some(off) = bytes.get(i..).and_then(|b| b.windows(2).position(|w| w == b"</")) {
            let p = i + off;
            let rest = &source[p + 2..];
            let len: usize = rest.chars().take_while(|c| is_tag_name_char(*c)).map(char::len_utf8).sum();
            let run = &rest[..len];
            last.insert(run, p);
            if rest[len..].starts_with('>') {
                exact.insert(run, p);
            }
            i = p + 2;
        }
        let mut runs: Vec<(&'s str, usize)> = last.into_iter().collect();
        runs.sort_unstable();
        CloseIndex { exact, runs }
    }

    /// `source.lastIndexOf('</' + name + '>')`.
    fn last_exact(&self, name: &str) -> isize {
        self.exact.get(name).map_or(-1, |p| *p as isize)
    }

    /// `source.lastIndexOf('</' + name)`: a tag name is made of name
    /// characters, so `</` + name occurs exactly where the run after `</`
    /// starts with the name. Each run is visited once per distinct name that
    /// is a prefix of it, so all queries together stay linear in the source.
    fn last_prefix(&self, name: &str) -> isize {
        let from = self.runs.partition_point(|(run, _)| *run < name);
        self.runs[from..]
            .iter()
            .take_while(|(run, _)| run.starts_with(name))
            .map(|(_, p)| *p as isize)
            .max()
            .unwrap_or(-1)
    }
}

impl<'s> Scans<'s> {
    fn new(source: &'s str) -> Self {
        Scans { source, close_map: HashMap::new(), close_index: None, finds: HashMap::new() }
    }

    /// `source.indexOf(pattern, from)`.
    fn find(&mut self, pattern: &str, from: usize) -> Option<usize> {
        if let Some(&(was_from, found)) = self.finds.get(pattern)
            && was_from <= from
        {
            match found {
                None => return None,
                Some(f) if f >= from => return Some(f),
                Some(_) => {}
            }
        }
        let found = self.source.get(from..).and_then(|s| s.find(pattern)).map(|i| i + from);
        self.finds.insert(pattern.to_owned(), (from, found));
        found
    }

    /// `fixSelfClosed(source, elStartEnd, tagName, closeMap)`.
    fn fix_self_closed(&mut self, el_start_end: usize, tag_name: &str) -> bool {
        // closeMap is a plain object: an inherited member is found as a
        // function, and `function < number` is false.
        if js::builtin(tag_name).is_some() {
            return false;
        }
        if let Some(pos) = self.close_map.get(tag_name) {
            return *pos < el_start_end as isize;
        }
        let source = self.source;
        let index = self.close_index.get_or_insert_with(|| CloseIndex::build(source));
        let mut pos = index.last_exact(tag_name);
        if pos < el_start_end as isize {
            pos = index.last_prefix(tag_name);
        }
        self.close_map.insert(tag_name.to_owned(), pos);
        pos < el_start_end as isize
    }
}

/// JavaScript `substring(a, b)` (clamped, swapped when reversed).
fn js_substring(source: &str, a: isize, b: isize) -> &str {
    let len = source.len() as isize;
    let a = a.clamp(0, len) as usize;
    let b = b.clamp(0, len) as usize;
    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
    // Positions come from ASCII delimiters or clamping, so they are char
    // boundaries except in contrived inputs; fall back to the nearest.
    let lo = (0..=lo).rev().find(|i| source.is_char_boundary(*i)).unwrap_or(0);
    let hi = (hi..=source.len()).find(|i| source.is_char_boundary(*i)).unwrap_or(source.len());
    &source[lo..hi]
}

/// `parseHtmlSpecialContent` (only for elements in the XHTML namespace).
fn parse_html_special_content(
    scans: &mut Scans<'_>,
    el_start_end: usize,
    tag_name: &str,
    builder: &mut Builder,
) -> R<isize> {
    let source = scans.source;
    let lower = tag_name.to_ascii_lowercase();
    if lower == "script" || lower == "textarea" {
        let close = format!("</{tag_name}>");
        let el_end_start = scans.find(&close, el_start_end).map_or(-1, |i| i as isize);
        let text = js_substring(source, el_start_end as isize + 1, el_end_start);
        if text.contains('&') || text.contains('<') {
            if lower == "script" {
                let t = text.to_owned();
                builder.characters(&t)?;
                return Ok(el_end_start);
            }
            let t = replace_entities(text)?;
            builder.characters(&t)?;
            return Ok(el_end_start);
        }
    }
    Ok(el_start_end as isize + 1)
}

/// The doctype tokens of sax.js's `split()`: `None` when the scan runs out
/// before a `>` or `<` token (upstream then throws on `undefined.length`).
fn split_tokens(source: &str, start: usize) -> Option<Vec<(usize, String, bool)>> {
    let bytes = source.as_bytes();
    let mut i = start;
    let mut out: Vec<(usize, String, bool)> = Vec::new();
    let mut skipped_first = false;
    while i < bytes.len() {
        let rest = &source[i..];
        let c = rest.chars().next().unwrap_or('\0');
        // 1: '[^']+'  2: "[^"]+"  3: [^\s<>\/=]+=?  4: (\/?\s*>|<)
        let quoted = |q: char| -> Option<usize> {
            if c != q {
                return None;
            }
            let body = &rest[1..];
            let close = body.find(q)?;
            (close > 0).then_some(1 + close + 1)
        };
        let token_len = quoted('\'').or_else(|| quoted('"')).or_else(|| {
            let n: usize = rest
                .chars()
                .take_while(|ch| !js_is_whitespace(*ch) && !matches!(ch, '<' | '>' | '/' | '='))
                .map(|ch| ch.len_utf8())
                .sum();
            if n == 0 {
                return None;
            }
            Some(if rest[n..].starts_with('=') { n + 1 } else { n })
        });
        if let Some(n) = token_len {
            if skipped_first {
                out.push((i, rest[..n].to_owned(), false));
            }
            skipped_first = true;
            i += n;
            continue;
        }
        // group 1: \/?\s*> or <
        let mut j = 0;
        if rest.starts_with('/') {
            j = 1;
        }
        let ws: usize = rest[j..].chars().take_while(|ch| js_is_whitespace(*ch)).map(|ch| ch.len_utf8()).sum();
        let group = if rest[j + ws..].starts_with('>') {
            Some(j + ws + 1)
        } else if rest.starts_with('<') {
            Some(1)
        } else {
            None
        };
        match group {
            Some(n) => {
                if skipped_first {
                    out.push((i, rest[..n].to_owned(), true));
                    return Some(out);
                }
                skipped_first = true;
                i += n;
            }
            None => i += c.len_utf8(),
        }
    }
    None
}

/// `parseDCC` (`<!`): comments, CDATA sections, doctypes.
fn parse_dcc(scans: &mut Scans<'_>, start: usize, builder: &mut Builder) -> R<isize> {
    let source = scans.source;
    let next = source.as_bytes().get(start + 2).copied();
    if next == Some(b'-') {
        if source.as_bytes().get(start + 3) == Some(&b'-') {
            return match source.get(start + 4..).and_then(|s| s.find("-->")) {
                Some(i) => {
                    builder.other();
                    Ok((start + 4 + i + 3) as isize)
                }
                None => fail("Unclosed comment"),
            };
        }
        return Ok(-1);
    }
    if source.get(start + 3..start + 9) == Some("CDATA[") {
        let end = scans.find("]]>", start + 9);
        builder.in_cdata = true;
        if let Some(end) = end {
            let content = source[start + 9..end].to_owned();
            builder.characters(&content)?;
        }
        builder.in_cdata = false;
        return Ok(end.map_or(2, |e| (e + 3) as isize));
    }
    let Some(tokens) = split_tokens(source, start) else {
        return fail("Cannot read properties of undefined (reading 'length')");
    };
    if tokens.len() > 1 && tokens[0].1.to_ascii_lowercase().contains("!doctype") {
        builder.doctype()?;
        let last = &tokens[tokens.len() - 1];
        return Ok((last.0 + last.1.len()) as isize);
    }
    Ok(-1)
}

/// `parseInstruction` (`<?`).
fn parse_instruction(scans: &mut Scans<'_>, start: usize, builder: &mut Builder) -> isize {
    let source = scans.source;
    match scans.find("?>", start) {
        Some(i) => {
            builder.other();
            (i + 2) as isize
        }
        None => {
            // `if (end)` is true for -1: the regex then runs on
            // `substring(start, -1)`, which is `source[0..start]`.
            if start >= 2 && source.starts_with("<?") {
                builder.other();
                1
            } else {
                -1
            }
        }
    }
}

/// `DOMParser.parseFromString(source)` as mammoth calls it: the document
/// element, or the first thing xmldom would have reported. `max_depth`
/// bounds element nesting (see the module docs of `mammoth`).
pub fn parse_from_string(source: &str, max_depth: usize) -> R<DomElement> {
    if source.is_empty() {
        return fail("invalid doc source");
    }
    let source = normalize_line_endings(source);
    let source = source.as_str();
    let mut builder =
        Builder { stack: Vec::new(), root: None, root_started: false, has_doctype: false, in_cdata: false, max_depth };
    let mut base_ns: NsMap = HashMap::new();
    base_ns.insert("xml".into(), NS_XML.into());
    let mut parse_stack: Vec<StackEntry> = vec![StackEntry { tag_name: None, ns: base_ns }];
    let mut scans = Scans::new(source);
    let mut start: usize = 0;

    let append_text = |builder: &mut Builder, start: &mut usize, end: usize| -> R<()> {
        if end > *start {
            let xt = replace_entities(&source[*start..end])?;
            builder.characters(&xt)?;
            *start = end;
        }
        Ok(())
    };

    // Trailing text after the last `<` goes to the document node: never part
    // of the tree.
    while let Some(found) = source[start..].find('<') {
        let tag_start = start + found;
        if tag_start > start {
            append_text(&mut builder, &mut start, tag_start)?;
        }
        let end: isize = match source.as_bytes().get(tag_start + 1) {
            Some(b'/') => {
                // indexOf('>', tagStart + 3): a byte search, as the skipped
                // units are "</" and the first unit of the name.
                let gt = source
                    .as_bytes()
                    .get(tag_start + 3..)
                    .and_then(|b| b.iter().position(|x| *x == b'>'))
                    .map(|i| i + tag_start + 3);
                let Some(gt) = gt else {
                    return fail("end tag name is not complete");
                };
                let raw = &source[tag_start + 2..gt];
                let tag_name = raw.trim_end_matches([' ', '\t', '\n', '\r']);
                let mut chars = tag_name.chars().peekable();
                while let Some(c) = chars.next() {
                    if js_is_whitespace(c) && chars.peek() == Some(&'<') {
                        return fail("end tag name maybe not complete");
                    }
                }
                let config = parse_stack.pop().expect("parse stack base");
                let end_match = config.tag_name.as_deref() == Some(tag_name);
                let end_ignore_case = end_match
                    || config.tag_name.as_deref().is_some_and(|t| t.to_lowercase() == tag_name.to_lowercase());
                if end_ignore_case {
                    builder.end_element();
                    if !end_match {
                        return fail("end tag name is not match the current start tagName");
                    }
                } else {
                    parse_stack.push(config);
                }
                gt as isize + 1
            }
            Some(b'?') => parse_instruction(&mut scans, tag_start, &mut builder),
            Some(b'!') => parse_dcc(&mut scans, tag_start, &mut builder)?,
            _ => {
                let mut el = ElementAttributes::default();
                let current_ns = parse_stack.last().expect("parse stack base").ns.clone();
                let end = parse_element_start_part(source, tag_start, &mut el, &current_ns)?;
                if !el.closed && scans.fix_self_closed(end, &el.tag_name) {
                    el.closed = true;
                    // `if (!entityMap.nbsp)`: always, with the XML entities.
                    return fail("unclosed xml attribute");
                }
                let tag_name = el.tag_name.clone();
                let opened = append_element(&mut el, &mut builder, &current_ns)?;
                let el_ns_is_html = match (&opened, builder.stack.last()) {
                    (Some(_), Some(top)) => top.ns.is_html(),
                    _ => false,
                };
                if let Some(ns) = opened {
                    parse_stack.push(StackEntry { tag_name: Some(tag_name.clone()), ns });
                }
                if el_ns_is_html && !el.closed {
                    parse_html_special_content(&mut scans, end, &tag_name, &mut builder)?
                } else {
                    end as isize + 1
                }
            }
        };
        if end > start as isize {
            start = end as usize;
        } else {
            // A `<` that started nothing: it is text.
            let next = tag_start.max(start) + 1;
            append_text(&mut builder, &mut start, next)?;
        }
    }

    // Elements still open at the end are in the tree all the same.
    while !builder.stack.is_empty() {
        builder.end_element();
    }
    match builder.root {
        Some(root) => Ok(root),
        // `document.documentElement.tagName` on null.
        None => fail("Cannot read properties of null (reading 'tagName')"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> R<DomElement> {
        parse_from_string(s, 10_000)
    }

    fn texts(el: &DomElement) -> Vec<String> {
        el.children
            .iter()
            .map(|c| match c {
                DomChild::Text(t) => format!("T:{t}"),
                DomChild::Element(e) => format!("E:{}", e.local_name),
                DomChild::Other => "O".into(),
            })
            .collect()
    }

    #[test]
    fn parses_namespaces_and_attributes() {
        let root = parse(r#"<?xml version="1.0"?><w:a xmlns:w="urn:w" w:v='1' x = "2"><w:b/>t&amp;&#x41;&#66;</w:a>"#)
            .unwrap();
        assert_eq!(root.ns, NsValue::Str("urn:w".into()));
        assert_eq!(root.local_name, "a");
        assert_eq!(root.attrs.len(), 3);
        assert_eq!(root.attrs[1].ns, NsValue::Str("urn:w".into()));
        assert_eq!(root.attrs[2].value, "2");
        assert_eq!(texts(&root), vec!["E:b", "T:t&AB"]);
    }

    #[test]
    fn reports_what_xmldom_reports() {
        assert!(parse("").is_err());
        assert!(parse("   ").is_err());
        assert!(parse("<a><b></a>").is_err());
        assert!(parse("<a b=c/>").is_err());
        assert!(parse("<a b='1' b='2'/>").is_err());
        assert!(parse("<a>&nbsp;</a>").is_err());
        assert!(parse("<a/><b/>").is_err());
        assert!(parse("<a><!-- x</a>").is_err());
        assert!(parse("<a b='1'c='2'/>").is_err());
        assert!(parse("<1a/>").is_err());
    }

    #[test]
    fn keeps_xmldom_leniencies() {
        // Stray end tags are ignored; a bare `<` that starts nothing is text.
        let root = parse("<a>x</b>y</a>").unwrap();
        assert_eq!(texts(&root), vec!["T:xy"]);
        let root = parse("<a>1 <!x 2</a>").unwrap();
        assert_eq!(texts(&root), vec!["T:1 <!x 2"]);
        let root = parse("<a>p<!--c-->q<![CDATA[r]]>s</a>").unwrap();
        assert_eq!(texts(&root), vec!["T:p", "O", "T:q", "O", "T:s"]);
        let root = parse("<a>\r\n\u{2028}&#13;</a>").unwrap();
        assert_eq!(texts(&root), vec!["T:\n\n\r"]);
        let root = parse("<a/ >").unwrap();
        assert_eq!(root.local_name, "a");
        let root = parse("<a __proto__='1' __proto__='2'/>").unwrap();
        assert_eq!(root.attrs.len(), 1);
        assert!(parse("<a hasOwnProperty='1' b='2'/>").is_err());
        let root = parse("<x:a/>").unwrap();
        assert_eq!(root.ns, NsValue::Undefined);
        let root = parse("<constructor:a/>").unwrap();
        assert_eq!(root.ns, NsValue::Builtin("constructor"));
    }

    #[test]
    fn indexed_scans_answer_like_naive_ones() {
        let source = "</a></ab ></a:b></a></abc></b x></a\u{e9}></ab";
        let index = CloseIndex::build(source);
        for name in ["a", "ab", "abc", "a:b", "b", "a\u{e9}", "zz", "a:"] {
            let naive_exact = source.rfind(&format!("</{name}>")).map_or(-1, |i| i as isize);
            let naive_prefix = source.rfind(&format!("</{name}")).map_or(-1, |i| i as isize);
            assert_eq!(index.last_exact(name), naive_exact, "exact {name}");
            assert_eq!(index.last_prefix(name), naive_prefix, "prefix {name}");
        }
        let mut scans = Scans::new("a?>b?>c");
        assert_eq!(scans.find("?>", 0), Some(1));
        assert_eq!(scans.find("?>", 1), Some(1));
        assert_eq!(scans.find("?>", 2), Some(4));
        assert_eq!(scans.find("?>", 5), None);
        assert_eq!(scans.find("?>", 6), None);
        assert_eq!(scans.find("?>", 0), Some(1));
    }

    /// Inputs upstream scans quadratically without ever reporting an error
    /// finish in linear time here, with the same tree.
    #[test]
    fn quadratic_upstream_inputs_stay_linear() {
        let n = 200_000;
        let mut doc = String::from("<?xml version=\"1.0\"?><a>");
        for _ in 0..n {
            doc.push_str("<?x");
        }
        doc.push_str("</a>");
        let started = std::time::Instant::now();
        assert!(parse(&doc).is_ok());
        let mut doc = String::from("<r>");
        for i in 0..n {
            doc.push_str(&format!("<e{i}></e{i} >"));
        }
        doc.push_str("</r>");
        let root = parse(&doc).unwrap();
        assert_eq!(root.children.len(), n);
        let mut doc = String::from("<r>");
        for _ in 0..n {
            doc.push_str("<![CDATA[");
        }
        doc.push_str("</r>");
        assert!(parse(&doc).is_ok());
        assert!(started.elapsed() < std::time::Duration::from_secs(20), "{:?}", started.elapsed());
    }

    #[test]
    fn character_references_go_through_parse_int() {
        let root = parse("<a>&#x1F600;&#xD83D;&#xDE00;&#1x;&#X41;</a>").unwrap();
        let DomChild::Text(t) = &root.children[0] else { panic!() };
        assert_eq!(js::resolve_surrogates(t), "\u{1F600}\u{1F600}\n\0");
    }
}
