//! The skin as egui sees it: the `ShareX.Color.*` palette as `Color32`s, the fonts
//! (`ShareXTheme.axaml`: Inter and the Lucide icon font; Geist under the Graphite skin)
//! and egui's own style tuned to `ControlStyles.axaml` (menus, tooltips, scroll bars,
//! text fields).

use std::sync::Arc;

use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, Stroke};
use modlight_core::theme::{Palette, ResolvedTheme, Rgba};

/// `ShareX.FontSize.Default`.
pub const FONT_SIZE: f32 = 13.0;

pub const FAMILY_MEDIUM: &str = "inter-medium";
pub const FAMILY_SEMIBOLD: &str = "inter-semibold";
pub const FAMILY_ICON: &str = "lucide";

pub fn medium() -> FontFamily {
    FontFamily::Name(FAMILY_MEDIUM.into())
}

pub fn semibold() -> FontFamily {
    FontFamily::Name(FAMILY_SEMIBOLD.into())
}

pub fn icon_family() -> FontFamily {
    FontFamily::Name(FAMILY_ICON.into())
}

/// A text weight, as the upstream styles name them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Weight {
    Normal,
    Medium,
    SemiBold,
}

impl Weight {
    pub fn family(self) -> FontFamily {
        match self {
            Weight::Normal => FontFamily::Proportional,
            Weight::Medium => medium(),
            Weight::SemiBold => semibold(),
        }
    }
}

pub fn rgba(color: Rgba) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), color.a())
}

/// The skin of the moment, with the colours by the names the styles use.
#[derive(Clone, Debug, PartialEq)]
pub struct Colors {
    pub dark: bool,
    pub palette: Palette,
    /// The Graphite skin: Geist, solid black or white primary buttons, neutral
    /// selections and headings, the accent kept for focus and what is chosen.
    pub graphite: bool,
}

/// The id of the app's own skin, Graphite.
pub const GRAPHITE: &str = "graphite";

macro_rules! color {
    ($($name:ident),* $(,)?) => {
        $(pub fn $name(&self) -> Color32 { rgba(self.palette.$name) })*
    };
}

impl Colors {
    pub fn new(theme: &ResolvedTheme) -> Colors {
        Colors {
            dark: theme.dark,
            palette: theme.palette,
            graphite: theme.skin.id == GRAPHITE,
        }
    }

    color!(
        accent_start,
        accent_end,
        accent_foreground,
        text,
        text_secondary,
        text_on_panel,
        text_secondary_on_panel,
        focus,
        background_main,
        background_panel,
        background_popup,
        background_menu,
        background_toolbar,
        border,
        control_background,
        control_background_hover,
        control_background_disabled,
        control_border,
        separator,
        cancel_background,
        cancel_border,
        continue_background,
        continue_border,
        task_foreground,
        checker_a,
        checker_b,
    );

    /// `ShareX.Brush.Overlay.Modal`.
    pub fn overlay_modal(&self) -> Color32 {
        Color32::from_black_alpha(0x80)
    }

    /// The fill of a primary button: Graphite's white (dark) or near black (light),
    /// otherwise the accent.
    pub fn primary(&self) -> Color32 {
        match (self.graphite, self.dark) {
            (true, true) => Color32::WHITE,
            (true, false) => Color32::from_rgb(0x17, 0x17, 0x17),
            (false, _) => self.accent_start(),
        }
    }

    /// A primary button under the pointer.
    pub fn primary_hover(&self) -> Color32 {
        match (self.graphite, self.dark) {
            (true, true) => Color32::from_rgb(0xE5, 0xE5, 0xE5),
            (true, false) => Color32::from_rgb(0x33, 0x33, 0x33),
            (false, _) => self.accent_end(),
        }
    }

    /// The text and icons on a primary button.
    pub fn primary_foreground(&self) -> Color32 {
        match (self.graphite, self.dark) {
            (true, true) => Color32::BLACK,
            (true, false) => Color32::WHITE,
            (false, _) => self.accent_foreground(),
        }
    }

    /// Titles of windows, dialogs and settings pages: the accent, plain text in
    /// Graphite.
    pub fn heading(&self) -> Color32 {
        if self.graphite {
            self.text()
        } else {
            self.accent_start()
        }
    }

    /// The fill of a chosen row in a list (navigation): the accent, a neutral surface
    /// in Graphite.
    pub fn selected_fill(&self) -> Color32 {
        if self.graphite {
            self.control_background_hover()
        } else {
            self.accent_start()
        }
    }

    /// The text on [`Colors::selected_fill`].
    pub fn selected_text(&self) -> Color32 {
        if self.graphite {
            self.text()
        } else {
            self.accent_foreground()
        }
    }
}

fn static_font(bytes: &'static [u8]) -> Arc<FontData> {
    Arc::new(FontData::from_static(bytes))
}

/// Inter in three weights (Latin, then Latin Extended for the Polish letters), the
/// Lucide icons, and egui's own fonts behind them as a fallback.
pub fn fonts() -> FontDefinitions {
    font_set(false)
}

/// The fonts of the skin in `colors`: Geist (`assets/fonts/Geist-*.ttf`, regular, medium
/// and semibold) under the Graphite skin, with Inter behind it for anything Geist lacks;
/// Inter otherwise. The families keep their names, so every caller draws with either.
pub fn fonts_for(colors: &Colors) -> FontDefinitions {
    font_set(colors.graphite)
}

fn font_set(geist: bool) -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    let faces: [(&str, &'static [u8]); 7] = [
        (
            "inter-regular-latin",
            include_bytes!("../../../assets/fonts/Inter-Regular-latin.ttf"),
        ),
        (
            "inter-regular-ext",
            include_bytes!("../../../assets/fonts/Inter-Regular-latin-ext.ttf"),
        ),
        (
            "inter-medium-latin",
            include_bytes!("../../../assets/fonts/Inter-Medium-latin.ttf"),
        ),
        (
            "inter-medium-ext",
            include_bytes!("../../../assets/fonts/Inter-Medium-latin-ext.ttf"),
        ),
        (
            "inter-semibold-latin",
            include_bytes!("../../../assets/fonts/Inter-SemiBold-latin.ttf"),
        ),
        (
            "inter-semibold-ext",
            include_bytes!("../../../assets/fonts/Inter-SemiBold-latin-ext.ttf"),
        ),
        ("lucide", modlight_core::icons::FONT),
    ];
    for (name, bytes) in faces {
        fonts.font_data.insert(name.to_string(), static_font(bytes));
    }
    if geist {
        let faces: [(&str, &'static [u8]); 3] = [
            (
                "geist-regular",
                include_bytes!("../../../assets/fonts/Geist-Regular.ttf"),
            ),
            (
                "geist-medium",
                include_bytes!("../../../assets/fonts/Geist-Medium.ttf"),
            ),
            (
                "geist-semibold",
                include_bytes!("../../../assets/fonts/Geist-SemiBold.ttf"),
            ),
        ];
        for (name, bytes) in faces {
            fonts.font_data.insert(name.to_string(), static_font(bytes));
        }
    }

    let fallback: Vec<String> = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();

    let chain = |geist_face: &str, first: &str, second: &str| {
        let mut list = Vec::new();
        if geist {
            list.push(geist_face.to_string());
        }
        list.extend([first.to_string(), second.to_string()]);
        list.extend(fallback.iter().cloned());
        list
    };
    fonts.families.insert(
        FontFamily::Proportional,
        chain("geist-regular", "inter-regular-latin", "inter-regular-ext"),
    );
    fonts.families.insert(
        medium(),
        chain("geist-medium", "inter-medium-latin", "inter-medium-ext"),
    );
    fonts.families.insert(
        semibold(),
        chain(
            "geist-semibold",
            "inter-semibold-latin",
            "inter-semibold-ext",
        ),
    );
    fonts
        .families
        .insert(icon_family(), vec!["lucide".to_string()]);
    fonts
}

/// egui's style, made to look like `ControlStyles.axaml` where egui draws for us.
pub fn apply_style(ctx: &egui::Context, colors: &Colors) {
    // The skin's fonts (a no-op when they are already the ones in use).
    ctx.set_fonts(fonts_for(colors));
    let mut visuals = if colors.dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };

    visuals.override_text_color = None;
    visuals.panel_fill = colors.background_main();
    visuals.window_fill = colors.background_popup();
    visuals.window_stroke = Stroke::new(1.0, colors.border());
    visuals.window_corner_radius = CornerRadius::same(10);
    visuals.menu_corner_radius = CornerRadius::same(10);
    visuals.extreme_bg_color = colors.control_background();
    visuals.text_edit_bg_color = Some(colors.control_background());
    visuals.faint_bg_color = colors.background_panel();
    visuals.code_bg_color = colors.control_background();
    visuals.hyperlink_color = colors.accent_start();
    visuals.selection.bg_fill = colors.accent_start().gamma_multiply(0.45);
    visuals.selection.stroke = Stroke::new(1.0, colors.accent_foreground());
    visuals.text_cursor.stroke = Stroke::new(1.5, colors.text());
    visuals.popup_shadow = egui::Shadow {
        offset: [0, 4],
        blur: 16,
        spread: 0,
        color: Color32::from_black_alpha(0x40),
    };
    visuals.window_shadow = visuals.popup_shadow;

    let widget = |fill: Color32, stroke: Color32, fg: Color32| egui::style::WidgetVisuals {
        bg_fill: fill,
        weak_bg_fill: fill,
        bg_stroke: Stroke::new(1.0, stroke),
        corner_radius: CornerRadius::same(6),
        fg_stroke: Stroke::new(1.0, fg),
        expansion: 0.0,
    };
    visuals.widgets.noninteractive = widget(
        colors.background_popup(),
        colors.separator(),
        colors.text_secondary(),
    );
    visuals.widgets.inactive = widget(colors.control_background(), colors.border(), colors.text());
    visuals.widgets.hovered = widget(
        colors.control_background_hover(),
        colors.border(),
        colors.text(),
    );
    visuals.widgets.active = widget(
        colors.control_background_hover(),
        colors.border(),
        colors.text(),
    );
    visuals.widgets.open = widget(
        colors.control_background_hover(),
        colors.border(),
        colors.text(),
    );

    let theme = if colors.dark {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    };
    ctx.set_theme(theme);
    ctx.style_mut_of(theme, |style| style.visuals = visuals);
    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(0.0, 0.0);
        style.spacing.button_padding = egui::vec2(10.0, 6.0);
        style.spacing.menu_margin = egui::Margin::symmetric(6, 4);
        style.spacing.window_margin = egui::Margin::same(8);
        style.spacing.interact_size = egui::vec2(20.0, 20.0);
        style.spacing.scroll = egui::style::ScrollStyle::thin();
        style.spacing.scroll.floating = true;
        style.spacing.tooltip_width = 420.0;
        style.interaction.selectable_labels = false;
        style.interaction.tooltip_delay = 0.4;
        style.text_styles.insert(
            egui::TextStyle::Body,
            egui::FontId::new(FONT_SIZE, FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Button,
            egui::FontId::new(FONT_SIZE, FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Small,
            egui::FontId::new(11.0, FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Monospace,
            egui::FontId::new(12.0, FontFamily::Monospace),
        );
    });
}
