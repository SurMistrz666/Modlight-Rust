//! The details drawer's state (`ModlightDetailsModel.cs`): which file, open or sliding,
//! and the brief "copied" acknowledgements.

use modlight_core::format::{cloud_size, n0};
use modlight_core::icons;
use modlight_core::l10n::{Arg, format, s, t};
use modlight_core::library::{CloudFile, LinkFormat};

/// How long the drawer takes to slide.
pub const SLIDE_SECONDS: f64 = 0.18;
/// How long a copy button shows its tick.
pub const COPIED_SECONDS: f64 = 1.4;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Details {
    /// The file shown, by id.
    pub file: Option<String>,
    /// Drawn at all (true from the moment it opens until it has slid away).
    pub present: bool,
    /// Where the slide is heading.
    pub open: bool,
    /// When the slide started (for the animation).
    pub moved_at: f64,
    /// No animation when the system asks for less motion.
    pub instant: bool,
    pub primary_copied_until: f64,
    /// Which link row was copied, and until when.
    pub row_copied: Option<(LinkFormat, f64)>,
    /// A single click waits a double-click window before opening the drawer.
    pub pending_open: Option<(String, f64)>,
}

impl Details {
    /// Open on `id` (or move to it if already open).
    pub fn open_on(&mut self, id: &str, now: f64, instant: bool) {
        self.file = Some(id.to_string());
        self.primary_copied_until = 0.0;
        self.row_copied = None;
        if self.present && self.open {
            return;
        }
        self.instant = instant;
        self.present = true;
        self.open = true;
        self.moved_at = now;
    }

    pub fn close(&mut self, now: f64) {
        if !self.present {
            return;
        }
        self.open = false;
        self.moved_at = now;
        if self.instant {
            self.retire();
        }
    }

    /// Fully gone: forget the file.
    pub fn retire(&mut self) {
        self.present = false;
        self.open = false;
        self.file = None;
    }

    /// How far the drawer is out: 0 hidden, 1 fully open. Cubic ease-out over 180 ms.
    pub fn progress(&self, now: f64) -> f32 {
        if self.instant {
            return if self.open { 1.0 } else { 0.0 };
        }
        let t = ((now - self.moved_at) / SLIDE_SECONDS).clamp(0.0, 1.0) as f32;
        let eased = 1.0 - (1.0 - t).powi(3);
        if self.open { eased } else { 1.0 - eased }
    }

    pub fn is_settled(&self, now: f64) -> bool {
        self.instant || now - self.moved_at >= SLIDE_SECONDS
    }
}

/// One "About" row.
pub struct Fact {
    pub label: String,
    pub value: String,
    pub mono: bool,
}

/// The rows of the About section, in upstream's order.
pub fn facts(file: &CloudFile) -> Vec<Fact> {
    let row = &file.row;
    let mut out = vec![
        Fact {
            label: t(s::MySpace_DetailsType).to_string(),
            value: row
                .mime_type
                .clone()
                .filter(|m| !m.is_empty())
                .unwrap_or_else(|| "-".to_string()),
            mono: true,
        },
        Fact {
            label: t(s::MySpace_DetailsSize).to_string(),
            value: cloud_size(row.size),
            mono: false,
        },
    ];
    if let Some(dimensions) = dimensions(file) {
        out.push(Fact {
            label: t(s::MySpace_DetailsDimensions).to_string(),
            value: format!("{dimensions} px"),
            mono: false,
        });
    }
    if let Some(created) = &row.created_at {
        out.push(Fact {
            label: t(s::MySpace_DetailsUploaded).to_string(),
            value: modlight_core::culture::local_full(created),
            mono: false,
        });
    }
    out.push(Fact {
        label: t(s::MySpace_DetailsCode).to_string(),
        value: file.code.clone(),
        mono: true,
    });
    out
}

/// "1920 x 1080", when the server knows both.
pub fn dimensions(file: &CloudFile) -> Option<String> {
    match (file.row.width, file.row.height) {
        (Some(w), Some(h)) if w > 0 && h > 0 => Some(format!("{w} x {h}")),
        _ => None,
    }
}

/// The type badge: the extension, else the kind, upper case.
pub fn badge(file: &CloudFile) -> String {
    if let Some(ext) = file.row.ext.as_deref().filter(|e| !e.is_empty()) {
        return ext.to_uppercase();
    }
    file.row.kind.as_deref().unwrap_or("").to_uppercase()
}

/// The link settings gates: expiry, view budget, password.
pub fn gates(file: &CloudFile) -> Vec<(char, String)> {
    let row = &file.row;
    let mut out = Vec::new();
    if let Some(expires) = &row.expires_at {
        out.push((
            icons::CLOCK,
            format(
                t(s::MySpace_BadgeExpires),
                &[Arg::from(modlight_core::culture::local_short(expires))],
            ),
        ));
    }
    if let Some(max) = row.max_views {
        out.push((
            icons::FLAME,
            format(
                t(s::MySpace_BadgeMaxViews),
                &[Arg::Int(row.views), Arg::Int(max)],
            ),
        ));
    }
    if row.has_password {
        out.push((icons::KEY_ROUND, t(s::MySpace_BadgePassword).to_string()));
    }
    out
}

pub fn views_text(file: &CloudFile) -> String {
    n0(file.row.views)
}

pub fn downloads_text(file: &CloudFile) -> String {
    n0(file.row.downloads)
}

/// "3 of 9".
pub fn position_text(index: Option<usize>, total: usize) -> String {
    match index {
        Some(i) => format(
            t(s::MySpace_DetailsPosition),
            &[Arg::from(i + 1), Arg::from(total)],
        ),
        None => String::new(),
    }
}
