//! mammoth `lib/docx/content-types-reader.js`. Only what decides the output
//! is kept: whether `findContentType` throws, and whether the type it finds
//! is one mammoth calls displayable (otherwise it records a warning).

use super::super::Failure;
use super::super::documents::JsVal;
use super::super::js::{self, JsObject, Lookup};
use super::super::xml::XmlElement;

/// `fallbackContentTypes`.
const FALLBACK: [(&str, &str); 7] = [
    ("png", "png"),
    ("gif", "gif"),
    ("jpeg", "jpeg"),
    ("jpg", "jpeg"),
    ("tif", "tiff"),
    ("tiff", "tiff"),
    ("bmp", "bmp"),
];

/// `supportedImageTypes` of body-reader.js.
const SUPPORTED: [&str; 5] = ["image/png", "image/gif", "image/jpeg", "image/svg+xml", "image/tiff"];

#[derive(Debug, Clone, Default)]
pub struct ContentTypes {
    overrides: JsObject<Option<String>>,
    extension_defaults: JsObject<Option<String>>,
}

/// A found content type, as `supportedImageTypes[contentType]` sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentType {
    /// A string key (`String(null)` is "null", `String(undefined)` "undefined").
    Key(String),
}

impl ContentType {
    /// `supportedImageTypes[contentType]` (a plain object: an inherited
    /// member's name counts as supported).
    pub fn is_supported(&self) -> bool {
        let ContentType::Key(k) = self;
        SUPPORTED.contains(&k.as_str()) || js::builtin(k).is_some()
    }
}

impl ContentTypes {
    /// `findContentType(path)`.
    pub fn find_content_type(&self, path: &JsVal) -> Result<ContentType, Failure> {
        let key = match path {
            JsVal::Str(s) => s.clone(),
            JsVal::Undefined => "undefined".into(),
            JsVal::Builtin(b) => js::builtin_to_string(b),
        };
        match self.overrides.get(&key) {
            Lookup::Own(Some(v)) if !v.is_empty() => return Ok(ContentType::Key(v.clone())),
            Lookup::Builtin(b) => return Ok(ContentType::Key(js::builtin_to_string(b))),
            _ => {}
        }
        let JsVal::Str(path) = path else {
            return Err(Failure::new("path.split is not a function"));
        };
        let extension = path.rsplit('.').next().unwrap_or("");
        // `extensionDefaults.hasOwnProperty(extension)`: an extension named
        // `hasOwnProperty` replaced the method.
        if self.extension_defaults.has_own("hasOwnProperty") {
            return Err(Failure::new("extensionDefaults.hasOwnProperty is not a function"));
        }
        if self.extension_defaults.has_own(extension) {
            return Ok(match self.extension_defaults.get(extension) {
                Lookup::Own(Some(v)) => ContentType::Key(v.clone()),
                _ => ContentType::Key("undefined".into()),
            });
        }
        let lower = extension.to_lowercase();
        if let Some((_, f)) = FALLBACK.iter().find(|(e, _)| *e == lower) {
            return Ok(ContentType::Key(format!("image/{f}")));
        }
        if let Some(b) = js::builtin(&lower) {
            return Ok(ContentType::Key(format!("image/{}", js::builtin_to_string(b))));
        }
        Ok(ContentType::Key("null".into()))
    }
}

/// `readContentTypesFromXml(element)`.
pub fn read_content_types_from_xml(element: &XmlElement) -> Result<ContentTypes, Failure> {
    let mut types = ContentTypes::default();
    for child in element.children.iter().filter_map(|c| c.as_element()) {
        if child.name == "content-types:Default" {
            let ext = child.attr("Extension").unwrap_or("undefined");
            types.extension_defaults.set_primitive(ext, child.attr("ContentType").map(str::to_owned));
        }
        if child.name == "content-types:Override" {
            let Some(name) = child.attr("PartName") else {
                return Err(Failure::new("Cannot read properties of undefined (reading 'charAt')"));
            };
            let name = name.strip_prefix('/').unwrap_or(name);
            types.overrides.set_primitive(name, child.attr("ContentType").map(str::to_owned));
        }
    }
    Ok(types)
}

/// `defaultContentTypes`.
pub fn default_content_types() -> ContentTypes {
    ContentTypes::default()
}
