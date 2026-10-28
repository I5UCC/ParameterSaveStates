//! The settings page and its tabs.

mod fixed;
mod general;
mod lists;

use egui::Ui;

use super::*;

impl MainUi {
    pub(super) fn settings_page(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            for (tab, label) in [
                (SettingsTab::General, "General"),
                (SettingsTab::Filters, "Profile Filters"),
                (SettingsTab::AutoSync, "Auto Sync"),
                (SettingsTab::Fixed, "Fixed Parameters"),
            ] {
                ui.selectable_value(&mut self.settings_tab, tab, label);
            }
        });
        ui.separator();
        match self.settings_tab {
            SettingsTab::General => self.general_settings(ui),
            SettingsTab::Filters => self.list_settings(ui, ListKind::Excluded),
            SettingsTab::AutoSync => self.list_settings(ui, ListKind::AutoSync),
            SettingsTab::Fixed => self.fixed_settings(ui),
        }
    }
}
