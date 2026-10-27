//! Pieces the upload windows share: the toolbar-coloured header band with a 40 px icon
//! tile and a heading, and the bordered input card.

use egui::{Rect, Stroke, StrokeKind, Ui, pos2, vec2};

use crate::theme::{Colors, Weight};
use crate::widgets::{layout, paint_icon, trimmed, wrapped};

/// The header band (`Background.Toolbar`, padding 18,12): a 40 px tile with the icon in
/// the accent colour, the heading (19 semibold, accent) and an optional line under it
/// (an unstyled TextBlock: the global 13 px SemiBold Text.Secondary), wrapped where the
/// window wraps it and kept to one line elsewhere. Returns the band's rect.
#[allow(clippy::too_many_arguments)]
pub fn header_band(
    ui: &Ui,
    colors: &Colors,
    top_left: egui::Pos2,
    width: f32,
    glyph: char,
    heading: &str,
    heading_size: f32,
    subtitle: Option<&str>,
    tile_radius: f32,
    subtitle_wraps: bool,
) -> Rect {
    let text_width = (width - 18.0 - 40.0 - 10.0 - 18.0).max(40.0);
    let title = layout(
        ui,
        trimmed(
            heading,
            heading_size,
            Weight::SemiBold,
            colors.heading(),
            text_width,
            1,
        ),
    );
    let sub = subtitle.map(|text| {
        let job = if subtitle_wraps {
            wrapped(
                text,
                13.0,
                Weight::SemiBold,
                colors.text_secondary(),
                text_width,
            )
        } else {
            trimmed(
                text,
                13.0,
                Weight::SemiBold,
                colors.text_secondary(),
                text_width,
                1,
            )
        };
        layout(ui, job)
    });
    let text_height = title.size().y + sub.as_ref().map(|g| 2.0 + g.size().y).unwrap_or(0.0);
    let height = 12.0 + text_height.max(40.0) + 12.0;
    let band = Rect::from_min_size(top_left, vec2(width, height));
    let painter = ui.painter();
    painter.rect_filled(band, 0.0, colors.background_toolbar());
    let tile = Rect::from_min_size(
        pos2(band.left() + 18.0, band.center().y - 20.0),
        vec2(40.0, 40.0),
    );
    painter.rect_filled(tile, tile_radius, colors.background_panel());
    paint_icon(
        ui,
        painter,
        tile.center(),
        glyph,
        20.0,
        colors.accent_start(),
    );
    let left = tile.right() + 10.0;
    let top = band.center().y - text_height / 2.0;
    let title_height = title.size().y;
    painter.galley(pos2(left, top), title, colors.heading());
    if let Some(sub) = sub {
        painter.galley(
            pos2(left, top + title_height + 2.0),
            sub,
            colors.text_secondary(),
        );
    }
    band
}

/// `Border.input-card`: panel fill, 1 px border, 6 px corners.
pub fn input_card(ui: &Ui, colors: &Colors, rect: Rect) {
    let painter = ui.painter();
    painter.rect_filled(rect, 6.0, colors.background_panel());
    painter.rect_stroke(
        rect,
        6.0,
        Stroke::new(1.0, colors.border()),
        StrokeKind::Inside,
    );
}
