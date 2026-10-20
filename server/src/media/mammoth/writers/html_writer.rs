//! mammoth `lib/writers/html-writer.js` (the simple writer: `prettyPrint` is
//! off). Attributes come out in `Object.keys` order and their values must be
//! strings (`value.replace` throws otherwise).

use super::super::Failure;
use super::super::js::js_key_order;
use super::super::styles::html_paths::AttrValue;

#[derive(Default)]
pub struct HtmlWriter {
    out: String,
}

fn escape_html_text(value: &str, out: &mut String) {
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            other => out.push(other),
        }
    }
}

fn escape_html_attribute(value: &str, out: &mut String) {
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            other => out.push(other),
        }
    }
}

impl HtmlWriter {
    fn attribute_string(&mut self, attributes: &[(String, AttrValue)]) -> Result<(), Failure> {
        let ordered = js_key_order(attributes.iter().map(|(k, v)| (k.clone(), v)).collect());
        for (key, value) in ordered {
            let AttrValue::Str(value) = value else {
                return Err(Failure::new("value.replace is not a function"));
            };
            self.out.push(' ');
            self.out.push_str(&key);
            self.out.push_str("=\"");
            escape_html_attribute(value, &mut self.out);
            self.out.push('"');
        }
        Ok(())
    }

    pub fn open(&mut self, tag_name: &str, attributes: &[(String, AttrValue)]) -> Result<(), Failure> {
        self.out.push('<');
        self.out.push_str(tag_name);
        self.attribute_string(attributes)?;
        self.out.push('>');
        Ok(())
    }

    pub fn close(&mut self, tag_name: &str) {
        self.out.push_str("</");
        self.out.push_str(tag_name);
        self.out.push('>');
    }

    pub fn self_closing(&mut self, tag_name: &str, attributes: &[(String, AttrValue)]) -> Result<(), Failure> {
        self.out.push('<');
        self.out.push_str(tag_name);
        self.attribute_string(attributes)?;
        self.out.push_str(" />");
        Ok(())
    }

    pub fn text(&mut self, value: &str) {
        escape_html_text(value, &mut self.out);
    }

    pub fn as_string(self) -> String {
        self.out
    }
}
