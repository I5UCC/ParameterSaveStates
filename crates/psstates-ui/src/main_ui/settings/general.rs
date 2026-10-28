//! General settings: copy and delete avatar data, backup, legacy import, SteamVR, about.

use egui::{RichText, Ui};
use psstates_service::VERSION;

use crate::main_ui::*;
use crate::widgets::{card, hint, AMBER, GREEN, RED};
use crate::VrControl;

impl MainUi {
    pub(super) fn general_settings(&mut self, ui: &mut Ui) {
        let current = self.status.current_avatar.clone();
        let with_profiles = self.data.avatars_with_profiles();
        let all = self.data.avatars.clone();

        card(ui, "Profile management", |ui| {
            ui.label("Copy all profiles of an avatar to the current avatar:");
            ui.horizontal(|ui| {
                self.copy_picker.ui(ui, "copy_picker", &with_profiles);
                let target_ok = current.is_some()
                    && self.copy_picker.selected.is_some()
                    && self.copy_picker.selected.as_ref() != current.as_ref().map(|c| &c.id);
                if ui
                    .add_enabled(target_ok, egui::Button::new("Copy to current avatar"))
                    .clicked()
                {
                    let id = self.copy_picker.selected.clone().unwrap_or_default();
                    let label = all
                        .iter()
                        .find(|a| a.id == id)
                        .map(|a| a.display())
                        .unwrap_or_else(|| id.clone());
                    self.actions
                        .push(Action::Open(Dialog::CopyFromAvatar { id, label }));
                }
            });
            ui.add_space(8.0);
            ui.label("Delete all stored data (profiles and settings) of an avatar:");
            ui.horizontal(|ui| {
                self.delete_picker.ui(ui, "delete_picker", &all);
                if ui
                    .add_enabled(
                        self.delete_picker.selected.is_some(),
                        egui::Button::new(RichText::new("Delete avatar data").color(RED)),
                    )
                    .clicked()
                {
                    let id = self.delete_picker.selected.clone().unwrap_or_default();
                    let label = all
                        .iter()
                        .find(|a| a.id == id)
                        .map(|a| a.display())
                        .unwrap_or_else(|| id.clone());
                    self.actions
                        .push(Action::Open(Dialog::DeleteAvatar { id, label }));
                }
            });
        });
        ui.add_space(8.0);

        if let Some(vr) = self.vr.clone() {
            self.vr_card(ui, vr.as_ref());
            ui.add_space(8.0);
        }

        if self.overlay_mode {
            card(ui, "More settings", |ui| {
                hint(
                    ui,
                    "Backup, restore and importing old data are available in the desktop window.",
                );
            });
            ui.add_space(8.0);
            self.about_card(ui);
            return;
        }

        card(ui, "Backup / restore", |ui| {
            ui.horizontal(|ui| {
                if ui.button("📤 Export backup...").clicked() {
                    self.actions.push(Action::Export);
                }
                if ui.button("📥 Import backup...").clicked() {
                    self.actions.push(Action::PickImport);
                }
            });
            hint(
                ui,
                "Import accepts backups of this version and of the old Unity version. \
                 Existing data is overwritten by the backup's files.",
            );
        });
        ui.add_space(8.0);

        card(ui, "Import from the old (Unity) version", |ui| {
            match &self.legacy_dir {
                Some(dir) => {
                    ui.label(format!("Found old data at {}", dir.display()));
                }
                None => {
                    ui.label(RichText::new("No old data found automatically.").weak());
                }
            }
            ui.horizontal_wrapped(|ui| {
                if let Some(dir) = &self.legacy_dir {
                    if ui.button("Import old profiles").clicked() {
                        self.actions.push(Action::MigrateLegacy(dir.clone()));
                    }
                }
                if ui.button("Choose 'Profiles' folder...").clicked() {
                    self.actions.push(Action::PickLegacyDir);
                }
                if let Some(dir) = &self.legacy_dir {
                    if ui.button("🗑 Delete old folder").clicked() {
                        self.actions
                            .push(Action::Open(Dialog::DeleteLegacy(dir.clone())));
                    }
                }
            });
            hint(
                ui,
                "Existing profiles are kept; imported profiles with the same name get a '(legacy)' suffix.",
            );
        });
        ui.add_space(8.0);
        self.about_card(ui);
    }

    pub(super) fn vr_card(&mut self, ui: &mut Ui, vr: &dyn VrControl) {
        let state = vr.state();
        card(ui, "SteamVR", |ui| {
            if state.connected {
                ui.colored_label(GREEN, "Dashboard overlay active");
            } else {
                ui.colored_label(AMBER, "SteamVR not running");
            }
            if let Some(err) = &state.error {
                ui.colored_label(RED, err);
            }
            match state.auto_launch {
                Some(mut enabled) => {
                    if ui
                        .checkbox(&mut enabled, "Start automatically with SteamVR")
                        .changed()
                    {
                        vr.set_auto_launch(enabled);
                    }
                }
                None => hint(
                    ui,
                    "Start SteamVR to configure automatic start (SteamVR > Settings > \
                     Startup / Shutdown > Choose startup overlay apps).",
                ),
            }
        });
    }

    pub(super) fn about_card(&mut self, ui: &mut Ui) {
        card(ui, "About", |ui| {
            let root = self.store().root().display().to_string();
            egui::Grid::new("about_grid")
                .num_columns(2)
                .spacing([16.0, 4.0])
                .show(ui, |ui| {
                    ui.label("Version");
                    ui.label(VERSION);
                    ui.end_row();
                    ui.label("Data folder");
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&root).monospace());
                        if ui.small_button("Copy").clicked() {
                            ui.ctx().copy_text(root.clone());
                        }
                    });
                    ui.end_row();
                    ui.label("OSC receive port");
                    ui.label(self.status.osc_port.to_string());
                    ui.end_row();
                    ui.label("OSCQuery port");
                    ui.label(self.status.oscquery_port.to_string());
                    ui.end_row();
                    ui.label("Sending to");
                    ui.label(
                        self.status
                            .osc_target
                            .map(|a| a.to_string())
                            .unwrap_or_else(|| "-".into()),
                    );
                    ui.end_row();
                });
        });
    }
}
