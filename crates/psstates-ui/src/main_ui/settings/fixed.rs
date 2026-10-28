//! Fixed parameters (global and per avatar).

use egui::Ui;

use crate::main_ui::*;
use crate::widgets::{card, hint};

impl MainUi {
    pub(super) fn fixed_settings(&mut self, ui: &mut Ui) {
        let store = self.store().clone();
        let avatars = self.data.avatars.clone();
        let global_items = self.data.settings.fixed_parameters.clone();
        if self.fixed.picker.selected.is_none() {
            self.fixed.picker.selected = self.status.current_avatar.as_ref().map(|a| a.id.clone());
        }

        hint(
            ui,
            "Fixed parameters are sent after a profile is applied and after every avatar change. \
             Per-avatar values override global ones with the same name.",
        );
        ui.add_space(6.0);
        let mut actions = Vec::new();
        let fixed = &mut self.fixed;
        card(ui, "Fixed parameters: all avatars", |ui| {
            fixed.global.sync("global", &global_items);
            if let Some(items) = fixed.global.ui(ui, "fixed_global", &avatars, &store) {
                fixed.global.mark_saved();
                actions.push(Action::SaveFixed {
                    avatar: None,
                    items,
                });
            }
        });
        ui.add_space(8.0);
        card(ui, "Fixed parameters: per avatar", |ui| {
            fixed.picker.ui(ui, "fixed_picker", &avatars);
            ui.add_space(4.0);
            if let Some(id) = fixed.picker.selected.clone() {
                let items = self.data.avatar(&store, &id).fixed_parameters.clone();
                fixed.avatar.sync(&id, &items);
                if let Some(items) = fixed.avatar.ui(ui, "fixed_avatar", &avatars, &store) {
                    fixed.avatar.mark_saved();
                    actions.push(Action::SaveFixed {
                        avatar: Some(id),
                        items,
                    });
                }
            }
        });
        self.actions.extend(actions);
    }
}
