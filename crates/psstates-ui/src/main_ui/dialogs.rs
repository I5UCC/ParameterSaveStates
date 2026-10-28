//! Confirmation dialogs and the legacy import prompt.

use egui::{RichText, Ui};

use super::*;
use crate::widgets::RED;

impl MainUi {
    pub(super) fn check_legacy_prompt(&mut self) {
        if self.legacy_checked {
            return;
        }
        self.legacy_checked = true;
        if self.data.settings.legacy_import_prompted {
            return;
        }
        if let Some(dir) = &self.legacy_dir {
            if self.dialog.is_none() {
                self.dialog = Some(Dialog::LegacyPrompt(dir.clone()));
            }
        }
    }

    pub(super) fn dialog_ui(&mut self, ctx: &egui::Context) {
        let focus_new_dialog = std::mem::take(&mut self.dialog_opened) && !self.overlay_mode;
        let Some(dialog) = self.dialog.as_mut() else {
            return;
        };
        let mut actions = Vec::new();
        let cancelled = std::cell::Cell::new(false);
        let modal = egui::Modal::new(egui::Id::new("dialog")).show(ctx, |ui| {
            ui.set_width(380.0);
            let confirm = |ui: &mut Ui, title: &str, text: &str, button: &str, danger: bool| {
                ui.heading(title);
                ui.add_space(6.0);
                ui.label(text);
                ui.add_space(12.0);
                let mut ok = false;
                ui.horizontal(|ui| {
                    let label = if danger {
                        RichText::new(button).color(RED)
                    } else {
                        RichText::new(button)
                    };
                    ok = ui.button(label).clicked();
                    if ui.button("Cancel").clicked() {
                        cancelled.set(true);
                    }
                });
                ok
            };
            match dialog {
                Dialog::Override(name) => {
                    if confirm(
                        ui,
                        "Override profile",
                        &format!("Replace '{name}' with the current avatar state?"),
                        "Override",
                        false,
                    ) {
                        actions.push(Action::Override(name.clone()));
                    }
                }
                Dialog::Delete(name) => {
                    if confirm(ui, "Delete profile", &format!("Delete '{name}'?"), "Delete", true) {
                        actions.push(Action::Delete(name.clone()));
                    }
                }
                Dialog::Rename { name, new_name } => {
                    ui.heading("Rename profile");
                    ui.add_space(6.0);
                    let resp = ui.add(
                        egui::TextEdit::singleline(new_name).desired_width(f32::INFINITY),
                    );
                    // Focus once when opened (not in VR, where focus opens the keyboard).
                    if focus_new_dialog {
                        resp.request_focus();
                    }
                    let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui.button("Rename").clicked() || enter {
                            actions.push(Action::Rename(name.clone(), new_name.clone()));
                        }
                        if ui.button("Cancel").clicked() {
                            actions.push(Action::CloseDialog);
                        }
                    });
                }
                Dialog::CopyPrevious => {
                    if confirm(
                        ui,
                        "Copy profiles",
                        "Copy all profiles of the previous avatar to the current avatar? \
                         Profiles with the same name are overwritten.",
                        "Copy",
                        false,
                    ) {
                        actions.push(Action::CopyPrevious);
                    }
                }
                Dialog::CopyFromAvatar { id, label } => {
                    if confirm(
                        ui,
                        "Copy profiles",
                        &format!(
                            "Copy all profiles of {label} to the current avatar? \
                             Profiles with the same name are overwritten."
                        ),
                        "Copy",
                        false,
                    ) {
                        actions.push(Action::CopyFromAvatar(id.clone()));
                    }
                }
                Dialog::DeleteAvatar { id, label } => {
                    if confirm(
                        ui,
                        "Delete avatar data",
                        &format!("Delete all profiles and settings of {label}? This cannot be undone."),
                        "Delete",
                        true,
                    ) {
                        actions.push(Action::DeleteAvatar(id.clone()));
                    }
                }
                Dialog::Import(path) => {
                    if confirm(
                        ui,
                        "Import backup",
                        &format!(
                            "Import '{}'? Files contained in the backup overwrite existing data.",
                            path.display()
                        ),
                        "Import",
                        false,
                    ) {
                        actions.push(Action::Import(path.clone()));
                    }
                }
                Dialog::DeleteLegacy(path) => {
                    if confirm(
                        ui,
                        "Delete old folder",
                        &format!(
                            "Permanently delete the old data folder '{}'? This cannot be undone.",
                            path.display()
                        ),
                        "Delete",
                        true,
                    ) {
                        actions.push(Action::DeleteLegacy(path.clone()));
                    }
                }
                Dialog::LegacyPrompt(path) => {
                    ui.heading("Import old profiles?");
                    ui.add_space(6.0);
                    ui.label(format!(
                        "Profiles from the old ParameterSaveStates version were found at\n{}\n\nImport them now?",
                        path.display()
                    ));
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui.button("Import").clicked() {
                            actions.push(Action::MigrateLegacy(path.clone()));
                        }
                        if ui.button("Not now").clicked() {
                            actions.push(Action::CloseDialog);
                        }
                        if ui.button("Don't ask again").clicked() {
                            actions.push(Action::SkipLegacy);
                        }
                    });
                }
            }
        });
        if (cancelled.get() || modal.should_close()) && actions.is_empty() {
            actions.push(Action::CloseDialog);
        }
        self.actions.extend(actions);
    }
}
