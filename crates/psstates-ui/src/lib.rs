//! egui user interface. Renders into any [`egui::Context`], so the same code
//! drives the desktop window (eframe) and can be rendered offscreen for a VR
//! overlay.

mod data;
mod main_ui;
mod widgets;

pub use main_ui::MainUi;

/// State of the optional SteamVR integration, shown in the settings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VrState {
    /// Connected to SteamVR and the dashboard overlay exists.
    pub connected: bool,
    /// Whether SteamVR starts the app automatically (`None` if unknown).
    pub auto_launch: Option<bool>,
    pub error: Option<String>,
}

/// Implemented by the VR integration so the UI can show and change its settings.
pub trait VrControl: Send + Sync {
    fn state(&self) -> VrState;
    fn set_auto_launch(&self, enabled: bool);
}

/// Sizes fonts, spacing and hit targets. The VR overlay gets larger text and
/// targets (laser pointing is imprecise) and no hover animations: every
/// changed frame is a full texture upload to SteamVR, so state changes should
/// settle in a single frame.
pub(crate) fn apply_style(ctx: &egui::Context, overlay: bool) {
    use egui::{vec2, FontFamily, FontId, TextStyle};
    let (body, small, heading, row_height, padding, spacing) = if overlay {
        (18.0, 14.0, 24.0, 40.0, vec2(18.0, 6.0), vec2(10.0, 6.0))
    } else {
        (15.0, 12.5, 21.0, 30.0, vec2(10.0, 5.0), vec2(8.0, 6.0))
    };
    ctx.all_styles_mut(|style| {
        style.text_styles = [
            (
                TextStyle::Heading,
                FontId::new(heading, FontFamily::Proportional),
            ),
            (TextStyle::Body, FontId::new(body, FontFamily::Proportional)),
            (
                TextStyle::Button,
                FontId::new(body, FontFamily::Proportional),
            ),
            (
                TextStyle::Small,
                FontId::new(small, FontFamily::Proportional),
            ),
            (
                TextStyle::Monospace,
                FontId::new(body - 1.0, FontFamily::Monospace),
            ),
        ]
        .into();
        let sp = &mut style.spacing;
        sp.item_spacing = spacing;
        sp.button_padding = padding;
        sp.interact_size.y = row_height;
        sp.icon_width = if overlay { 26.0 } else { 18.0 };
        sp.icon_width_inner = sp.icon_width * 0.55;
        sp.scroll.bar_width = if overlay { 18.0 } else { 10.0 };
        if overlay {
            style.animation_time = 0.0;
            // A laser pointer is never perfectly still, so tooltips would keep
            // popping in and out (each a changed frame). Not worth it in VR.
            style.interaction.tooltip_delay = 1.0e6;
        }
    });
}

/// Applies the theme to an egui context.
pub fn apply_theme(ctx: &egui::Context, theme: psstates_core::Theme) {
    let mut visuals = match theme {
        psstates_core::Theme::Dark => egui::Visuals::dark(),
        psstates_core::Theme::Light => egui::Visuals::light(),
    };
    visuals.selection.bg_fill = egui::Color32::from_rgb(37, 99, 235);
    ctx.set_visuals(visuals);
}

#[cfg(test)]
mod tests {
    #[test]
    fn icons_exist_in_default_fonts() {
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |_| {});
        let font = egui::FontId::proportional(14.0);
        for icon in [
            "⬆", "⬇", "▶", "📋", "🗑", "💾", "📤", "📥", "➕", "☀", "🌙", "⟳",
        ] {
            assert!(
                ctx.fonts(|f| f.has_glyphs(&font, icon)),
                "missing glyph {icon}"
            );
        }
    }
}
