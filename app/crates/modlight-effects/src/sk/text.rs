//! `SKFont` and `SKCanvas.DrawText` on top of the editor's text engine (fontdb +
//! rustybuzz). `SKTypeface.FromFamilyName` resolves through the system fonts with the
//! bundled Inter as the fallback; on Windows the fallback is Segoe UI, so text effects
//! keep upstream's layout but not its letter shapes.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use modlight_editor::model::TextAlign;
use modlight_editor::text::{self, FontRequest};
use tiny_skia::{Path, Transform};

/// `SKFont`: typeface family, size and style.
#[derive(Clone, Debug, PartialEq)]
pub struct Font {
    pub family: String,
    pub size: f32,
    pub bold: bool,
    pub italic: bool,
}

impl Font {
    pub fn new(family: &str, size: f32) -> Font {
        Font {
            family: family.to_string(),
            size,
            bold: false,
            italic: false,
        }
    }

    /// `SKTypeface.Default`.
    pub fn default_face(size: f32) -> Font {
        Font::new(text::DEFAULT_FAMILY, size)
    }

    pub fn bold(mut self, bold: bool) -> Font {
        self.bold = bold;
        self
    }

    pub fn italic(mut self, italic: bool) -> Font {
        self.italic = italic;
        self
    }

    fn request(&self) -> FontRequest {
        FontRequest {
            family: self.family.clone(),
            bold: self.bold,
            italic: self.italic,
        }
    }

    /// `SKFont.MeasureText(text)`: the advance width.
    pub fn measure(&self, s: &str) -> f32 {
        if s.is_empty() {
            return 0.0;
        }
        let laid = text::layout(s, &self.request(), self.size, None, TextAlign::Left);
        laid.lines
            .first()
            .map(|l| l.width_with_trailing)
            .unwrap_or(0.0)
    }

    /// `SKFont.Metrics`: (ascent, descent), ascent negative as Skia reports it.
    pub fn metrics(&self) -> (f32, f32) {
        let laid = text::layout("M", &self.request(), self.size, None, TextAlign::Left);
        let line = laid.lines.first();
        let ascent = line.map(|l| l.ascent).unwrap_or(self.size * 0.97);
        let height = line.map(|l| l.height).unwrap_or(self.size * 1.21);
        (-ascent, height - ascent)
    }

    /// The outline of `s` with its baseline starting at (`x`, `y`).
    pub fn path(&self, s: &str, x: f32, y: f32) -> Option<Path> {
        if s.trim().is_empty() {
            return None;
        }
        let key = (
            s.to_string(),
            self.family.clone(),
            self.size.to_bits(),
            self.bold,
            self.italic,
        );
        let cached = {
            let cache = cache().lock().unwrap();
            cache.get(&key).cloned()
        };
        let path = match cached {
            Some(p) => p,
            None => {
                let laid = text::layout(s, &self.request(), self.size, None, TextAlign::Left);
                let ascent = laid.lines.first().map(|l| l.ascent).unwrap_or(0.0);
                let p = text::build_geometry(&laid, 0.0, -ascent);
                let mut c = cache().lock().unwrap();
                if c.len() > 4096 {
                    c.clear();
                }
                c.insert(key, p.clone());
                p
            }
        }?;
        path.transform(Transform::from_translate(x, y))
    }
}

type Key = (String, String, u32, bool, bool);

fn cache() -> &'static Mutex<HashMap<Key, Option<Path>>> {
    static CACHE: OnceLock<Mutex<HashMap<Key, Option<Path>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}
