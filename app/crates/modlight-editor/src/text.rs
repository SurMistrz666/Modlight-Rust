//! Text as Avalonia lays it out for the editor: `FormattedText` with a typeface, a size,
//! a maximum width (word wrapping) and an alignment, turned into glyph outlines
//! (`BuildGeometry`) that the renderer fills and strokes.
//!
//! Fonts come from the system (`FontManager.Current.SystemFonts`) through `fontdb`; the
//! default family is Inter, which is what upstream's Avalonia uses on a Mac
//! (`.WithInterFont()`), and what a family that is not installed (upstream's default
//! "Segoe UI") falls back to. Shaping is `rustybuzz`, line breaking UAX #14.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use rustybuzz::ttf_parser;
use tiny_skia::{Path, PathBuilder};

/// The families the editor treats as Avalonia's `FontFamily.Default`.
pub const DEFAULT_FAMILY: &str = "Inter";

const INTER_REGULAR_LATIN: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular-latin.ttf");
const INTER_REGULAR_EXT: &[u8] =
    include_bytes!("../../../assets/fonts/Inter-Regular-latin-ext.ttf");
const INTER_SEMIBOLD_LATIN: &[u8] =
    include_bytes!("../../../assets/fonts/Inter-SemiBold-latin.ttf");
const INTER_SEMIBOLD_EXT: &[u8] =
    include_bytes!("../../../assets/fonts/Inter-SemiBold-latin-ext.ttf");

type FontData = Arc<dyn AsRef<[u8]> + Send + Sync>;

/// One font face with its bytes kept alive.
#[derive(Clone)]
pub struct FaceRef {
    pub data: FontData,
    pub index: u32,
    key: u64,
}

impl std::fmt::Debug for FaceRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "FaceRef({}#{})", self.key, self.index)
    }
}

impl PartialEq for FaceRef {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key && self.index == other.index
    }
}

impl FaceRef {
    pub fn with_face<T>(&self, f: impl FnOnce(&ttf_parser::Face) -> T) -> Option<T> {
        let face = ttf_parser::Face::parse((*self.data).as_ref(), self.index).ok()?;
        Some(f(&face))
    }
}

/// A typeface request (`new Typeface(family, style, weight)`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FontRequest {
    pub family: String,
    pub bold: bool,
    pub italic: bool,
}

/// The faces a request resolves to: the matched face first, then the fallbacks used for
/// characters it lacks; with Avalonia's simulations when the match is not bold or italic.
#[derive(Clone, Debug)]
pub struct ResolvedFont {
    pub faces: Vec<FaceRef>,
    pub synth_bold: bool,
    pub synth_italic: bool,
}

struct FontSystem {
    db: fontdb::Database,
    system_loaded: bool,
    shared: HashMap<fontdb::ID, FaceRef>,
    resolved: HashMap<FontRequest, ResolvedFont>,
    families: Option<Arc<Vec<String>>>,
    inter: [FaceRef; 4],
    next_key: u64,
}

fn system() -> &'static Mutex<FontSystem> {
    static SYSTEM: OnceLock<Mutex<FontSystem>> = OnceLock::new();
    SYSTEM.get_or_init(|| {
        let face = |bytes: &'static [u8], key: u64| FaceRef {
            data: Arc::new(bytes) as FontData,
            index: 0,
            key,
        };
        Mutex::new(FontSystem {
            db: fontdb::Database::new(),
            system_loaded: false,
            shared: HashMap::new(),
            resolved: HashMap::new(),
            families: None,
            inter: [
                face(INTER_REGULAR_LATIN, 1),
                face(INTER_REGULAR_EXT, 2),
                face(INTER_SEMIBOLD_LATIN, 3),
                face(INTER_SEMIBOLD_EXT, 4),
            ],
            next_key: 100,
        })
    })
}

/// Whether system fonts may be read. Snapshot tests switch this off so a render never
/// depends on what the machine has installed.
static SYSTEM_FONTS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

pub fn set_system_fonts_enabled(enabled: bool) {
    SYSTEM_FONTS.store(enabled, std::sync::atomic::Ordering::Relaxed);
}

fn system_fonts_enabled() -> bool {
    SYSTEM_FONTS.load(std::sync::atomic::Ordering::Relaxed)
}

impl FontSystem {
    fn ensure_system(&mut self) {
        if !self.system_loaded && system_fonts_enabled() {
            self.db.load_system_fonts();
            self.system_loaded = true;
        }
    }

    fn shared_face(&mut self, id: fontdb::ID) -> Option<FaceRef> {
        if let Some(face) = self.shared.get(&id) {
            return Some(face.clone());
        }
        // SAFETY: font files are memory-mapped read-only; a file changed on disk while
        // mapped is the risk fontdb documents, and the system fonts do not change.
        let (data, index) = unsafe { self.db.make_shared_face_data(id) }?;
        let key = self.next_key;
        self.next_key += 1;
        let face = FaceRef { data, index, key };
        self.shared.insert(id, face.clone());
        Some(face)
    }

    fn query(&mut self, family: &str, bold: bool, italic: bool) -> Option<(FaceRef, bool, bool)> {
        self.ensure_system();
        let families = [fontdb::Family::Name(family)];
        let query = fontdb::Query {
            families: &families,
            weight: if bold {
                fontdb::Weight::BOLD
            } else {
                fontdb::Weight::NORMAL
            },
            stretch: fontdb::Stretch::Normal,
            style: if italic {
                fontdb::Style::Italic
            } else {
                fontdb::Style::Normal
            },
        };
        let id = self.db.query(&query)?;
        let info = self.db.face(id)?;
        if !info
            .families
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case(family))
        {
            return None;
        }
        let synth_bold = bold && info.weight.0 < 600;
        let synth_italic = italic && info.style == fontdb::Style::Normal;
        let face = self.shared_face(id)?;
        Some((face, synth_bold, synth_italic))
    }

    fn inter(&self, bold: bool) -> [FaceRef; 2] {
        if bold {
            [self.inter[2].clone(), self.inter[3].clone()]
        } else {
            [self.inter[0].clone(), self.inter[1].clone()]
        }
    }

    fn resolve(&mut self, request: &FontRequest) -> ResolvedFont {
        if let Some(found) = self.resolved.get(request) {
            return found.clone();
        }
        let family = request.family.trim();
        let is_default = family.is_empty() || family.eq_ignore_ascii_case(DEFAULT_FAMILY);
        let mut faces = Vec::new();
        let mut synth_bold = false;
        let mut synth_italic = request.italic;
        if !is_default
            && let Some((face, bold, italic)) = self.query(family, request.bold, request.italic)
        {
            faces.push(face);
            synth_bold = bold;
            synth_italic = italic;
        }
        // Avalonia's default family (bundled Inter): the SemiBold cut stands in for Bold,
        // which the port does not bundle; italics are simulated.
        faces.extend(self.inter(request.bold));
        if system_fonts_enabled() {
            for fallback in ["Helvetica", "Arial Unicode MS"] {
                if let Some((face, _, _)) = self.query(fallback, false, false) {
                    faces.push(face);
                }
            }
        }
        let resolved = ResolvedFont {
            faces,
            synth_bold,
            synth_italic,
        };
        self.resolved.insert(request.clone(), resolved.clone());
        resolved
    }

    fn families(&mut self) -> Arc<Vec<String>> {
        if let Some(families) = &self.families {
            return families.clone();
        }
        self.ensure_system();
        let mut names: Vec<String> = self
            .db
            .faces()
            .flat_map(|face| face.families.first().map(|(name, _)| name.clone()))
            .filter(|name| !name.starts_with('.') && !name.is_empty())
            .collect();
        names.push(DEFAULT_FAMILY.to_string());
        names.sort_by_key(|name| name.to_lowercase());
        names.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        let names = Arc::new(names);
        self.families = Some(names.clone());
        names
    }
}

pub fn resolve(request: &FontRequest) -> ResolvedFont {
    system().lock().unwrap().resolve(request)
}

/// `FontManager.Current.SystemFonts` names, sorted like the font picker sorts them.
pub fn system_families() -> Arc<Vec<String>> {
    system().lock().unwrap().families()
}

/// A face by family name (for the emoji renderer: "Apple Color Emoji").
pub fn find_face(family: &str) -> Option<FaceRef> {
    if !system_fonts_enabled() {
        return None;
    }
    let mut system = system().lock().unwrap();
    system.query(family, false, false).map(|(face, _, _)| face)
}

/// The face Avalonia's default family uses for a run (bundled Inter).
pub fn default_faces(bold: bool) -> [FaceRef; 2] {
    system().lock().unwrap().inter(bold)
}

// ---------------------------------------------------------------------------------------
// Layout

/// One glyph placed on a line, relative to the line's left edge and baseline.
#[derive(Clone, Debug)]
pub struct PlacedGlyph {
    pub face: FaceRef,
    pub glyph: u16,
    pub x: f32,
    pub y: f32,
    pub scale: f32,
}

#[derive(Clone, Debug, Default)]
pub struct Line {
    pub glyphs: Vec<PlacedGlyph>,
    /// Width without trailing white space.
    pub width: f32,
    pub width_with_trailing: f32,
    pub ascent: f32,
    pub height: f32,
}

/// `FormattedText` after layout.
#[derive(Clone, Debug, Default)]
pub struct TextLayout {
    pub lines: Vec<Line>,
    pub width: f32,
    pub height: f32,
    pub synth_bold: bool,
    pub synth_italic: bool,
    pub size: f32,
    max_width: Option<f32>,
    align: crate::model::TextAlign,
}

impl TextLayout {
    /// Skia's fake-bold outline width at this size (`kStdFakeBoldInterp*`).
    pub fn embolden_width(&self) -> f32 {
        if !self.synth_bold {
            return 0.0;
        }
        fake_bold_extra(self.size)
    }

    /// `TextLine.Start` for a line under the layout's alignment.
    pub fn line_offset(&self, line: &Line) -> f32 {
        let paragraph = self.max_width.unwrap_or(self.width);
        match self.align {
            crate::model::TextAlign::Left => 0.0,
            crate::model::TextAlign::Center => (paragraph - line.width) / 2.0,
            crate::model::TextAlign::Right => paragraph - line.width_with_trailing,
        }
    }
}

pub fn fake_bold_extra(size: f32) -> f32 {
    let scale = if size <= 9.0 {
        1.0 / 24.0
    } else if size >= 36.0 {
        1.0 / 32.0
    } else {
        let t = (size - 9.0) / 27.0;
        1.0 / 24.0 + t * (1.0 / 32.0 - 1.0 / 24.0)
    };
    size * scale
}

struct Cluster {
    start_x: f32,
    glyphs: Vec<PlacedGlyph>,
    advance: f32,
    whitespace: bool,
    end: usize,
}

struct FaceMetrics {
    ascent: f32,
    height: f32,
}

fn metrics(face: &FaceRef, size: f32) -> FaceMetrics {
    face.with_face(|f| {
        let scale = size / f.units_per_em() as f32;
        let ascent = f.ascender() as f32 * scale;
        let descent = -(f.descender() as f32) * scale;
        let gap = f.line_gap() as f32 * scale;
        FaceMetrics {
            ascent,
            height: ascent + descent + gap,
        }
    })
    .unwrap_or(FaceMetrics {
        ascent: size * 0.8,
        height: size * 1.2,
    })
}

fn covers(face: &FaceRef, c: char) -> bool {
    face.with_face(|f| f.glyph_index(c).is_some())
        .unwrap_or(false)
}

/// Shapes one paragraph (no line breaks inside) into clusters.
fn shape_paragraph(text: &str, font: &ResolvedFont, size: f32) -> Vec<Cluster> {
    if text.is_empty() {
        return Vec::new();
    }
    // Split into runs of characters the same face covers.
    let mut runs: Vec<(usize, usize, usize)> = Vec::new(); // (start, end, face index)
    for (i, c) in text.char_indices() {
        let face_index = if c.is_whitespace() || c.is_control() {
            runs.last().map(|r| r.2).unwrap_or(0)
        } else {
            font.faces
                .iter()
                .position(|face| covers(face, c))
                .unwrap_or(0)
        };
        match runs.last_mut() {
            Some(last) if last.2 == face_index && last.1 == i => last.1 = i + c.len_utf8(),
            _ => runs.push((i, i + c.len_utf8(), face_index)),
        }
    }
    let mut clusters: Vec<Cluster> = Vec::new();
    let mut pen = 0.0f32;
    for (start, end, face_index) in runs {
        let face = &font.faces[face_index];
        let run_text = &text[start..end];
        let shaped = face.with_face(|f| {
            let hb = rustybuzz::Face::from_face(f.clone());
            let mut buffer = rustybuzz::UnicodeBuffer::new();
            buffer.push_str(run_text);
            buffer.guess_segment_properties();
            let output = rustybuzz::shape(&hb, &[], buffer);
            let upem = f.units_per_em() as f32;
            let infos = output.glyph_infos().to_vec();
            let positions = output.glyph_positions().to_vec();
            (upem, infos, positions)
        });
        let Some((upem, infos, positions)) = shaped else {
            continue;
        };
        let scale = size / upem;
        let mut index = 0;
        while index < infos.len() {
            let cluster_start = infos[index].cluster as usize;
            let mut glyphs = Vec::new();
            let mut advance = 0.0;
            while index < infos.len() && infos[index].cluster as usize == cluster_start {
                let pos = positions[index];
                glyphs.push(PlacedGlyph {
                    face: face.clone(),
                    glyph: infos[index].glyph_id as u16,
                    x: pen + advance + pos.x_offset as f32 * scale,
                    y: -(pos.y_offset as f32) * scale,
                    scale,
                });
                advance += pos.x_advance as f32 * scale;
                index += 1;
            }
            let next_start = if index < infos.len() {
                infos[index].cluster as usize
            } else {
                run_text.len()
            };
            let cluster_text = &run_text[cluster_start.min(run_text.len())
                ..next_start.max(cluster_start).min(run_text.len())];
            clusters.push(Cluster {
                start_x: pen,
                glyphs,
                advance,
                whitespace: !cluster_text.is_empty()
                    && cluster_text.chars().all(char::is_whitespace),
                end: start + next_start.max(cluster_start).min(run_text.len()),
            });
            pen += advance;
        }
    }
    clusters
}

/// Lays text out the way `FormattedText` does with `MaxTextWidth` (word wrapping,
/// breaking words that do not fit) and a `TextAlignment`.
pub fn layout(
    text: &str,
    request: &FontRequest,
    size: f32,
    max_width: Option<f32>,
    align: crate::model::TextAlign,
) -> TextLayout {
    let font = resolve(request);
    let primary = metrics(&font.faces[0], size);
    let mut lines = Vec::new();
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    for paragraph in normalized.split('\n') {
        let clusters = shape_paragraph(paragraph, &font, size);
        if clusters.is_empty() {
            lines.push(Line {
                ascent: primary.ascent,
                height: primary.height,
                ..Line::default()
            });
            continue;
        }
        let breaks: Vec<usize> = unicode_linebreak::linebreaks(paragraph)
            .filter(|(_, kind)| *kind == unicode_linebreak::BreakOpportunity::Allowed)
            .map(|(i, _)| i)
            .collect();
        let mut line_start = 0usize;
        while line_start < clusters.len() {
            let origin = clusters[line_start].start_x;
            let mut width = 0.0f32;
            let mut last_break: Option<usize> = None; // index of the first cluster after a break
            let mut end = clusters.len();
            for (i, cluster) in clusters.iter().enumerate().skip(line_start) {
                let new_width = width + cluster.advance;
                if let Some(limit) = max_width
                    && !cluster.whitespace
                    && new_width > limit + 0.01
                    && i > line_start
                {
                    end = match last_break {
                        Some(b) if b > line_start => b,
                        _ => i,
                    };
                    break;
                }
                width = new_width;
                if breaks.contains(&cluster.end) {
                    last_break = Some(i + 1);
                }
            }
            let slice = &clusters[line_start..end];
            let mut glyphs = Vec::new();
            for cluster in slice {
                for g in &cluster.glyphs {
                    let mut g = g.clone();
                    g.x -= origin;
                    glyphs.push(g);
                }
            }
            let width_with_trailing: f32 = slice.iter().map(|c| c.advance).sum();
            let trailing: f32 = slice
                .iter()
                .rev()
                .take_while(|c| c.whitespace)
                .map(|c| c.advance)
                .sum();
            lines.push(Line {
                glyphs,
                width: width_with_trailing - trailing,
                width_with_trailing,
                ascent: primary.ascent,
                height: primary.height,
            });
            line_start = end;
        }
    }
    let width = lines.iter().map(|l| l.width).fold(0.0, f32::max);
    let height = lines.iter().map(|l| l.height).sum();
    TextLayout {
        lines,
        width,
        height,
        synth_bold: font.synth_bold,
        synth_italic: font.synth_italic,
        size,
        max_width: max_width.filter(|w| *w > 0.0),
        align,
    }
}

struct Outline<'a> {
    builder: &'a mut PathBuilder,
    ox: f32,
    oy: f32,
    scale: f32,
    skew: f32,
}

impl Outline<'_> {
    fn map(&self, x: f32, y: f32) -> (f32, f32) {
        let fx = x * self.scale;
        let fy = y * self.scale;
        (self.ox + fx + self.skew * fy, self.oy - fy)
    }
}

impl ttf_parser::OutlineBuilder for Outline<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.map(x, y);
        self.builder.move_to(x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.map(x, y);
        self.builder.line_to(x, y);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let (x1, y1) = self.map(x1, y1);
        let (x, y) = self.map(x, y);
        self.builder.quad_to(x1, y1, x, y);
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (x1, y1) = self.map(x1, y1);
        let (x2, y2) = self.map(x2, y2);
        let (x, y) = self.map(x, y);
        self.builder.cubic_to(x1, y1, x2, y2, x, y);
    }
    fn close(&mut self) {
        self.builder.close();
    }
}

/// `FormattedText.BuildGeometry(origin)`: every glyph outline, one path.
pub fn build_geometry(layout: &TextLayout, origin_x: f32, origin_y: f32) -> Option<Path> {
    let mut builder = PathBuilder::new();
    // Avalonia's oblique simulation skews by 0.2 (`SKFont.SkewX = -0.2`).
    let skew = if layout.synth_italic { 0.2 } else { 0.0 };
    let mut top = origin_y;
    for line in &layout.lines {
        let x0 = origin_x + layout.line_offset(line);
        let baseline = top + line.ascent;
        for g in &line.glyphs {
            let mut outline = Outline {
                builder: &mut builder,
                ox: x0 + g.x,
                oy: baseline + g.y,
                scale: g.scale,
                skew,
            };
            g.face.with_face(|f| {
                f.outline_glyph(ttf_parser::GlyphId(g.glyph), &mut outline);
            });
        }
        top += line.height;
    }
    builder.finish()
}

/// `SKFont(SKTypeface.Default, size).MeasureText(text)` and `Descent - Ascent`, the
/// measurement `NumberAnnotation.CalculateRadius` makes.
pub fn measure_default_line(text: &str, size: f32, bold: bool) -> (f32, f32) {
    let request = FontRequest {
        family: DEFAULT_FAMILY.into(),
        bold,
        italic: false,
    };
    let laid = layout(text, &request, size, None, crate::model::TextAlign::Left);
    let height = laid.lines.first().map(|l| l.height).unwrap_or(size);
    let width = laid
        .lines
        .first()
        .map(|l| l.width_with_trailing)
        .unwrap_or(0.0);
    (width, height)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::TextAlign;

    fn inter() -> FontRequest {
        FontRequest {
            family: "Segoe UI".into(),
            bold: false,
            italic: false,
        }
    }

    #[test]
    fn missing_family_falls_back_to_inter() {
        set_system_fonts_enabled(false);
        let laid = layout("Hello", &inter(), 48.0, None, TextAlign::Left);
        assert_eq!(laid.lines.len(), 1);
        // Inter's line height is about 1.21 em.
        assert!((laid.height / 48.0 - 1.21).abs() < 0.02, "{}", laid.height);
        assert!(laid.width > 100.0 && laid.width < 140.0, "{}", laid.width);
    }

    #[test]
    fn wraps_at_word_boundaries_and_breaks_long_words() {
        set_system_fonts_enabled(false);
        let laid = layout("one two three", &inter(), 20.0, Some(70.0), TextAlign::Left);
        assert!(laid.lines.len() >= 2, "{}", laid.lines.len());
        assert!(laid.lines.iter().all(|l| l.width <= 70.5));
        let long = layout(
            "abcdefghijklmnop",
            &inter(),
            20.0,
            Some(50.0),
            TextAlign::Left,
        );
        assert!(long.lines.len() >= 3);
    }

    #[test]
    fn newline_starts_a_line_and_polish_letters_shape() {
        set_system_fonts_enabled(false);
        let laid = layout("Zażółć\ngęślą", &inter(), 20.0, None, TextAlign::Center);
        assert_eq!(laid.lines.len(), 2);
        assert!(
            laid.lines
                .iter()
                .all(|l| l.glyphs.iter().all(|g| g.glyph != 0))
        );
        assert!(build_geometry(&laid, 0.0, 0.0).is_some());
    }
}
