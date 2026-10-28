//! Profile filters and auto sync: parameter name lists (global and per avatar).

use egui::Ui;

use crate::main_ui::*;
use crate::widgets::{card, hint};

impl MainUi {
    pub(super) fn list_settings(&mut self, ui: &mut Ui, kind: ListKind) {
        let (title, description) = match kind {
            ListKind::Excluded => (
                "Profile filters",
                "Parameters listed here are never applied when loading a profile. \
                 Names match the full path (/avatar/parameters/X), the path relative to \
                 /avatar/parameters/ or just the last segment.",
            ),
            ListKind::AutoSync => (
                "Auto sync",
                "When switching avatars, the values of these parameters are copied from the \
                 previous avatar to the new one. Same name matching rules as filters.",
            ),
        };
        let store = self.store().clone();
        let avatars = self.data.avatars.clone();
        let current_id = self.status.current_avatar.as_ref().map(|a| a.id.clone());
        let global_items = match kind {
            ListKind::Excluded => self.data.settings.excluded_parameters.clone(),
            ListKind::AutoSync => self.data.settings.auto_sync_parameters.clone(),
        };

        let lists = match kind {
            ListKind::Excluded => &mut self.filters,
            ListKind::AutoSync => &mut self.auto_sync,
        };
        if lists.picker.selected.is_none() {
            lists.picker.selected = current_id;
        }

        hint(ui, description);
        ui.add_space(6.0);
        let mut actions = Vec::new();
        card(ui, &format!("{title}: all avatars"), |ui| {
            lists.global.sync("global", &global_items);
            if let Some(items) = lists.global.ui(ui, "global") {
                lists.global.mark_saved();
                actions.push(Action::SaveList {
                    kind,
                    avatar: None,
                    items,
                });
            }
        });
        ui.add_space(8.0);
        card(ui, &format!("{title}: per avatar"), |ui| {
            lists.picker.ui(ui, &format!("{kind:?}_picker"), &avatars);
            ui.add_space(4.0);
            if let Some(id) = lists.picker.selected.clone() {
                let data = self.data.avatar(&store, &id);
                let items = match kind {
                    ListKind::Excluded => data.excluded_parameters.clone(),
                    ListKind::AutoSync => data.auto_sync_parameters.clone(),
                };
                lists.avatar.sync(&id, &items);
                if let Some(items) = lists.avatar.ui(ui, "avatar") {
                    lists.avatar.mark_saved();
                    actions.push(Action::SaveList {
                        kind,
                        avatar: Some(id),
                        items,
                    });
                }
            }
        });
        self.actions.extend(actions);
    }
}
