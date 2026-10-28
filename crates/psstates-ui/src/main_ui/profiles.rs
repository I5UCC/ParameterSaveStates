//! The profiles page: connection summary, saving and applying profiles.

use egui::{RichText, Ui};
use psstates_core::MoveDirection;
use psstates_service::Status;

use super::*;
use crate::widgets::{hint, AMBER, RED};

impl MainUi {
    pub(super) fn profiles_page(&mut self, ui: &mut Ui) {
        let status = self.status.clone();
        let store = self.store().clone();

        self.status_bar(ui, &status);
        ui.add_space(4.0);

        let Some(current) = status.current_avatar.clone() else {
            ui.add_space(8.0);
            ui.label(RichText::new("No avatar detected yet.").weak());
            return;
        };
        let avatar = self.data.avatar(&store, &current.id).clone();

        // Save row: name field fills the width, button on the right.
        let row_h = ui.spacing().interact_size.y;
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let can_save = status.connected && status.busy.is_none();
                let save = ui
                    .add_enabled(
                        can_save,
                        egui::Button::new(RichText::new("💾 Save new").strong())
                            .min_size(egui::vec2(140.0, row_h)),
                    )
                    .on_disabled_hover_text("VRChat must be connected");
                let resp = ui.add_sized(
                    [ui.available_width(), row_h],
                    egui::TextEdit::singleline(&mut self.new_profile_name)
                        .hint_text(format!("New profile name ({})", avatar.next_default_name()))
                        .vertical_align(egui::Align::Center),
                );
                let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if (save.clicked() || enter) && can_save {
                    self.actions.push(Action::SaveNew);
                }
            });
        });
        ui.add_space(6.0);

        // Header: title on the left, actions on the right.
        let prev_has_profiles = status.previous_avatar.as_ref().is_some_and(|p| {
            self.data
                .avatars
                .iter()
                .any(|a| a.id == p.id && a.profile_count > 0)
        });
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("Profiles ({})", avatar.profiles.len()))
                    .strong()
                    .size(ui.text_style_height(&egui::TextStyle::Heading) * 0.85),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if !avatar.profiles.is_empty() {
                    let label = if self.edit_mode { "Done" } else { "Edit" };
                    if ui.button(label).clicked() {
                        self.edit_mode = !self.edit_mode;
                    }
                }
                if prev_has_profiles
                    && ui
                        .button("📋 Copy from previous avatar")
                        .on_hover_text("Copy all profiles of the previous avatar to this one")
                        .clicked()
                {
                    self.actions.push(Action::Open(Dialog::CopyPrevious));
                }
            });
        });
        if avatar.profiles.is_empty() {
            ui.add_space(6.0);
            ui.label(RichText::new("No profiles saved for this avatar yet.").weak());
            return;
        }

        let count = avatar.profiles.len();
        let edit = self.edit_mode;
        for (i, p) in avatar.profiles.iter().enumerate() {
            egui::Frame::new()
                .fill(ui.visuals().code_bg_color)
                .corner_radius(6)
                .inner_margin(egui::Margin::symmetric(10, 3))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            // Buttons are added right to left.
                            let apply = egui::Button::new(
                                RichText::new("Apply").strong().color(egui::Color32::WHITE),
                            )
                            .fill(egui::Color32::from_rgb(22, 140, 70))
                            .min_size(egui::vec2(110.0, 0.0));
                            if ui.add(apply).clicked() {
                                self.actions.push(Action::Apply(p.name.clone()));
                            }
                            if edit {
                                if ui.button(RichText::new("Delete").color(RED)).clicked() {
                                    self.actions
                                        .push(Action::Open(Dialog::Delete(p.name.clone())));
                                }
                                if ui.button("Rename").clicked() {
                                    self.actions.push(Action::Open(Dialog::Rename {
                                        name: p.name.clone(),
                                        new_name: p.name.clone(),
                                    }));
                                }
                                if ui
                                    .add_enabled(status.connected, egui::Button::new("Override"))
                                    .on_hover_text("Replace with the current avatar state")
                                    .clicked()
                                {
                                    self.actions
                                        .push(Action::Open(Dialog::Override(p.name.clone())));
                                }
                                if ui
                                    .add_enabled(i + 1 < count, egui::Button::new("⬇"))
                                    .clicked()
                                {
                                    self.actions
                                        .push(Action::Move(p.name.clone(), MoveDirection::Down));
                                }
                                if ui.add_enabled(i > 0, egui::Button::new("⬆")).clicked() {
                                    self.actions
                                        .push(Action::Move(p.name.clone(), MoveDirection::Up));
                                }
                            }
                            // The name takes the remaining space on the left.
                            ui.with_layout(
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    ui.add(
                                        egui::Label::new(RichText::new(&p.name).strong())
                                            .truncate(),
                                    )
                                    .on_hover_text(format!("{} parameters", p.parameters.len()));
                                },
                            );
                        });
                    });
                });
        }
    }

    /// Connection and avatar summary at the top of the profiles page.
    pub(super) fn status_bar(&mut self, ui: &mut Ui, status: &Status) {
        egui::Frame::group(ui.style())
            .inner_margin(egui::Margin::symmetric(10, 6))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                // One row: avatar name (or what we are waiting for) and a small
                // reconnect button. The connection state is shown in the header.
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .button("⟳")
                            .on_hover_text("Reconnect to VRChat")
                            .clicked()
                        {
                            self.actions.push(Action::Reconnect);
                        }
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            match &status.current_avatar {
                                Some(a) => {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(a.short()).heading().strong(),
                                        )
                                        .truncate(),
                                    );
                                }
                                None if status.connected => {
                                    ui.label(RichText::new("Waiting for an avatar...").weak());
                                }
                                None => {
                                    ui.colored_label(AMBER, "Waiting for VRChat...");
                                }
                            }
                        });
                    });
                });
                // Details on one thin line: id and previous avatar.
                let mut details = Vec::new();
                if let Some(a) = status.current_avatar.as_ref().filter(|a| a.name.is_some()) {
                    details.push(a.id.clone());
                }
                if let Some(prev) = &status.previous_avatar {
                    details.push(format!("Previous: {}", prev.short()));
                }
                if !details.is_empty() {
                    ui.add(
                        egui::Label::new(RichText::new(details.join("  ·  ")).weak().small())
                            .truncate(),
                    );
                }
                if let Some(err) = &status.discovery_error {
                    ui.colored_label(RED, format!("Network discovery unavailable: {err}"));
                }
                if !status.connected {
                    hint(
                        ui,
                        "Make sure OSC is enabled in VRChat (Action Menu > Options > OSC > Enabled).",
                    );
                }
            });
    }
}
