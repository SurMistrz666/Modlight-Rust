//! `MainViewModel.BackgroundState` and the canvas part of `CanvasState`: the Background
//! panel's settings, the smart padding cache, the gradient presets.

use std::sync::Arc;

use image::RgbaImage;

use crate::color::Color;
use crate::frame::{BackgroundBrush, BackgroundMode, BackgroundState, ImageLayout};
use crate::options::ImageEditorOptions;

/// A host-provided wallpaper lookup (`IDesktopWallpaperService`).
pub trait WallpaperService: Send + Sync {
    /// `TryGetDesktopWallpaper`: the image path and its layout.
    fn wallpaper(&self) -> Option<(String, ImageLayout)>;
}

static WALLPAPER: std::sync::OnceLock<std::sync::RwLock<Option<Box<dyn WallpaperService>>>> =
    std::sync::OnceLock::new();

fn wallpaper_slot() -> &'static std::sync::RwLock<Option<Box<dyn WallpaperService>>> {
    WALLPAPER.get_or_init(|| std::sync::RwLock::new(None))
}

/// `EditorServices.DesktopWallpaper`: the Wallpaper background mode exists only when a
/// service is installed.
pub fn set_wallpaper_service(service: Option<Box<dyn WallpaperService>>) {
    *wallpaper_slot().write().unwrap() = service;
}

pub fn wallpaper_supported() -> bool {
    wallpaper_slot().read().unwrap().is_some()
}

fn current_wallpaper() -> Option<(String, ImageLayout)> {
    wallpaper_slot().read().unwrap().as_ref()?.wallpaper()
}

/// A host-provided picture reader for files the `image` crate cannot decode (a HEIC
/// desktop picture on a Mac).
pub type ImageLoader = Box<dyn Fn(&std::path::Path) -> Option<RgbaImage> + Send + Sync>;

static LOADER: std::sync::OnceLock<std::sync::RwLock<Option<ImageLoader>>> =
    std::sync::OnceLock::new();

pub fn set_image_loader(loader: Option<ImageLoader>) {
    *LOADER
        .get_or_init(|| std::sync::RwLock::new(None))
        .write()
        .unwrap() = loader;
}

/// Opens a picture: the `image` crate first, then the host's reader.
pub fn open_image(path: &std::path::Path) -> Option<RgbaImage> {
    if let Ok(image) = image::open(path) {
        return Some(image.to_rgba8());
    }
    LOADER
        .get()
        .and_then(|l| l.read().ok())
        .and_then(|l| l.as_ref().and_then(|f| f(path)))
}

/// The ratio buttons (`SetOutputRatioCommand` parameters).
pub const OUTPUT_RATIOS: [&str; 9] = [
    "Auto", "1:1", "4:3", "3:2", "16:9", "5:3", "9:16", "3:4", "2:3",
];

/// `GradientPresets`.
pub const GRADIENT_PRESETS: [(&str, &str, &str); 60] = [
    ("Sunset Glow", "#FF7E5F", "#FEB47B"),
    ("Ocean Breeze", "#2193B0", "#6DD5ED"),
    ("Skyline", "#00C6FF", "#0072FF"),
    ("Peach Bloom", "#ED4264", "#FFEDBC"),
    ("Mint Fresh", "#11998E", "#38EF7D"),
    ("Royal Blue", "#396AFC", "#2948FF"),
    ("Warm Flame", "#FF9A9E", "#FAD0C4"),
    ("Coral Candy", "#FF9966", "#FF5E62"),
    ("Mango", "#FFE259", "#FFA751"),
    ("Violet Dream", "#4776E6", "#8E54E9"),
    ("Ruby Fire", "#DD2476", "#FF512F"),
    ("Aqua Marine", "#1A2980", "#26D0CE"),
    ("Instagram", "#833AB4", "#FD1D1D"),
    ("Aurora Green", "#00F260", "#0575E6"),
    ("Citrus Pop", "#F7971E", "#FFD200"),
    ("Arctic Sky", "#74EBD5", "#9FACE6"),
    ("Cherry Rush", "#EB3349", "#F45C43"),
    ("Lavender Sky", "#E0C3FC", "#8EC5FC"),
    ("Midnight City", "#232526", "#414345"),
    ("Deep Space", "#000428", "#004E92"),
    ("Lagoon", "#43C6AC", "#191654"),
    ("Rose Gold", "#EECDA3", "#EF629F"),
    ("Emerald Water", "#348F50", "#56B4D3"),
    ("Lemon Lime", "#56AB2F", "#A8E063"),
    ("Pink Haze", "#EC008C", "#FC6767"),
    ("Slate", "#BDC3C7", "#2C3E50"),
    ("Carbon Steel", "#485563", "#29323C"),
    ("Purple Bliss", "#8E2DE2", "#4A00E0"),
    ("Ember", "#CB2D3E", "#EF473A"),
    ("Soft Lilac", "#C471F5", "#FA71CD"),
    ("Blue Hour", "#355C7D", "#6C5B7B"),
    ("Golden Hour", "#F3904F", "#FFD194"),
    ("Watermelon", "#FF5F6D", "#FFC371"),
    ("Sea Foam", "#4CB8C4", "#3CD3AD"),
    ("Twilight Plum", "#41295A", "#2F0743"),
    ("Candy Grape", "#B24592", "#F15F79"),
    ("Fresh Mint", "#76B852", "#8DC26F"),
    ("Polar Night", "#283048", "#859398"),
    ("Tropical Sun", "#F6D365", "#FDA085"),
    ("Blue Raspberry", "#00B4DB", "#0083B0"),
    ("Rosewater", "#E55D87", "#5FC3E4"),
    ("Copper Glow", "#B79891", "#94716B"),
    ("Aurora Violet", "#654EA3", "#EAAFC8"),
    ("Apple Mint", "#5A3F37", "#2C7744"),
    ("Sunrise Peach", "#F857A6", "#FF5858"),
    ("Iceberg", "#C9FFBF", "#FFAFBD"),
    ("Nightfall", "#141E30", "#243B55"),
    ("Electric Cyan", "#0575E6", "#00F260"),
    ("Velvet Rose", "#DA4453", "#89216B"),
    ("Desert Sand", "#BA8B02", "#181818"),
    ("Cosmic Fusion", "#FF00CC", "#333399"),
    ("Deep Ocean", "#360033", "#0B8793"),
    ("Northern Lights", "#43E97B", "#38F9D7"),
    ("Neon Glow", "#4FACFE", "#00F2FE"),
    ("Burning Orange", "#FF416C", "#FF4B2B"),
    ("Mystic Purple", "#9D50BB", "#6E48AA"),
    ("Golden Sunset", "#F12711", "#F5AF19"),
    ("Midnight Dark", "#2C3E50", "#3498DB"),
    ("Cool Sky", "#2980B9", "#6DD5FA"),
    ("Purple Love", "#CC2B5E", "#753A88"),
];

/// The Background panel's values.
#[derive(Clone, Debug)]
pub struct BackgroundSettings {
    pub margin: f64,
    pub padding: f64,
    pub smart_padding: bool,
    pub rounded_corner: f64,
    pub shadow_radius: f64,
    pub selected_ratio: String,
    pub mode: BackgroundMode,
    pub color: String,
    pub gradient1: String,
    pub gradient2: String,
    pub image_path: Option<String>,
    image: Option<(String, ImageLayout, Arc<RgbaImage>)>,
    insets: (u32, u32, u32, u32),
    insets_valid: bool,
}

impl Default for BackgroundSettings {
    fn default() -> Self {
        BackgroundSettings {
            margin: 30.0,
            padding: 30.0,
            smart_padding: true,
            rounded_corner: 15.0,
            shadow_radius: 30.0,
            selected_ratio: "Auto".into(),
            mode: BackgroundMode::Transparent,
            color: "#FFFFFFFF".into(),
            gradient1: "#FFFF7E5F".into(),
            gradient2: "#FFFEB47B".into(),
            image_path: None,
            image: None,
            insets: (0, 0, 0, 0),
            insets_valid: false,
        }
    }
}

fn normalize(hex: &str, fallback: Color) -> String {
    Color::parse(hex).unwrap_or(fallback).to_hex_argb()
}

impl BackgroundSettings {
    /// `InitializeBackgroundSettingsFromOptions`.
    pub fn from_options(o: &ImageEditorOptions) -> BackgroundSettings {
        let mut mode = BackgroundMode::parse(&o.background_type);
        if mode == BackgroundMode::Wallpaper && !wallpaper_supported() {
            mode = BackgroundMode::Gradient;
        }
        BackgroundSettings {
            margin: o.background_margin.max(0.0),
            padding: o.background_padding.max(0.0),
            smart_padding: o.background_smart_padding,
            rounded_corner: o.background_rounded_corner.max(0.0),
            shadow_radius: o.background_shadow_radius.max(0.0),
            gradient1: normalize(&o.background_gradient_color1_hex, Color::rgb(255, 126, 95)),
            gradient2: normalize(&o.background_gradient_color2_hex, Color::rgb(254, 180, 123)),
            color: normalize(&o.background_color_hex, Color::rgb(34, 34, 34)),
            image_path: Some(o.background_image_path.clone()).filter(|p| !p.trim().is_empty()),
            mode,
            ..BackgroundSettings::default()
        }
    }

    /// Writes every value back (`Options.Background*`).
    pub fn write_options(&self, o: &mut ImageEditorOptions) {
        o.background_margin = self.margin;
        o.background_padding = self.padding;
        o.background_smart_padding = self.smart_padding;
        o.background_rounded_corner = self.rounded_corner;
        o.background_shadow_radius = self.shadow_radius;
        o.background_gradient_color1_hex = self.gradient1.clone();
        o.background_gradient_color2_hex = self.gradient2.clone();
        o.background_color_hex = self.color.clone();
        o.background_image_path = self.image_path.clone().unwrap_or_default();
        o.background_type = self.mode.name().to_string();
    }

    /// The modes the combo box offers (Wallpaper only with a service).
    pub fn modes() -> Vec<BackgroundMode> {
        let mut modes = vec![
            BackgroundMode::Gradient,
            BackgroundMode::Color,
            BackgroundMode::Transparent,
            BackgroundMode::Image,
        ];
        if wallpaper_supported() {
            modes.push(BackgroundMode::Wallpaper);
        }
        modes
    }

    pub fn invalidate_smart_padding(&mut self) {
        self.insets = (0, 0, 0, 0);
        self.insets_valid = false;
    }

    /// `EnsureSmartPaddingCache` (only while the background is shown).
    pub fn ensure_smart_padding(&mut self, image: &RgbaImage, active: bool) {
        if !active || self.insets_valid {
            return;
        }
        self.insets = crate::frame::detect_smart_insets(image);
        self.insets_valid = true;
    }

    pub fn has_detected_smart_padding(&self) -> bool {
        let (l, t, r, b) = self.insets;
        l > 0 || t > 0 || r > 0 || b > 0
    }

    /// `CanUseBackgroundSmartPadding`.
    pub fn can_use_smart_padding(&self, has_image: bool) -> bool {
        has_image && (!self.insets_valid || self.has_detected_smart_padding())
    }

    /// `DisableBackgroundSmartPaddingIfUnavailable`: an image without a border turns the
    /// option off for good.
    pub fn disable_smart_padding_if_unavailable(&mut self, o: &mut ImageEditorOptions) {
        if self.insets_valid && !self.has_detected_smart_padding() && self.smart_padding {
            self.smart_padding = false;
            o.background_smart_padding = false;
        }
    }

    /// `ParseAspectRatio`.
    pub fn output_ratio(&self) -> Option<f64> {
        if self.selected_ratio.eq_ignore_ascii_case("Auto") {
            return None;
        }
        let (w, h) = self.selected_ratio.split_once(':')?;
        let w: f64 = w.trim().parse().ok()?;
        let h: f64 = h.trim().parse().ok()?;
        (w > 0.0 && h > 0.0).then_some(w / h)
    }

    fn brush(&mut self) -> BackgroundBrush {
        match self.mode {
            BackgroundMode::Transparent => BackgroundBrush::Transparent,
            BackgroundMode::Color => {
                BackgroundBrush::Solid(Color::parse_or_transparent(&self.color))
            }
            BackgroundMode::Gradient => BackgroundBrush::Gradient(
                Color::parse_or_transparent(&self.gradient1),
                Color::parse_or_transparent(&self.gradient2),
            ),
            BackgroundMode::Image => match self.image_path.clone() {
                Some(path) => self.load_brush(&path, ImageLayout::Fill),
                None => BackgroundBrush::Transparent,
            },
            BackgroundMode::Wallpaper => match current_wallpaper() {
                Some((path, layout)) => self.load_brush(&path, layout),
                None => BackgroundBrush::Transparent,
            },
        }
    }

    fn load_brush(&mut self, path: &str, layout: ImageLayout) -> BackgroundBrush {
        if let Some((p, l, image)) = &self.image
            && p == path
            && *l == layout
        {
            return BackgroundBrush::Image(image.clone(), layout);
        }
        match open_image(std::path::Path::new(path)) {
            Some(img) => {
                let image = Arc::new(img);
                self.image = Some((path.to_string(), layout, image.clone()));
                BackgroundBrush::Image(image, layout)
            }
            None => BackgroundBrush::Transparent,
        }
    }

    /// What the frame needs to draw (`UpdateCanvasProperties` and friends).
    pub fn state(&self, active: bool, o: &ImageEditorOptions) -> BackgroundState {
        let mut this = self.clone();
        BackgroundState {
            active,
            margin: self.margin,
            padding: self.padding,
            smart_padding: self.smart_padding,
            rounded_corner: self.rounded_corner,
            shadow_radius: self.shadow_radius,
            output_ratio: self.output_ratio(),
            brush: this.brush(),
            shadow_color: o.shadow_color(),
            smart_insets: self.insets,
        }
    }

    /// Preloads an image background so drawing does not decode it every frame.
    pub fn prepare(&mut self) {
        let _ = self.brush();
    }
}

impl super::Editor {
    /// Changes the Background panel's values: the options follow, the frame redraws.
    pub fn update_background(&mut self, change: impl FnOnce(&mut BackgroundSettings)) {
        change(&mut self.background);
        self.background.write_options(&mut self.options);
        self.background.prepare();
        self.invalidate();
    }

    /// `ApplyGradientPresetCommand`.
    pub fn apply_gradient_preset(&mut self, index: usize) {
        if let Some((_, a, b)) = GRADIENT_PRESETS.get(index) {
            let (a, b) = (normalize(a, Color::WHITE), normalize(b, Color::WHITE));
            self.update_background(|s| {
                s.gradient1 = a;
                s.gradient2 = b;
                s.mode = BackgroundMode::Gradient;
            });
        }
    }

    /// The host picked a background image (`OnBrowseBackgroundImageClick`).
    pub fn set_background_image(&mut self, path: &str) {
        let path = path.to_string();
        self.update_background(|s| {
            s.image_path = Some(path);
            s.mode = BackgroundMode::Image;
        });
    }
}
