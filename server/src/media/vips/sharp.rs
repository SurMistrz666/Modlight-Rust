//! The part of sharp 0.34.5 the server uses, on the same libvips calls.
//!
//! [`Sharp`] collects options the way sharp's JavaScript does (the
//! `PipelineBaton`), and [`Sharp::to_buffer_blocking`] replays
//! `src/pipeline.cc` step by step for that subset: same load options, same
//! shrink-on-load rules, same colour management, same premultiply, resize,
//! orientation, crop and encoder arguments, in the same order. A step sharp
//! performs for options this port does not offer is left out, not
//! approximated; the list of what is missing is in the module docs of
//! `media::vips`.
//!
//! Where libvips is concerned the two are byte for byte the same program;
//! where they can differ is the libvips build underneath (see "Loaders" in
//! `media::vips`).

use bytes::Bytes;

use super::image::{Angle, Blob, Direction, Image, band_format, interpretation};
use super::operation::{Arg, Operation};
use super::{VipsError, run_blocking};

const META_N_PAGES: &str = "n-pages";
const META_PAGE_HEIGHT: &str = "page-height";
const META_ORIENTATION: &str = "orientation";
const META_ICC_NAME: &str = "icc-profile-data";
const META_EXIF_NAME: &str = "exif-data";
const META_SEQUENTIAL: &str = "vips-sequential";

// ---------------------------------------------------------------------------
// Input
// ---------------------------------------------------------------------------

/// sharp's `ImageType`, restricted to what the production libvips (the
/// sharp-libvips 1.2.4 build) can load from a buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageType {
    Jpeg,
    Png,
    Webp,
    Tiff,
    Gif,
    Svg,
    Heif,
    Raw,
}

impl ImageType {
    /// `ImageTypeId`: the `format` sharp reports.
    pub fn id(self) -> &'static str {
        match self {
            ImageType::Jpeg => "jpeg",
            ImageType::Png => "png",
            ImageType::Webp => "webp",
            ImageType::Tiff => "tiff",
            ImageType::Gif => "gif",
            ImageType::Svg => "svg",
            ImageType::Heif => "heif",
            ImageType::Raw => "raw",
        }
    }

    /// `ImageTypeSupportsPage`.
    fn supports_page(self) -> bool {
        matches!(self, ImageType::Webp | ImageType::Gif | ImageType::Tiff | ImageType::Heif)
    }
}

/// The buffer loaders the production image has, in libvips' own detection
/// order (priority: png and webp 200, gif/jpeg/tiff 50, heif 0, svg -5), with
/// the type sharp's `loaderToType` table gives them. Everything else this
/// libvips may know (ImageMagick, PDF, JPEG XL, Ultra HDR...) is refused the
/// way sharp-libvips refuses it: "unsupported image format".
pub const BUFFER_LOADERS: [(&str, ImageType); 7] = [
    ("pngload_buffer", ImageType::Png),
    ("webpload_buffer", ImageType::Webp),
    ("gifload_buffer", ImageType::Gif),
    ("jpegload_buffer", ImageType::Jpeg),
    ("tiffload_buffer", ImageType::Tiff),
    ("heifload_buffer", ImageType::Heif),
    ("svgload_buffer", ImageType::Svg),
];

/// `DetermineImageType(buffer)`, over [`BUFFER_LOADERS`] only.
pub fn determine_image_type(buf: &[u8]) -> Option<(ImageType, &'static str)> {
    if buf.is_empty() || super::init().is_err() {
        return None;
    }
    BUFFER_LOADERS.iter().find_map(|&(loader, ty)| {
        let name = super::operation::cstring(loader).ok()?;
        // SAFETY: libvips only reads `buf` for the duration of the call.
        let yes = unsafe { super::ffi::vips_foreign_is_a_buffer(name.as_ptr(), buf.as_ptr().cast(), buf.len()) };
        (yes != 0).then_some((ty, loader))
    })
}

/// `failOn`: how sensitive to invalid data the decoder is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FailOn {
    None,
    Truncated,
    Error,
    /// sharp's default.
    #[default]
    Warning,
}

impl FailOn {
    fn nick(self) -> &'static str {
        match self {
            FailOn::None => "none",
            FailOn::Truncated => "truncated",
            FailOn::Error => "error",
            FailOn::Warning => "warning",
        }
    }
}

/// Whether HEVC-compressed HEIF (`.heic`) pixels may be decoded.
///
/// sharp-libvips ships libheif with the AV1 codecs only, so production reads a
/// HEIC's header (dimensions, page count) and fails as soon as a pipeline
/// needs its pixels: no blurhash, no thumbnail, no avatar. Homebrew's and
/// Debian's libheif usually do carry an HEVC decoder, which would quietly give
/// this server previews production never had. Off, for parity; turning it on
/// is a product decision (and an HEVC licensing one), not a port detail.
pub const HEVC_DECODE: bool = false;

/// The error production's libheif gives for those pixels.
const HEVC_UNSUPPORTED: &str =
    "heif: Error while loading plugin: Support for this compression format has not been built in (11.6003)";

/// sharp's default `limitInputPixels`: `0x3FFF ** 2`.
pub const DEFAULT_LIMIT_INPUT_PIXELS: u64 = 0x3FFF * 0x3FFF;

#[derive(Clone)]
enum Source {
    Buffer(Bytes),
    Raw {
        data: Bytes,
        width: u32,
        height: u32,
        channels: u32,
        page_height: u32,
    },
    /// `sharp([a, b, ...], { join: { across, animated } })`. Every item is its
    /// own input descriptor built with NO options (sharp maps the array
    /// through `_createInputDescriptor(i)`), so each is opened with the
    /// defaults (`failOn: 'warning'`, the default pixel limit, one page).
    Join {
        items: Vec<Bytes>,
        across: i32,
        animated: bool,
    },
    /// `sharp({ text: { ... } })`.
    Text(TextInput),
}

/// `sharp({ text: { text, font, fontfile, rgba } })` (the options the server
/// uses; the rest keep sharp's defaults: align left, no justify, spacing 0,
/// wrap word, dpi 72).
#[derive(Debug, Clone, Default)]
pub struct TextInput {
    /// Pango markup.
    pub text: String,
    pub font: Option<String>,
    pub fontfile: Option<String>,
    pub rgba: bool,
}

/// One `.composite([{ input, left, top }])` layer, blended `over` the image
/// at that offset. The input is a compressed buffer opened with default
/// options, as sharp opens it.
#[derive(Debug, Clone)]
pub struct CompositeLayer {
    pub input: Bytes,
    pub left: i32,
    pub top: i32,
}

/// `VIPS_BLEND_MODE_OVER`, sharp's default `blend`.
const BLEND_MODE_OVER: i32 = 2;

/// sharp's `InputDescriptor` (the options this port supports).
#[derive(Clone)]
struct InputDescriptor {
    source: Source,
    fail_on: FailOn,
    /// 0 = no limit (`limitInputPixels: false`).
    limit_input_pixels: u64,
    /// `n`: 1 by default, -1 for every page (`animated: true`).
    pages: i32,
    page: i32,
    auto_orient: bool,
    density: f64,
    ignore_icc: bool,
}

impl InputDescriptor {
    /// `_createInputDescriptor(input)` with no options: what sharp builds for
    /// each joined image and each composite layer.
    fn defaults(source: Source) -> InputDescriptor {
        InputDescriptor {
            source,
            fail_on: FailOn::Warning,
            limit_input_pixels: DEFAULT_LIMIT_INPUT_PIXELS,
            pages: 1,
            page: 0,
            auto_orient: false,
            density: 72.0,
            ignore_icc: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Resize, extract and output options
// ---------------------------------------------------------------------------

/// `fit`. `contain` (an embed on a background) is not ported: no upstream
/// server code uses it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Fit {
    #[default]
    Cover,
    Fill,
    Inside,
    Outside,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Canvas {
    Crop,
    Max,
    Min,
    IgnoreAspect,
}

impl Fit {
    /// `mapFitToCanvas`.
    fn canvas(self) -> Canvas {
        match self {
            Fit::Cover => Canvas::Crop,
            Fit::Fill => Canvas::IgnoreAspect,
            Fit::Inside => Canvas::Max,
            Fit::Outside => Canvas::Min,
        }
    }
}

/// `position`: a gravity, or one of the two smart-crop strategies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Position {
    #[default]
    Centre,
    North,
    East,
    South,
    West,
    Northeast,
    Southeast,
    Southwest,
    Northwest,
    Entropy,
    Attention,
}

impl Position {
    /// The integer sharp hands to C++ (`gravity` 0-8, `strategy` 16/17).
    fn code(self) -> i32 {
        match self {
            Position::Centre => 0,
            Position::North => 1,
            Position::East => 2,
            Position::South => 3,
            Position::West => 4,
            Position::Northeast => 5,
            Position::Southeast => 6,
            Position::Southwest => 7,
            Position::Northwest => 8,
            Position::Entropy => 16,
            Position::Attention => 17,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Kernel {
    Nearest,
    Linear,
    Cubic,
    Mitchell,
    Lanczos2,
    #[default]
    Lanczos3,
    Mks2013,
    Mks2021,
}

impl Kernel {
    fn nick(self) -> &'static str {
        match self {
            Kernel::Nearest => "nearest",
            Kernel::Linear => "linear",
            Kernel::Cubic => "cubic",
            Kernel::Mitchell => "mitchell",
            Kernel::Lanczos2 => "lanczos2",
            Kernel::Lanczos3 => "lanczos3",
            Kernel::Mks2013 => "mks2013",
            Kernel::Mks2021 => "mks2021",
        }
    }
}

/// `.resize({ ... })`.
#[derive(Debug, Clone, Copy)]
pub struct Resize {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fit: Fit,
    pub position: Position,
    pub kernel: Kernel,
    pub without_enlargement: bool,
    pub without_reduction: bool,
    pub fast_shrink_on_load: bool,
}

impl Default for Resize {
    fn default() -> Self {
        Resize {
            width: None,
            height: None,
            fit: Fit::Cover,
            position: Position::Centre,
            kernel: Kernel::Lanczos3,
            without_enlargement: false,
            without_reduction: false,
            fast_shrink_on_load: true,
        }
    }
}

impl Resize {
    /// `.resize(width, height)` with the other options at sharp's defaults.
    pub fn to(width: Option<u32>, height: Option<u32>) -> Resize {
        Resize { width, height, ..Resize::default() }
    }

    pub fn fit(mut self, fit: Fit) -> Resize {
        self.fit = fit;
        self
    }

    pub fn position(mut self, position: Position) -> Resize {
        self.position = position;
        self
    }

    pub fn without_enlargement(mut self, yes: bool) -> Resize {
        self.without_enlargement = yes;
        self
    }
}

/// `.extract({ left, top, width, height })`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    pub left: u32,
    pub top: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChromaSubsampling {
    /// `'4:2:0'`.
    #[default]
    Yuv420,
    /// `'4:4:4'`.
    Yuv444,
}

impl ChromaSubsampling {
    fn subsample_mode(self) -> &'static str {
        match self {
            ChromaSubsampling::Yuv444 => "off",
            ChromaSubsampling::Yuv420 => "on",
        }
    }
}

/// `.jpeg({ ... })`.
#[derive(Debug, Clone)]
pub struct JpegOptions {
    pub quality: u32,
    pub progressive: bool,
    pub chroma_subsampling: ChromaSubsampling,
    pub optimise_coding: bool,
    /// Shorthand for trellis + overshoot deringing + optimised scans +
    /// progressive + quantisation table 3 (mozjpeg's defaults).
    pub mozjpeg: bool,
    pub trellis_quantisation: bool,
    pub overshoot_deringing: bool,
    pub optimise_scans: bool,
    pub quantisation_table: u32,
}

impl Default for JpegOptions {
    fn default() -> Self {
        JpegOptions {
            quality: 80,
            progressive: false,
            chroma_subsampling: ChromaSubsampling::Yuv420,
            optimise_coding: true,
            mozjpeg: false,
            trellis_quantisation: false,
            overshoot_deringing: false,
            optimise_scans: false,
            quantisation_table: 0,
        }
    }
}

/// `.png({ ... })`. The palette-only options are `Option`s because sharp
/// switches to palette output when any of them is given.
#[derive(Debug, Clone)]
pub struct PngOptions {
    pub progressive: bool,
    pub compression_level: u32,
    pub adaptive_filtering: bool,
    pub palette: Option<bool>,
    pub quality: Option<u32>,
    pub effort: Option<u32>,
    pub colours: Option<u32>,
    pub dither: Option<f64>,
}

impl Default for PngOptions {
    fn default() -> Self {
        PngOptions {
            progressive: false,
            compression_level: 6,
            adaptive_filtering: false,
            palette: None,
            quality: None,
            effort: None,
            colours: None,
            dither: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WebpPreset {
    #[default]
    Default,
    Photo,
    Picture,
    Drawing,
    Icon,
    Text,
}

impl WebpPreset {
    fn nick(self) -> &'static str {
        match self {
            WebpPreset::Default => "default",
            WebpPreset::Photo => "photo",
            WebpPreset::Picture => "picture",
            WebpPreset::Drawing => "drawing",
            WebpPreset::Icon => "icon",
            WebpPreset::Text => "text",
        }
    }
}

/// `.webp({ ... })`, including the animation options (`loop`, `delay`).
#[derive(Debug, Clone)]
pub struct WebpOptions {
    pub quality: u32,
    pub alpha_quality: u32,
    pub lossless: bool,
    pub near_lossless: bool,
    pub smart_subsample: bool,
    pub smart_deblock: bool,
    pub preset: WebpPreset,
    pub effort: u32,
    pub min_size: bool,
    pub mixed: bool,
    pub loop_count: Option<u32>,
    /// One value (applied to every frame) or one per frame, in milliseconds.
    pub delay: Option<Vec<u32>>,
}

impl Default for WebpOptions {
    fn default() -> Self {
        WebpOptions {
            quality: 80,
            alpha_quality: 100,
            lossless: false,
            near_lossless: false,
            smart_subsample: false,
            smart_deblock: false,
            preset: WebpPreset::Default,
            effort: 4,
            min_size: false,
            mixed: false,
            loop_count: None,
            delay: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HeifCompression {
    #[default]
    Av1,
    Hevc,
}

/// `.heif({ ... })`; `.avif()` is this with AV1 compression.
#[derive(Debug, Clone)]
pub struct HeifOptions {
    pub quality: u32,
    pub lossless: bool,
    pub compression: HeifCompression,
    pub effort: u32,
    pub chroma_subsampling: ChromaSubsampling,
    pub bitdepth: u32,
}

impl Default for HeifOptions {
    fn default() -> Self {
        HeifOptions {
            quality: 50,
            lossless: false,
            compression: HeifCompression::Av1,
            effort: 4,
            chroma_subsampling: ChromaSubsampling::Yuv444,
            bitdepth: 8,
        }
    }
}

/// `.gif({ ... })`, including the animation options.
#[derive(Debug, Clone)]
pub struct GifOptions {
    pub reuse: bool,
    pub progressive: bool,
    pub colours: Option<u32>,
    pub effort: u32,
    pub dither: f64,
    pub inter_frame_max_error: f64,
    pub inter_palette_max_error: f64,
    pub keep_duplicate_frames: bool,
    pub loop_count: Option<u32>,
    pub delay: Option<Vec<u32>>,
}

impl Default for GifOptions {
    fn default() -> Self {
        GifOptions {
            reuse: true,
            progressive: false,
            colours: None,
            effort: 7,
            dither: 1.0,
            inter_frame_max_error: 0.0,
            inter_palette_max_error: 3.0,
            keep_duplicate_frames: false,
            loop_count: None,
            delay: None,
        }
    }
}

#[derive(Debug, Clone)]
enum FormatOut {
    /// sharp's default: the input's own format.
    Input,
    Jpeg(JpegOptions),
    Png(PngOptions),
    Webp(WebpOptions),
    Gif(GifOptions),
    Heif(HeifOptions),
    Raw,
}

/// The `info` object of `toBuffer({ resolveWithObject: true })`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputInfo {
    pub format: &'static str,
    pub width: u32,
    pub height: u32,
    pub channels: u32,
    /// Set for raw output: always `"uchar"` here.
    pub depth: Option<&'static str>,
    pub premultiplied: bool,
    pub size: usize,
    /// Present when the output carries more than one page.
    pub page_height: Option<i32>,
    pub pages: Option<i32>,
}

/// What `sharp(...).metadata()` reports (the fields server code reads, plus
/// the cheap neighbours).
#[derive(Debug, Clone, PartialEq)]
pub struct Metadata {
    pub format: &'static str,
    pub size: Option<usize>,
    pub width: u32,
    pub height: u32,
    pub space: String,
    pub channels: u32,
    pub depth: String,
    pub density: Option<u32>,
    pub pages: Option<u32>,
    pub page_height: Option<u32>,
    pub loop_count: Option<i32>,
    pub delay: Option<Vec<i32>>,
    /// `heif-compression` (`"av1"`, `"hevc"`).
    pub compression: Option<String>,
    pub has_profile: bool,
    pub has_alpha: bool,
    /// EXIF orientation, when the file has one (1-8).
    pub orientation: Option<u32>,
    pub exif: Option<Vec<u8>>,
}

// ---------------------------------------------------------------------------
// The builder
// ---------------------------------------------------------------------------

/// One sharp pipeline: `sharp(input, options)` plus chained calls.
///
/// Every method takes and returns `self`; nothing touches libvips until
/// [`Sharp::to_buffer`] / [`Sharp::metadata`] (or their `_blocking`
/// forms). Invalid parameters are reported then, with sharp's wording.
#[derive(Clone)]
pub struct Sharp {
    input: InputDescriptor,
    resize: Option<Resize>,
    extract_pre: Option<Region>,
    extract_post: Option<Region>,
    orient_before: bool,
    remove_alpha: bool,
    ensure_alpha: Option<f64>,
    timeout_seconds: u32,
    loop_count: i32,
    delay: Vec<i32>,
    format_out: FormatOut,
    flatten: Option<[f64; 3]>,
    composite: Vec<CompositeLayer>,
    invalid: Option<String>,
}

/// `sharp(buffer)`.
pub fn sharp(input: Bytes) -> Sharp {
    Sharp::new(input)
}

impl Sharp {
    /// `sharp(buffer)`: compressed image data (JPEG, PNG, WebP, GIF, TIFF,
    /// AVIF/HEIF, SVG).
    pub fn new(input: Bytes) -> Sharp {
        Sharp::with_source(Source::Buffer(input))
    }

    /// `sharp(pixels, { raw: { width, height, channels } })`: 8-bit pixels,
    /// row major, interleaved.
    pub fn raw(data: Bytes, width: u32, height: u32, channels: u32) -> Sharp {
        Sharp::with_source(Source::Raw { data, width, height, channels, page_height: 0 })
    }

    /// `sharp([buffers], { join: { across, animated } })`: the images stacked
    /// into one, `across` per row; `animated` makes the stack the pages of an
    /// animation (page height = joined height / count).
    pub fn join(items: Vec<Bytes>, across: u32, animated: bool) -> Sharp {
        let count = items.len();
        let s = Sharp::with_source(Source::Join { items, across: across as i32, animated });
        if count < 2 {
            return s.invalid("Expected at least two images to join".into());
        }
        if !(1..=1_000_000).contains(&across) {
            return s.invalid(format!(
                "Expected integer between 1 and 100000 for join.across but received {across} of type number"
            ));
        }
        s
    }

    /// `sharp({ text: { ... } })`: an image made of Pango markup.
    pub fn text(text: TextInput) -> Sharp {
        if text.text.is_empty() {
            return Sharp::with_source(Source::Text(text))
                .invalid("Expected a valid string to create an image with text.".into());
        }
        Sharp::with_source(Source::Text(text))
    }

    fn with_source(source: Source) -> Sharp {
        Sharp {
            input: InputDescriptor::defaults(source),
            resize: None,
            extract_pre: None,
            extract_post: None,
            orient_before: false,
            remove_alpha: false,
            ensure_alpha: None,
            timeout_seconds: 0,
            loop_count: -1,
            delay: Vec::new(),
            format_out: FormatOut::Input,
            flatten: None,
            composite: Vec::new(),
            invalid: None,
        }
    }

    fn invalid(mut self, message: String) -> Sharp {
        self.invalid.get_or_insert(message);
        self
    }

    // --- input options ---------------------------------------------------------------

    /// `{ failOn }`.
    pub fn fail_on(mut self, fail_on: FailOn) -> Sharp {
        self.input.fail_on = fail_on;
        self
    }

    /// `{ limitInputPixels }`: `None` is `false` (no limit).
    pub fn limit_input_pixels(mut self, limit: Option<u64>) -> Sharp {
        self.input.limit_input_pixels = limit.unwrap_or(0);
        self
    }

    /// `{ animated }`: every page (`true`) or the first one (`false`).
    pub fn animated(mut self, animated: bool) -> Sharp {
        self.input.pages = if animated { -1 } else { 1 };
        self
    }

    /// `{ pages }`: how many pages to load, -1 for all.
    pub fn pages(mut self, pages: i32) -> Sharp {
        if !(-1..=100_000).contains(&pages) {
            return self.invalid(format!("Expected integer between -1 and 100000 for pages but received {pages}"));
        }
        self.input.pages = pages;
        self
    }

    /// `{ page }`: the first page to load.
    pub fn page(mut self, page: u32) -> Sharp {
        if page > 100_000 {
            return self.invalid(format!("Expected integer between 0 and 100000 for page but received {page}"));
        }
        self.input.page = page as i32;
        self
    }

    /// `{ raw: { pageHeight } }` for raw input: the frames are stacked.
    pub fn raw_page_height(mut self, page_height: u32) -> Sharp {
        if let Source::Raw { page_height: ph, .. } = &mut self.input.source {
            *ph = page_height;
        }
        self
    }

    /// `.rotate()` without an angle: orient by EXIF, then drop the tag.
    pub fn rotate(self) -> Sharp {
        self.auto_orient()
    }

    /// `.autoOrient()`.
    pub fn auto_orient(mut self) -> Sharp {
        self.input.auto_orient = true;
        self
    }

    // --- operations -----------------------------------------------------------------------

    /// `.resize({ ... })`.
    pub fn resize(mut self, resize: Resize) -> Sharp {
        if resize.width == Some(0) {
            return self.invalid("Expected positive integer for width but received 0 of type number".into());
        }
        if resize.height == Some(0) {
            return self.invalid("Expected positive integer for height but received 0 of type number".into());
        }
        self.resize = Some(resize);
        self
    }

    /// `.extract(region)`: before the resize when no resize has been asked
    /// for yet, after it otherwise (sharp decides by call order too).
    pub fn extract(mut self, region: Region) -> Sharp {
        // isResizeExpected(...) || widthPre !== -1
        let resize_expected = self.resize.is_some_and(|r| r.width.is_some() || r.height.is_some());
        if resize_expected || self.extract_pre.is_some() {
            self.extract_post = Some(region);
        } else {
            self.extract_pre = Some(region);
        }
        if self.input.auto_orient {
            self.orient_before = true;
        }
        self
    }

    /// `.flatten({ background })`: merge the alpha channel onto a solid colour
    /// (`[r, g, b]`, 0-255; `#ffffff` is `[255.0, 255.0, 255.0]`).
    pub fn flatten(mut self, background: [f64; 3]) -> Sharp {
        self.flatten = Some(background);
        self
    }

    /// `.composite([{ input, left, top }, ...])`: layers blended `over` the
    /// image, each at its offset.
    pub fn composite(mut self, layers: Vec<CompositeLayer>) -> Sharp {
        self.composite = layers;
        self
    }

    /// `.ensureAlpha(alpha)` (`1` is sharp's default argument).
    pub fn ensure_alpha(mut self, alpha: f64) -> Sharp {
        if !(0.0..=1.0).contains(&alpha) {
            return self
                .invalid(format!("Expected number between 0 and 1 for alpha but received {alpha} of type number"));
        }
        self.ensure_alpha = Some(alpha);
        self
    }

    /// `.removeAlpha()`.
    pub fn remove_alpha(mut self) -> Sharp {
        self.remove_alpha = true;
        self
    }

    /// `.timeout({ seconds })`.
    pub fn timeout(mut self, seconds: u32) -> Sharp {
        if seconds > 3600 {
            return self.invalid(format!(
                "Expected integer between 0 and 3600 for seconds but received {seconds} of type number"
            ));
        }
        self.timeout_seconds = seconds;
        self
    }

    // --- output formats ----------------------------------------------------------------------

    pub fn jpeg(mut self, o: JpegOptions) -> Sharp {
        if !(1..=100).contains(&o.quality) {
            return self.invalid(range_error("quality", "integer between 1 and 100", o.quality));
        }
        if o.quantisation_table > 8 {
            return self.invalid(range_error("quantisationTable", "integer between 0 and 8", o.quantisation_table));
        }
        self.format_out = FormatOut::Jpeg(o);
        self
    }

    pub fn png(mut self, o: PngOptions) -> Sharp {
        if o.compression_level > 9 {
            return self.invalid(range_error("compressionLevel", "integer between 0 and 9", o.compression_level));
        }
        if let Some(c) = o.colours
            && !(2..=256).contains(&c)
        {
            return self.invalid(range_error("colours", "integer between 2 and 256", c));
        }
        if let Some(q) = o.quality
            && q > 100
        {
            return self.invalid(range_error("quality", "integer between 0 and 100", q));
        }
        if let Some(e) = o.effort
            && !(1..=10).contains(&e)
        {
            return self.invalid(range_error("effort", "integer between 1 and 10", e));
        }
        if let Some(d) = o.dither
            && !(0.0..=1.0).contains(&d)
        {
            return self
                .invalid(format!("Expected number between 0.0 and 1.0 for dither but received {d} of type number"));
        }
        self.format_out = FormatOut::Png(o);
        self
    }

    pub fn webp(mut self, o: WebpOptions) -> Sharp {
        if !(1..=100).contains(&o.quality) {
            return self.invalid(range_error("quality", "integer between 1 and 100", o.quality));
        }
        if o.alpha_quality > 100 {
            return self.invalid(range_error("alphaQuality", "integer between 0 and 100", o.alpha_quality));
        }
        if o.effort > 6 {
            return self.invalid(range_error("effort", "integer between 0 and 6", o.effort));
        }
        if let Err(message) = self.set_animation_options(o.loop_count, o.delay.as_deref()) {
            return self.invalid(message);
        }
        self.format_out = FormatOut::Webp(o);
        self
    }

    pub fn gif(mut self, o: GifOptions) -> Sharp {
        if let Some(c) = o.colours
            && !(2..=256).contains(&c)
        {
            return self.invalid(range_error("colours", "integer between 2 and 256", c));
        }
        if !(1..=10).contains(&o.effort) {
            return self.invalid(range_error("effort", "integer between 1 and 10", o.effort));
        }
        if !(0.0..=1.0).contains(&o.dither) {
            return self.invalid(format!(
                "Expected number between 0.0 and 1.0 for dither but received {} of type number",
                o.dither
            ));
        }
        if !(0.0..=32.0).contains(&o.inter_frame_max_error) {
            return self.invalid(format!(
                "Expected number between 0.0 and 32.0 for interFrameMaxError but received {} of type number",
                o.inter_frame_max_error
            ));
        }
        if !(0.0..=256.0).contains(&o.inter_palette_max_error) {
            return self.invalid(format!(
                "Expected number between 0.0 and 256.0 for interPaletteMaxError but received {} of type number",
                o.inter_palette_max_error
            ));
        }
        if let Err(message) = self.set_animation_options(o.loop_count, o.delay.as_deref()) {
            return self.invalid(message);
        }
        self.format_out = FormatOut::Gif(o);
        self
    }

    pub fn heif(mut self, o: HeifOptions) -> Sharp {
        if !(1..=100).contains(&o.quality) {
            return self.invalid(range_error("quality", "integer between 1 and 100", o.quality));
        }
        if o.effort > 9 {
            return self.invalid(range_error("effort", "integer between 0 and 9", o.effort));
        }
        // The production binaries are sharp's prebuilt ones, which refuse
        // anything but 8 bits.
        if o.bitdepth != 8 {
            return self.invalid(range_error("bitdepth when using prebuilt binaries", "8", o.bitdepth));
        }
        self.format_out = FormatOut::Heif(o);
        self
    }

    /// `.avif({ ... })` = `.heif({ ...options, compression: 'av1' })`.
    pub fn avif(self, o: HeifOptions) -> Sharp {
        self.heif(HeifOptions { compression: HeifCompression::Av1, ..o })
    }

    /// `.raw()`: uncompressed 8-bit pixels.
    pub fn raw_output(mut self) -> Sharp {
        self.format_out = FormatOut::Raw;
        self
    }

    /// `trySetAnimationOptions`.
    fn set_animation_options(&mut self, loop_count: Option<u32>, delay: Option<&[u32]>) -> Result<(), String> {
        if let Some(l) = loop_count {
            if l > 65535 {
                return Err(range_error("loop", "integer between 0 and 65535", l));
            }
            self.loop_count = l as i32;
        }
        if let Some(d) = delay {
            if d.iter().any(|&v| v > 65535) {
                return Err("Expected integer or an array of integers between 0 and 65535 for delay".into());
            }
            self.delay = d.iter().map(|&v| v as i32).collect();
        }
        Ok(())
    }

    // --- terminal calls ------------------------------------------------------------------------

    /// `toBuffer()`, on tokio's blocking pool.
    pub async fn to_buffer(self) -> Result<Vec<u8>, VipsError> {
        run_blocking(move || self.to_buffer_blocking()).await
    }

    /// `toBuffer({ resolveWithObject: true })`, on tokio's blocking pool.
    pub async fn to_buffer_with_info(self) -> Result<(Vec<u8>, OutputInfo), VipsError> {
        run_blocking(move || self.to_buffer_with_info_blocking()).await
    }

    /// `metadata()`, on tokio's blocking pool.
    pub async fn metadata(self) -> Result<Metadata, VipsError> {
        run_blocking(move || self.metadata_blocking()).await
    }

    pub fn to_buffer_blocking(self) -> Result<Vec<u8>, VipsError> {
        self.to_buffer_with_info_blocking().map(|(data, _)| data)
    }

    pub fn to_buffer_with_info_blocking(self) -> Result<(Vec<u8>, OutputInfo), VipsError> {
        if let Some(message) = self.invalid.clone() {
            return Err(VipsError::new(message));
        }
        self.pipeline()
    }

    pub fn metadata_blocking(self) -> Result<Metadata, VipsError> {
        if let Some(message) = self.invalid.clone() {
            return Err(VipsError::new(message));
        }
        metadata(&self.input)
    }

    // --- pipeline.cc ----------------------------------------------------------------------------

    fn pipeline(self) -> Result<(Vec<u8>, OutputInfo), VipsError> {
        let opened = open_input(&self.input)?;
        let input_image_type = opened.image_type;
        let mut image = opened.image;

        // See HEVC_DECODE: production fails here, when the pixels are needed.
        if !HEVC_DECODE
            && input_image_type == ImageType::Heif
            && image.get_string("heif-compression").as_deref() == Some("hevc")
        {
            return Err(VipsError::new(HEVC_UNSUPPORTED));
        }

        let mut n_pages = self.input.pages;
        if n_pages == -1 {
            // Resolve the number of pages if we need to render until the end of the document
            n_pages = match image.get_int_any(META_N_PAGES) {
                Some(n) => n - self.input.page,
                None => 1,
            };
        }

        // Get pre-resize page height
        let mut page_height = image.page_height();

        // Calculate angle of rotation
        let (mut auto_rotation, mut auto_flop) = if self.input.auto_orient {
            calculate_exif_rotation_and_flop(exif_orientation(&image))
        } else {
            (Angle::D0, false)
        };

        // rotate(angle), flip() and flop() are not ported, so `rotateBefore`
        // can never be set; only an extract can ask for orientation first.
        let should_orient_before = self.orient_before && (auto_rotation != Angle::D0 || auto_flop);

        if should_orient_before {
            image = stay_sequential(image, auto_rotation != Angle::D0)?;
            if auto_rotation != Angle::D0 {
                if auto_rotation != Angle::D180 {
                    multi_page_unsupported(n_pages, "Rotate")?;
                }
                image = image.rot(auto_rotation)?;
                auto_rotation = Angle::D0;
            }
            if auto_flop {
                image = image.flip(Direction::Horizontal)?;
                auto_flop = false;
            }
        }

        // Pre extraction
        if let Some(r) = self.extract_pre {
            image = if n_pages > 1 {
                crop_multi_page(
                    image,
                    r.left as i32,
                    r.top as i32,
                    r.width as i32,
                    r.height as i32,
                    n_pages,
                    &mut page_height,
                )?
            } else {
                image.extract_area(r.left as i32, r.top as i32, r.width as i32, r.height as i32)?
            };
        }

        // Get pre-resize image width and height
        let mut input_width = image.width();
        let mut input_height = image.height();

        // Is there just one page? Shrink to inputHeight instead
        if n_pages == 1 {
            page_height = input_height;
        }

        let resize = self.resize.unwrap_or_default();
        let canvas = resize.fit.canvas();
        let requested_width = resize.width.map(|w| w as i32).unwrap_or(-1);
        let requested_height = resize.height.map(|h| h as i32).unwrap_or(-1);

        // Scaling calculations
        let mut target_resize_width = requested_width;
        let mut target_resize_height = requested_height;

        // When auto-rotating by 90 or 270 degrees, swap the target width and
        // height to ensure the behavior aligns with how it would have been if
        // the rotation had taken place *before* resizing.
        if auto_rotation == Angle::D90 || auto_rotation == Angle::D270 {
            std::mem::swap(&mut target_resize_width, &mut target_resize_height);
        }

        // Shrink to pageHeight, so we work for multi-page images
        let (hshrink, vshrink) = resolve_shrink(
            input_width,
            page_height,
            target_resize_width,
            target_resize_height,
            canvas,
            resize.without_enlargement,
            resize.without_reduction,
        );

        // The jpeg preload shrink.
        let mut jpeg_shrink_on_load = 1;
        // WebP, PDF, SVG scale
        let mut scale = 1.0f64;

        // Try to reload input using shrink-on-load for JPEG, WebP, SVG and PDF, when:
        //  - the width or height parameters are specified;
        //  - gamma correction doesn't need to be applied;
        //  - trimming or pre-resize extract isn't required;
        //  - input colourspace is not specified;
        let should_pre_shrink = (target_resize_width > 0 || target_resize_height > 0)
            && self.extract_pre.is_none()
            && !should_orient_before;

        if should_pre_shrink {
            // The common part of the shrink: the bit by which both axes must be shrunk
            let shrink = hshrink.min(vshrink);
            match input_image_type {
                ImageType::Jpeg => {
                    // Leave at least a factor of two for the final resize step, when fastShrinkOnLoad: false
                    // for more consistent results and to avoid extra sharpness to the image
                    let factor = if resize.fast_shrink_on_load { 1.0 } else { 2.0 };
                    if shrink >= 8.0 * factor {
                        jpeg_shrink_on_load = 8;
                    } else if shrink >= 4.0 * factor {
                        jpeg_shrink_on_load = 4;
                    } else if shrink >= 2.0 * factor {
                        jpeg_shrink_on_load = 2;
                    }
                    // Lower shrink-on-load for known libjpeg rounding errors
                    if jpeg_shrink_on_load > 1 && shrink as i32 == jpeg_shrink_on_load {
                        jpeg_shrink_on_load /= 2;
                    }
                }
                ImageType::Webp if resize.fast_shrink_on_load && shrink > 1.0 => {
                    // Avoid upscaling via webp
                    scale = 1.0 / shrink;
                }
                ImageType::Svg => {
                    scale = 1.0 / shrink;
                }
                _ => {}
            }
        }

        // Reload input using shrink-on-load, it'll be an integer shrink
        // factor for jpegload*, a double scale factor for webpload*,
        // pdfload* and svgload*
        if jpeg_shrink_on_load > 1 {
            image = reload(&opened.blob, &self.input, input_image_type, |op| op.set("shrink", jpeg_shrink_on_load))?;
        } else if scale != 1.0 {
            match input_image_type {
                ImageType::Webp => {
                    image = reload(&opened.blob, &self.input, input_image_type, |op| op.set("scale", scale))?;
                }
                ImageType::Svg => {
                    image = reload(&opened.blob, &self.input, input_image_type, |op| op.set("scale", scale))?;
                    image = set_density(&image, self.input.density)?;
                    if image.width() > 32767 || image.height() > 32767 {
                        return Err(VipsError::new("Input SVG image will exceed 32767x32767 pixel limit when scaled"));
                    }
                }
                _ => {}
            }
        } else if input_image_type == ImageType::Svg && (image.width() > 32767 || image.height() > 32767) {
            return Err(VipsError::new("Input SVG image exceeds 32767x32767 pixel limit"));
        }
        if self.input.auto_orient {
            image = remove_exif_orientation(&image)?;
        }

        // Any pre-shrinking may already have been done
        input_width = image.width();
        input_height = image.height();

        // After pre-shrink, but before the main shrink stage
        // Reuse the initial pageHeight if we didn't pre-shrink
        if should_pre_shrink {
            page_height = image.page_height();
        }

        // Shrink to pageHeight, so we work for multi-page images
        let (hshrink, mut vshrink) = resolve_shrink(
            input_width,
            page_height,
            target_resize_width,
            target_resize_height,
            canvas,
            resize.without_enlargement,
            resize.without_reduction,
        );

        let mut target_height = (page_height as f64 / vshrink).round_ties_even() as i32;
        let mut target_page_height = target_height;

        // In toilet-roll mode, we must adjust vshrink so that we exactly hit
        // pageHeight or we'll have pixels straddling pixel boundaries
        if input_height > page_height {
            target_height *= n_pages;
            vshrink = input_height as f64 / target_height as f64;
        }

        // Ensure we're using a device-independent colour space
        let processing_profile = if image.interpretation() == interpretation::RGB16 { "p3" } else { "srgb" };
        let interp = image.interpretation();
        if image.has_blob(META_ICC_NAME)
            && interp != interpretation::LABS
            && interp != interpretation::GREY16
            && !self.input.ignore_icc
        {
            // Convert to sRGB/P3 using embedded profile
            match image.icc_transform_embedded(processing_profile, if is_16bit(interp) { 16 } else { 8 }) {
                Ok(converted) => image = converted,
                Err(_) => tracing::debug!(target: "vips", "Invalid embedded profile"),
            }
        } else if interp == interpretation::CMYK {
            image = image.icc_transform_from(processing_profile, "cmyk")?;
        }

        // Flatten image to remove alpha channel
        if let Some(bg) = self.flatten
            && image.has_alpha()
        {
            let multiplier = if is_16bit(image.interpretation()) { 256.0 } else { 1.0 };
            image = image.flatten(&[bg[0] * multiplier, bg[1] * multiplier, bg[2] * multiplier])?;
        }

        let should_resize = hshrink != 1.0 || vshrink != 1.0;
        let should_composite = !self.composite.is_empty();

        if should_composite && !image.has_alpha() {
            image = ensure_alpha(image, 1.0)?;
        }

        let premultiply_format = image.format();
        let should_premultiply_alpha = image.has_alpha() && should_resize;

        if should_premultiply_alpha {
            image = image.premultiply()?.cast(premultiply_format)?;
        }

        // Resize
        if should_resize {
            image = image.resize(1.0 / hshrink, 1.0 / vshrink, resize.kernel.nick())?;
        }

        image = stay_sequential(image, auto_rotation != Angle::D0)?;
        // Auto-rotate post-extract
        if auto_rotation != Angle::D0 {
            if auto_rotation != Angle::D180 {
                multi_page_unsupported(n_pages, "Rotate")?;
            }
            image = image.rot(auto_rotation)?;
        }
        // Mirror horizontally (left-right) about the y-axis
        if auto_flop {
            image = image.flip(Direction::Horizontal)?;
        }

        input_width = image.width();
        input_height = if n_pages > 1 { target_page_height } else { image.height() };

        // Resolve dimensions
        let mut width = if requested_width <= 0 { input_width } else { requested_width };
        let mut height = if requested_height <= 0 { input_height } else { requested_height };

        // Crop/embed
        if (input_width != width || input_height != height) && canvas == Canvas::Crop {
            width = width.min(input_width);
            height = height.min(input_height);
            let position = resize.position.code();
            if position < 9 {
                // Gravity-based crop
                let (left, top) = calculate_crop(input_width, input_height, width, height, position);
                image = if n_pages > 1 {
                    crop_multi_page(image, left, top, width, height, n_pages, &mut target_page_height)?
                } else {
                    image.extract_area(left, top, width, height)?
                };
            } else {
                // Attention-based or Entropy-based crop
                multi_page_unsupported(n_pages, "Resize strategy")?;
                image = stay_sequential(image, true)?;
                let interesting = if position == 16 { "entropy" } else { "attention" };
                image = image.smartcrop(width, height, interesting, should_premultiply_alpha)?;
            }
        }

        // Post extraction
        let mut post_size = None;
        if let Some(r) = self.extract_post {
            let (l, t, w, h) = (r.left as i32, r.top as i32, r.width as i32, r.height as i32);
            if n_pages > 1 {
                image = crop_multi_page(image, l, t, w, h, n_pages, &mut target_page_height)?;
                // heightPost is used in the info object, so update to reflect the number of pages
                post_size = Some((w, h * n_pages));
            } else {
                image = image.extract_area(l, t, w, h)?;
                post_size = Some((w, h));
            }
        }

        // Reverse premultiplication after all transformations
        if should_premultiply_alpha {
            image = image.unpremultiply()?.cast(premultiply_format)?;
        }

        // Composite
        if should_composite {
            let mut images = vec![image.clone()];
            let (mut modes, mut xs, mut ys) = (Vec::new(), Vec::new(), Vec::new());
            for layer in &self.composite {
                let mut composite_image =
                    open_input(&InputDescriptor::defaults(Source::Buffer(layer.input.clone())))?.image;
                // Verify within current dimensions
                if composite_image.width() > image.width() || composite_image.height() > image.height() {
                    return Err(VipsError::new("Image to composite must have same dimensions or smaller"));
                }
                // Ensure image to composite is with unpremultiplied alpha
                composite_image = ensure_alpha(composite_image, 1.0)?;
                images.push(composite_image);
                modes.push(BLEND_MODE_OVER);
                xs.push(layer.left);
                ys.push(layer.top);
            }
            image = Image::composite(&images, &modes, &xs, &ys, "srgb")?;
            // RemoveGifPalette
            image = image.mutate(|m| m.remove("gif-palette"))?;
        }

        // Remove alpha channel, if any
        if self.remove_alpha {
            image = remove_alpha(image)?;
        }

        // Ensure alpha channel, if missing
        if let Some(alpha) = self.ensure_alpha {
            image = ensure_alpha(image, alpha)?;
        }

        // Ensure output colour space
        if is_16bit(image.interpretation()) {
            image = image.cast(band_format::USHORT)?;
        }
        if image.interpretation() != interpretation::SRGB {
            let source = image.interpretation();
            image = image.colourspace(interpretation::SRGB, source)?;
        }

        // Number of channels used in output image
        let mut channels = image.bands();
        let out_width = image.width();
        let out_height = image.height();

        image = set_animation_properties(&image, n_pages, target_page_height, &self.delay, self.loop_count)?;

        let (page_height_out, pages_out) = match image.get_int(META_PAGE_HEIGHT) {
            Some(ph) => (Some(ph), image.get_int(META_N_PAGES)),
            None => (None, None),
        };

        // Output
        image.set_timeout(self.timeout_seconds);

        let format_out = match &self.format_out {
            FormatOut::Input => match input_image_type {
                ImageType::Jpeg => FormatOut::Jpeg(JpegOptions::default()),
                ImageType::Png | ImageType::Svg => FormatOut::Png(PngOptions::default()),
                ImageType::Webp => FormatOut::Webp(WebpOptions::default()),
                ImageType::Gif => FormatOut::Gif(GifOptions::default()),
                ImageType::Heif => FormatOut::Heif(HeifOptions::default()),
                ImageType::Raw => FormatOut::Raw,
                ImageType::Tiff => {
                    return Err(VipsError::new(format!(
                        "Unsupported output format when trying to match input format of {}",
                        input_image_type.id()
                    )));
                }
            },
            other => other.clone(),
        };

        let mut depth = None;
        let (data, format) = match format_out {
            FormatOut::Jpeg(o) => {
                assert_image_type_dimensions(&image, ImageType::Jpeg)?;
                let data = image.save_buffer(jpeg_save(&o)?)?;
                channels = channels.min(3);
                (data, "jpeg")
            }
            FormatOut::Png(o) => {
                assert_image_type_dimensions(&image, ImageType::Png)?;
                let bitdepth = if is_16bit(image.interpretation()) { 16 } else { png_bitdepth(&o) };
                (image.save_buffer(png_save(&o, bitdepth)?)?, "png")
            }
            FormatOut::Webp(o) => {
                assert_image_type_dimensions(&image, ImageType::Webp)?;
                (image.save_buffer(webp_save(&o)?)?, "webp")
            }
            FormatOut::Gif(o) => {
                assert_image_type_dimensions(&image, ImageType::Gif)?;
                (image.save_buffer(gif_save(&o)?)?, "gif")
            }
            FormatOut::Heif(o) => {
                assert_image_type_dimensions(&image, ImageType::Heif)?;
                image = remove_animation_properties(&image)?;
                (image.save_buffer(heif_save(&o)?)?, "heif")
            }
            FormatOut::Raw => {
                // Write raw, uncompressed image data to buffer
                if image.interpretation() == interpretation::B_W {
                    // Extract first band for greyscale image
                    image = image.extract_band(0, None)?;
                    channels = 1;
                }
                if image.format() != band_format::UCHAR {
                    // Cast pixels to requested format
                    image = image.cast(band_format::UCHAR)?;
                }
                depth = Some("uchar");
                (image.write_to_memory()?, "raw")
            }
            FormatOut::Input => unreachable!("resolved above"),
        };

        let (info_width, info_height) = post_size.unwrap_or((out_width, out_height));
        let info = OutputInfo {
            format,
            width: info_width as u32,
            height: info_height as u32,
            channels: channels as u32,
            depth,
            premultiplied: should_premultiply_alpha,
            size: data.len(),
            page_height: page_height_out.filter(|&p| p != 0),
            pages: page_height_out.filter(|&p| p != 0).and(pages_out),
        };
        Ok((data, info))
    }
}

fn range_error(name: &str, expected: &str, actual: u32) -> String {
    format!("Expected {expected} for {name} but received {actual} of type number")
}

// ---------------------------------------------------------------------------
// common.cc
// ---------------------------------------------------------------------------

struct Opened {
    image: Image,
    image_type: ImageType,
    /// The compressed input, kept for a shrink-on-load reload.
    blob: Option<Blob>,
}

/// `GetOptionsForImageType`: what every load of this type is told.
fn load_options(op: Operation, image_type: ImageType, descriptor: &InputDescriptor) -> Operation {
    let mut op = op.set("access", Arg::Nick("sequential")).set("fail_on", Arg::Nick(descriptor.fail_on.nick()));
    if image_type.supports_page() {
        op = op.set("n", descriptor.pages).set("page", descriptor.page);
    }
    match image_type {
        ImageType::Svg => op
            .set("dpi", descriptor.density)
            // `stylesheet` and `high_bitdepth` arrived in libvips 8.17;
            // sharp passes their defaults (none, false).
            .set_if_known("stylesheet", Arg::Str(""))
            .set_if_known("high_bitdepth", false),
        ImageType::Tiff => op.set("subifd", -1),
        _ => op,
    }
}

fn loader_name(image_type: ImageType) -> &'static str {
    BUFFER_LOADERS.iter().find(|(_, t)| *t == image_type).map(|(n, _)| *n).unwrap_or("")
}

/// `OpenInput`.
fn open_input(descriptor: &InputDescriptor) -> Result<Opened, VipsError> {
    let (image, image_type, blob) = match &descriptor.source {
        Source::Raw { data, width, height, channels, page_height } => {
            // Raw, uncompressed pixel data
            let mut image =
                Image::from_memory(data, *width as i32, *height as i32, *channels as i32, band_format::UCHAR)?;
            image = image.copy_with_interpretation(if *channels < 3 { "b-w" } else { "srgb" })?;
            if *page_height > 0 {
                let (ph, pages) = (*page_height as i32, (*height / *page_height) as i32);
                image = image.mutate(|m| {
                    m.set_int(META_PAGE_HEIGHT, ph);
                    m.set_int(META_N_PAGES, pages);
                })?;
            }
            (image, ImageType::Raw, None)
        }
        Source::Join { items, across, animated } => {
            let mut images = Vec::with_capacity(items.len());
            let mut has_alpha = false;
            for item in items {
                let image = open_input(&InputDescriptor::defaults(Source::Buffer(item.clone())))?.image;
                has_alpha |= image.has_alpha();
                images.push(image);
            }
            let mut background = vec![0.0, 0.0, 0.0, 255.0];
            if has_alpha {
                images = images.into_iter().map(|i| ensure_alpha(i, 1.0)).collect::<Result<_, _>>()?;
            } else {
                background.pop();
            }
            let mut image = Image::arrayjoin_with(&images, *across, 0, &background, "low", "low")?;
            if *animated {
                let (pages, height) = (images.len() as i32, image.height());
                image = image.mutate(|m| {
                    m.set_int(META_N_PAGES, pages);
                    m.set_int(META_PAGE_HEIGHT, height / pages);
                })?;
            }
            // The joined image is not checked against this descriptor's pixel
            // limit: upstream never passes it through OpenInput.
            return Ok(Opened { image, image_type: ImageType::Png, blob: None });
        }
        Source::Text(t) => {
            let mut image = Image::text(
                &t.text,
                t.font.as_deref(),
                t.fontfile.as_deref(),
                t.rgba,
                "low",
                false,
                0,
                "word",
                Some(72),
            )?;
            if !t.rgba {
                image = image.copy_with_interpretation("b-w")?;
            }
            (image, ImageType::Raw, None)
        }
        Source::Buffer(bytes) => {
            if bytes.is_empty() {
                return Err(VipsError::new("Input Buffer is empty"));
            }
            // Compressed data
            let Some((image_type, loader)) = determine_image_type(bytes) else {
                return Err(VipsError::new("Input buffer contains unsupported image format"));
            };
            let blob = Blob::from_bytes(bytes)?;
            let op = load_options(Operation::new(loader)?.set("buffer", &blob), image_type, descriptor);
            let mut image = match op.call().and_then(|b| b.image("out")) {
                Ok(image) => image,
                Err(err) => return Err(VipsError::new(format!("Input buffer has corrupt header: {}", err.message()))),
            };
            if image_type == ImageType::Svg {
                image = set_density(&image, descriptor.density)?;
            }
            (image, image_type, Some(blob))
        }
    };

    // Limit input images to a given number of pixels, where pixels = width * height
    if descriptor.limit_input_pixels > 0 && image.width() as u64 * image.height() as u64 > descriptor.limit_input_pixels
    {
        return Err(VipsError::new("Input image exceeds pixel limit"));
    }
    Ok(Opened { image, image_type, blob })
}

/// The shrink-on-load reload of the same buffer with one more option.
fn reload(
    blob: &Option<Blob>,
    descriptor: &InputDescriptor,
    image_type: ImageType,
    extra: impl FnOnce(Operation) -> Operation,
) -> Result<Image, VipsError> {
    let blob = blob.as_ref().ok_or_else(|| VipsError::new("reload without a buffer"))?;
    let op = load_options(Operation::new(loader_name(image_type))?.set("buffer", blob), image_type, descriptor);
    extra(op).call()?.image("out")
}

/// `SetDensity`.
fn set_density(image: &Image, density: f64) -> Result<Image, VipsError> {
    let pixels_per_mm = density / 25.4;
    image.copy_with_resolution(pixels_per_mm, pixels_per_mm)
}

/// `ExifOrientation`.
fn exif_orientation(image: &Image) -> i32 {
    image.get_int_any(META_ORIENTATION).unwrap_or(0)
}

/// `RemoveExifOrientation`.
fn remove_exif_orientation(image: &Image) -> Result<Image, VipsError> {
    image.mutate(|m| {
        m.remove(META_ORIENTATION);
        m.remove("exif-ifd0-Orientation");
    })
}

/// `CalculateExifRotationAndFlop`.
fn calculate_exif_rotation_and_flop(orientation: i32) -> (Angle, bool) {
    match orientation {
        6 => (Angle::D90, false),
        3 => (Angle::D180, false),
        8 => (Angle::D270, false),
        2 => (Angle::D0, true),
        7 => (Angle::D270, true),
        4 => (Angle::D180, true),
        5 => (Angle::D90, true),
        _ => (Angle::D0, false),
    }
}

/// `MultiPageUnsupported`.
fn multi_page_unsupported(pages: i32, op: &str) -> Result<(), VipsError> {
    if pages > 1 { Err(VipsError::new(format!("{op} is not supported for multi-page images"))) } else { Ok(()) }
}

/// `StaySequential`: an operation that needs random access cannot run on a
/// sequentially loaded image, so render it to memory first.
fn stay_sequential(image: Image, condition: bool) -> Result<Image, VipsError> {
    if image.is_sequential() && condition {
        let rendered = image.copy_memory()?;
        return rendered.mutate(|m| m.remove(META_SEQUENTIAL));
    }
    Ok(image)
}

/// `Is16Bit`.
fn is_16bit(interp: i32) -> bool {
    interp == interpretation::RGB16 || interp == interpretation::GREY16
}

/// `ResolveShrink`.
fn resolve_shrink(
    width: i32,
    height: i32,
    target_width: i32,
    target_height: i32,
    canvas: Canvas,
    without_enlargement: bool,
    without_reduction: bool,
) -> (f64, f64) {
    let mut hshrink = 1.0f64;
    let mut vshrink = 1.0f64;

    if target_width > 0 && target_height > 0 {
        // Fixed width and height
        hshrink = width as f64 / target_width as f64;
        vshrink = height as f64 / target_height as f64;

        match canvas {
            Canvas::Crop | Canvas::Min => {
                if hshrink < vshrink {
                    vshrink = hshrink;
                } else {
                    hshrink = vshrink;
                }
            }
            Canvas::Max => {
                if hshrink > vshrink {
                    vshrink = hshrink;
                } else {
                    hshrink = vshrink;
                }
            }
            Canvas::IgnoreAspect => {}
        }
    } else if target_width > 0 {
        // Fixed width
        hshrink = width as f64 / target_width as f64;
        if canvas != Canvas::IgnoreAspect {
            // Auto height
            vshrink = hshrink;
        }
    } else if target_height > 0 {
        // Fixed height
        vshrink = height as f64 / target_height as f64;
        if canvas != Canvas::IgnoreAspect {
            // Auto width
            hshrink = vshrink;
        }
    }

    // We should not reduce or enlarge the output image, if
    // withoutReduction or withoutEnlargement is specified.
    if without_reduction {
        // Equivalent of VIPS_SIZE_UP
        hshrink = hshrink.min(1.0);
        vshrink = vshrink.min(1.0);
    } else if without_enlargement {
        // Equivalent of VIPS_SIZE_DOWN
        hshrink = hshrink.max(1.0);
        vshrink = vshrink.max(1.0);
    }

    // We don't want to shrink so much that we send an axis to 0
    hshrink = hshrink.min(width as f64);
    vshrink = vshrink.min(height as f64);

    (hshrink, vshrink)
}

/// `CalculateCrop` (gravity form).
fn calculate_crop(in_width: i32, in_height: i32, out_width: i32, out_height: i32, gravity: i32) -> (i32, i32) {
    let mut left = 0;
    let mut top = 0;
    match gravity {
        1 => {
            // North
            left = (in_width - out_width + 1) / 2;
        }
        2 => {
            // East
            left = in_width - out_width;
            top = (in_height - out_height + 1) / 2;
        }
        3 => {
            // South
            left = (in_width - out_width + 1) / 2;
            top = in_height - out_height;
        }
        4 => {
            // West
            top = (in_height - out_height + 1) / 2;
        }
        5 => {
            // Northeast
            left = in_width - out_width;
        }
        6 => {
            // Southeast
            left = in_width - out_width;
            top = in_height - out_height;
        }
        7 => {
            // Southwest
            top = in_height - out_height;
        }
        8 => {
            // Northwest
        }
        _ => {
            // Centre
            left = (in_width - out_width + 1) / 2;
            top = (in_height - out_height + 1) / 2;
        }
    }
    (left, top)
}

/// `CropMultiPage` (operations.cc): split, crop each frame, reassemble.
fn crop_multi_page(
    image: Image,
    left: i32,
    top: i32,
    width: i32,
    height: i32,
    n_pages: i32,
    page_height: &mut i32,
) -> Result<Image, VipsError> {
    if top == 0 && height == *page_height {
        // Fast path; no need to adjust the height of the multi-page image
        return image.extract_area(left, 0, width, image.height());
    }
    // Split the image into cropped frames
    let image = stay_sequential(image, true)?;
    let pages = (0..n_pages)
        .map(|i| image.extract_area(left, *page_height * i + top, width, height))
        .collect::<Result<Vec<_>, _>>()?;
    // Reassemble the frames into a tall, thin image
    let assembled = Image::arrayjoin(&pages, 1)?;
    // Update the page height
    *page_height = height;
    Ok(assembled)
}

/// `RemoveAlpha`.
fn remove_alpha(mut image: Image) -> Result<Image, VipsError> {
    while image.bands() > 1 && image.has_alpha() {
        image = image.extract_band(0, Some(image.bands() - 1))?;
    }
    Ok(image)
}

/// `EnsureAlpha`.
fn ensure_alpha(image: Image, value: f64) -> Result<Image, VipsError> {
    if image.has_alpha() {
        return Ok(image);
    }
    // SAFETY: plain function of an enum value.
    let max_alpha = unsafe { super::ffi::vips_interpretation_max_alpha(image.interpretation()) };
    image.bandjoin_const(&[value * max_alpha])
}

/// `SetAnimationProperties`.
fn set_animation_properties(
    image: &Image,
    n_pages: i32,
    page_height: i32,
    delay: &[i32],
    loop_count: i32,
) -> Result<Image, VipsError> {
    let has_delay = !delay.is_empty();
    let mut delay = delay.to_vec();
    let mut loop_count = loop_count;
    image.mutate(|m| {
        // Only set page-height if we have more than one page, or this could
        // accidentally turn into an animated image later.
        if n_pages > 1 {
            m.set_int(META_PAGE_HEIGHT, page_height);
        }
        if has_delay {
            if delay.len() == 1 {
                // We have just one delay, repeat that value for all frames.
                let first = delay[0];
                delay.extend(std::iter::repeat_n(first, (n_pages - 1).max(0) as usize));
            }
            m.set_array_int("delay", &delay);
        }
        if n_pages == 1 && !has_delay && loop_count == -1 {
            loop_count = 1;
        }
        if loop_count != -1 {
            m.set_int("loop", loop_count);
        }
    })
}

/// `RemoveAnimationProperties`.
fn remove_animation_properties(image: &Image) -> Result<Image, VipsError> {
    image.mutate(|m| {
        m.remove(META_PAGE_HEIGHT);
        m.remove("delay");
        m.remove("loop");
    })
}

/// `AssertImageTypeDimensions`.
fn assert_image_type_dimensions(image: &Image, image_type: ImageType) -> Result<(), VipsError> {
    let height = image.get_int(META_PAGE_HEIGHT).unwrap_or_else(|| image.height());
    let width = image.width();
    let (limit, name) = match image_type {
        ImageType::Jpeg => (65535, "JPEG"),
        ImageType::Webp => (16383, "WebP"),
        ImageType::Gif => (65535, "GIF"),
        ImageType::Heif => (16384, "HEIF"),
        _ => return Ok(()),
    };
    if width > limit || height > limit {
        return Err(VipsError::new(format!("Processed image is too large for the {name} format")));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Encoders: the exact argument lists of pipeline.cc's buffer branch
// ---------------------------------------------------------------------------

/// `keep: 0` - sharp strips all metadata unless asked to keep it.
const KEEP_NONE: &str = "none";

fn jpeg_save(o: &JpegOptions) -> Result<Operation, VipsError> {
    let mut progressive = o.progressive;
    let mut trellis = o.trellis_quantisation;
    let mut overshoot = o.overshoot_deringing;
    let mut scans = o.optimise_scans;
    let mut table = o.quantisation_table;
    if o.mozjpeg {
        trellis = true;
        overshoot = true;
        scans = true;
        progressive = true;
        table = 3;
    }
    if scans {
        progressive = true;
    }
    Ok(Operation::new("jpegsave_buffer")?
        .set("keep", Arg::Nick(KEEP_NONE))
        .set("Q", o.quality as i32)
        .set("interlace", progressive)
        .set("subsample_mode", Arg::Nick(o.chroma_subsampling.subsample_mode()))
        .set("trellis_quant", trellis)
        .set("quant_table", table as i32)
        .set("overshoot_deringing", overshoot)
        .set("optimize_scans", scans)
        .set("optimize_coding", o.optimise_coding))
}

/// `bitdepthFromColourCount`: `1 << 31 - Math.clz32(Math.ceil(Math.log2(colours)))`.
fn bitdepth_from_colour_count(colours: u32) -> i32 {
    let bits = (colours as f64).log2().ceil() as u32;
    1 << (31 - bits.leading_zeros())
}

fn png_palette(o: &PngOptions) -> bool {
    o.palette.unwrap_or(o.quality.is_some() || o.effort.is_some() || o.colours.is_some() || o.dither.is_some())
}

fn png_bitdepth(o: &PngOptions) -> i32 {
    o.colours.map(bitdepth_from_colour_count).unwrap_or(8)
}

fn png_save(o: &PngOptions, bitdepth: i32) -> Result<Operation, VipsError> {
    let palette = png_palette(o);
    let (quality, effort, dither) = if palette {
        (o.quality.unwrap_or(100), o.effort.unwrap_or(7), o.dither.unwrap_or(1.0))
    } else {
        (100, 7, 1.0)
    };
    Ok(Operation::new("pngsave_buffer")?
        .set("keep", Arg::Nick(KEEP_NONE))
        .set("interlace", o.progressive)
        .set("compression", o.compression_level as i32)
        .set("filter", Arg::Nick(if o.adaptive_filtering { "all" } else { "none" }))
        .set("palette", palette)
        .set("Q", quality as i32)
        .set("effort", effort as i32)
        .set("bitdepth", bitdepth)
        .set("dither", dither))
}

fn webp_save(o: &WebpOptions) -> Result<Operation, VipsError> {
    Ok(Operation::new("webpsave_buffer")?
        .set("keep", Arg::Nick(KEEP_NONE))
        .set("Q", o.quality as i32)
        .set("lossless", o.lossless)
        .set("near_lossless", o.near_lossless)
        .set("smart_subsample", o.smart_subsample)
        // `smart_deblock` arrived in libvips 8.16; older releases never
        // deblocked smartly, which is what `false` asks for.
        .set_if_known("smart_deblock", o.smart_deblock)
        .set("preset", Arg::Nick(o.preset.nick()))
        .set("effort", o.effort as i32)
        .set("min_size", o.min_size)
        .set("mixed", o.mixed)
        .set("alpha_q", o.alpha_quality as i32))
}

fn gif_save(o: &GifOptions) -> Result<Operation, VipsError> {
    Ok(Operation::new("gifsave_buffer")?
        .set("keep", Arg::Nick(KEEP_NONE))
        .set("bitdepth", o.colours.map(bitdepth_from_colour_count).unwrap_or(8))
        .set("effort", o.effort as i32)
        .set("reuse", o.reuse)
        .set("interlace", o.progressive)
        .set("interframe_maxerror", o.inter_frame_max_error)
        .set("interpalette_maxerror", o.inter_palette_max_error)
        // `keep_duplicate_frames` arrived in libvips 8.17; before it,
        // duplicates were always merged, which is what `false` asks for.
        .set_if_known("keep_duplicate_frames", o.keep_duplicate_frames)
        .set("dither", o.dither))
}

fn heif_save(o: &HeifOptions) -> Result<Operation, VipsError> {
    let compression = match o.compression {
        HeifCompression::Av1 => "av1",
        HeifCompression::Hevc => "hevc",
    };
    Ok(Operation::new("heifsave_buffer")?
        .set("keep", Arg::Nick(KEEP_NONE))
        .set("Q", o.quality as i32)
        .set("compression", Arg::Nick(compression))
        .set("effort", o.effort as i32)
        .set("bitdepth", o.bitdepth as i32)
        .set("subsample_mode", Arg::Nick(o.chroma_subsampling.subsample_mode()))
        .set("lossless", o.lossless))
}

// ---------------------------------------------------------------------------
// metadata.cc
// ---------------------------------------------------------------------------

fn metadata(descriptor: &InputDescriptor) -> Result<Metadata, VipsError> {
    let opened = open_input(descriptor)?;
    let image = &opened.image;
    let density = (image.xres() > 1.0).then(|| (image.xres() * 25.4).round() as u32);
    let orientation = exif_orientation(image);
    Ok(Metadata {
        format: opened.image_type.id(),
        size: match &descriptor.source {
            Source::Buffer(b) => Some(b.len()),
            Source::Raw { .. } | Source::Join { .. } | Source::Text(_) => None,
        },
        width: image.width() as u32,
        height: image.height() as u32,
        space: image.interpretation_nick(),
        channels: image.bands() as u32,
        depth: image.format_nick(),
        density,
        pages: image.get_int(META_N_PAGES).filter(|&p| p > 0).map(|p| p as u32),
        page_height: image.get_int(META_PAGE_HEIGHT).filter(|&p| p > 0).map(|p| p as u32),
        loop_count: image.get_int("loop").filter(|&l| l >= 0),
        delay: image.get_array_int("delay").filter(|d| !d.is_empty()),
        compression: image.get_string("heif-compression").filter(|c| !c.is_empty()),
        has_profile: image.has_blob(META_ICC_NAME),
        has_alpha: image.has_alpha(),
        orientation: (orientation > 0).then_some(orientation as u32),
        exif: image.get_blob(META_EXIF_NAME).filter(|e| !e.is_empty()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bitdepth_from_colour_count_rounds_like_sharp() {
        // 1 << 31 - Math.clz32(Math.ceil(Math.log2(colours)))
        assert_eq!(bitdepth_from_colour_count(256), 8);
        assert_eq!(bitdepth_from_colour_count(255), 8);
        assert_eq!(bitdepth_from_colour_count(16), 4);
        assert_eq!(bitdepth_from_colour_count(8), 2);
        assert_eq!(bitdepth_from_colour_count(5), 2);
        assert_eq!(bitdepth_from_colour_count(4), 2);
        assert_eq!(bitdepth_from_colour_count(3), 2);
        assert_eq!(bitdepth_from_colour_count(2), 1);
    }

    #[test]
    fn resolve_shrink_matches_sharp() {
        // inside: the larger shrink wins
        assert_eq!(resolve_shrink(24, 16, 32, 32, Canvas::Max, false, false), (0.75, 0.75));
        // cover: the smaller shrink wins
        assert_eq!(resolve_shrink(1600, 1200, 256, 256, Canvas::Crop, false, false), (4.6875, 4.6875));
        // withoutEnlargement never goes below 1
        assert_eq!(resolve_shrink(48, 48, 256, 256, Canvas::Crop, true, false), (1.0, 1.0));
        // width only keeps the aspect ratio
        assert_eq!(resolve_shrink(100, 50, 50, -1, Canvas::Crop, false, false), (2.0, 2.0));
        // never shrink an axis to zero
        assert_eq!(resolve_shrink(4, 1000, 1, 1, Canvas::IgnoreAspect, false, false), (4.0, 1000.0));
    }

    #[test]
    fn calculate_crop_matches_sharp() {
        assert_eq!(calculate_crop(10, 10, 4, 4, 0), (3, 3));
        assert_eq!(calculate_crop(11, 11, 4, 4, 0), (4, 4));
        assert_eq!(calculate_crop(10, 10, 4, 4, 6), (6, 6));
        assert_eq!(calculate_crop(10, 10, 4, 4, 8), (0, 0));
    }

    #[test]
    fn exif_orientation_table_matches_sharp() {
        assert_eq!(calculate_exif_rotation_and_flop(6), (Angle::D90, false));
        assert_eq!(calculate_exif_rotation_and_flop(5), (Angle::D90, true));
        assert_eq!(calculate_exif_rotation_and_flop(1), (Angle::D0, false));
        assert_eq!(calculate_exif_rotation_and_flop(0), (Angle::D0, false));
    }
}
