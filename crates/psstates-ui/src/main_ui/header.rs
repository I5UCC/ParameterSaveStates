//! Title bar: tabs, theme toggle and connection state.

use egui::{RichText, Ui};
use psstates_core::Theme;
use psstates_service::VERSION;

use super::*;
use crate::widgets::{AMBER, GREEN};

impl MainUi {
    pub(super) fn header(&mut self, ui: &mut Ui) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Parameter Save States").strong().size(20.0));
            ui.label(RichText::new(format!("v{VERSION}")).weak());
            ui.add_space(16.0);
            ui.selectable_value(
                &mut self.tab,
                Tab::Profiles,
                RichText::new("Profiles").size(15.0),
            );
            ui.selectable_value(
                &mut self.tab,
                Tab::Settings,
                RichText::new("Settings").size(15.0),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let theme = self.data.settings.theme;
                let (icon, next) = match theme {
                    Theme::Dark => ("☀", Theme::Light),
                    Theme::Light => ("🌙", Theme::Dark),
                };
                if ui.button(icon).on_hover_text("Toggle theme").clicked() {
                    self.actions.push(Action::SetTheme(next));
                }
                self.connection_badge(ui);
            });
        });
        ui.add_space(6.0);
    }

    pub(super) fn connection_badge(&self, ui: &mut Ui) {
        if let Some(busy) = &self.status.busy {
            ui.label(busy);
            ui.spinner();
        } else if self.status.connected {
            ui.colored_label(GREEN, "Connected")
                .on_hover_text(self.status.vrchat_instance.as_deref().unwrap_or("VRChat"));
        } else {
            ui.colored_label(AMBER, "Waiting for VRChat");
        }
    }
}
