//! Reusable widgets: cards, avatar picker, name list editor, fixed parameter editor.

use egui::{Color32, RichText, Ui};
use psstates_core::model::FixedParam;
use psstates_core::rules::{normalize_name_list, resolve_fixed_from_profile};
use psstates_core::{AvatarSummary, ParamType, ParamValue, Store};

pub const GREEN: Color32 = Color32::from_rgb(34, 197, 94);
pub const AMBER: Color32 = Color32::from_rgb(245, 158, 11);
pub const RED: Color32 = Color32::from_rgb(244, 63, 94);

/// A framed section with a title.
pub fn card<R>(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::group(ui.style())
        .inner_margin(12.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).strong().size(16.0));
            ui.add_space(6.0);
            add(ui)
        })
        .inner
}

pub fn hint(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).weak().small());
}

// ------------------------------------------------------------------ picker

/// Searchable avatar selection combo box.
#[derive(Default)]
pub struct AvatarPicker {
    pub selected: Option<String>,
    filter: String,
}

impl AvatarPicker {
    /// Returns `true` if the selection changed.
    pub fn ui(&mut self, ui: &mut Ui, id_salt: &str, avatars: &[AvatarSummary]) -> bool {
        let mut changed = false;
        if let Some(sel) = &self.selected {
            if !avatars.iter().any(|a| &a.id == sel) {
                self.selected = None;
                changed = true;
            }
        }
        let selected_text = self
            .selected
            .as_ref()
            .and_then(|id| avatars.iter().find(|a| &a.id == id))
            .map(AvatarSummary::display)
            .unwrap_or_else(|| "Select avatar...".to_string());

        egui::ComboBox::from_id_salt(id_salt)
            .width(ui.available_width().min(460.0))
            .selected_text(selected_text)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show_ui(ui, |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.filter)
                        .hint_text("Search name or id...")
                        .desired_width(f32::INFINITY),
                );
                let needle = self.filter.trim().to_lowercase();
                egui::ScrollArea::vertical()
                    .max_height(260.0)
                    .show(ui, |ui| {
                        let mut any = false;
                        for a in avatars {
                            let label = a.display();
                            if !needle.is_empty() && !label.to_lowercase().contains(&needle) {
                                continue;
                            }
                            any = true;
                            let text = format!("{label}  [{}]", a.profile_count);
                            let is_sel = self.selected.as_deref() == Some(a.id.as_str());
                            if ui.selectable_label(is_sel, text).clicked() {
                                self.selected = Some(a.id.clone());
                                changed = true;
                                ui.memory_mut(|m| m.close_popup());
                            }
                        }
                        if !any {
                            ui.label(RichText::new("No avatars").weak());
                        }
                    });
            });
        changed
    }
}

// ------------------------------------------------------------- list editor

/// Editor for a list of parameter names with explicit save.
#[derive(Default)]
pub struct ListEditor {
    items: Vec<String>,
    input: String,
    dirty: bool,
    loaded_key: Option<String>,
}

impl ListEditor {
    /// Syncs with the stored list unless there are unsaved edits for the same key.
    pub fn sync(&mut self, key: &str, source: &[String]) {
        if self.loaded_key.as_deref() == Some(key) && self.dirty {
            return;
        }
        if self.loaded_key.as_deref() != Some(key) {
            self.input.clear();
        }
        self.loaded_key = Some(key.to_string());
        self.items = source.to_vec();
        self.dirty = false;
    }

    pub fn mark_saved(&mut self) {
        self.dirty = false;
    }

    /// Returns the normalised list when the user clicked save.
    pub fn ui(&mut self, ui: &mut Ui, id_salt: &str) -> Option<Vec<String>> {
        let mut remove = None;
        ui.push_id(id_salt, |ui| {
            if self.items.is_empty() {
                ui.label(RichText::new("No parameters configured").weak());
            }
            for (i, item) in self.items.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    // Button first (right aligned), the field takes the rest. Measuring
                    // the button by hand overflowed and made the page grow wider.
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("🗑").on_hover_text("Remove").clicked() {
                            remove = Some(i);
                        }
                        if ui
                            .add(egui::TextEdit::singleline(item).desired_width(f32::INFINITY))
                            .changed()
                        {
                            self.dirty = true;
                        }
                    });
                });
            }
        });
        if let Some(i) = remove {
            self.items.remove(i);
            self.dirty = true;
        }

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(!self.input.trim().is_empty(), egui::Button::new("Add"))
                    .clicked()
                {
                    self.items.extend(
                        self.input
                            .split(['\n', ','])
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(str::to_string),
                    );
                    self.input.clear();
                    self.dirty = true;
                }
                ui.add(
                    egui::TextEdit::multiline(&mut self.input)
                        .desired_rows(1)
                        .desired_width(f32::INFINITY)
                        .hint_text("Add names (one per line or comma separated)"),
                );
            });
        });

        let mut save = None;
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(self.dirty, egui::Button::new("💾 Save"))
                .clicked()
            {
                self.items = normalize_name_list(&self.items);
                save = Some(self.items.clone());
            }
            if ui
                .add_enabled(self.dirty, egui::Button::new("Revert"))
                .clicked()
            {
                self.loaded_key = None;
                self.dirty = false;
            }
            if self.dirty {
                ui.colored_label(AMBER, "Unsaved changes");
            }
        });
        save
    }
}

// ------------------------------------------------------- fixed parameters

struct FixedRow {
    name: String,
    ty: ParamType,
    value: String,
}

impl FixedRow {
    fn from_param(p: &FixedParam) -> Self {
        Self {
            name: p.name.clone(),
            ty: p.value.param_type(),
            value: p.value.to_text(),
        }
    }

    fn parse(&self) -> Result<FixedParam, String> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err("Name is required".into());
        }
        ParamValue::parse(self.ty, &self.value)
            .map(|v| FixedParam::new(name, v))
            .map_err(|e| format!("{e} for '{name}'"))
    }
}

/// Editor for fixed parameters, with "fill values from profile".
#[derive(Default)]
pub struct FixedEditor {
    rows: Vec<FixedRow>,
    dirty: bool,
    loaded_key: Option<String>,
    fill_avatar: AvatarPicker,
    fill_profile: Option<String>,
    info: Option<(bool, String)>,
}

impl FixedEditor {
    pub fn sync(&mut self, key: &str, source: &[FixedParam]) {
        if self.loaded_key.as_deref() == Some(key) && self.dirty {
            return;
        }
        if self.loaded_key.as_deref() != Some(key) {
            self.info = None;
        }
        self.loaded_key = Some(key.to_string());
        self.rows = source.iter().map(FixedRow::from_param).collect();
        self.dirty = false;
    }

    pub fn mark_saved(&mut self) {
        self.dirty = false;
    }

    /// Returns the parameters when the user clicked save and all rows are valid.
    pub fn ui(
        &mut self,
        ui: &mut Ui,
        id_salt: &str,
        avatars: &[AvatarSummary],
        store: &Store,
    ) -> Option<Vec<FixedParam>> {
        let mut remove = None;
        ui.push_id(id_salt, |ui| {
            if self.rows.is_empty() {
                ui.label(RichText::new("No fixed parameters configured").weak());
            }
            // The name field takes what the type, value and remove columns leave.
            let row_h = ui.spacing().interact_size.y;
            let value_width = 110.0;
            let type_width = 90.0;
            let name_width = (ui.available_width()
                - value_width
                - type_width
                - row_h * 2.0
                - 40.0)
                .clamp(200.0, 700.0);
            egui::Grid::new("fixed_rows")
                .num_columns(4)
                .spacing([10.0, 8.0])
                .show(ui, |ui| {
                    for (i, row) in self.rows.iter_mut().enumerate() {
                        if ui
                            .add_sized(
                                [name_width, row_h],
                                egui::TextEdit::singleline(&mut row.name)
                                    .hint_text("Parameter name")
                                    .margin(egui::vec2(6.0, 4.0)),
                            )
                            .changed()
                        {
                            self.dirty = true;
                        }
                        let before = row.ty;
                        egui::ComboBox::from_id_salt(("ty", i))
                            .width(type_width)
                            .selected_text(row.ty.label())
                            .show_ui(ui, |ui| {
                                for ty in ParamType::ALL {
                                    ui.selectable_value(&mut row.ty, ty, ty.label());
                                }
                            });
                        if row.ty != before {
                            self.dirty = true;
                            if ParamValue::parse(row.ty, &row.value).is_err() {
                                row.value = row.ty.default_value().to_text();
                            }
                        }
                        let valid = ParamValue::parse(row.ty, &row.value).is_ok();
                        if row.ty == ParamType::Bool {
                            let mut b = row.value.trim().eq_ignore_ascii_case("true");
                            if ui.checkbox(&mut b, "").changed() {
                                row.value = b.to_string();
                                self.dirty = true;
                            }
                        } else {
                            let mut edit = egui::TextEdit::singleline(&mut row.value)
                                .margin(egui::vec2(6.0, 4.0));
                            if !valid {
                                edit = edit.text_color(RED);
                            }
                            if ui.add_sized([value_width, row_h], edit).changed() {
                                self.dirty = true;
                            }
                        }
                        if ui.small_button("🗑").on_hover_text("Remove").clicked() {
                            remove = Some(i);
                        }
                        ui.end_row();
                    }
                });
        });
        if let Some(i) = remove {
            self.rows.remove(i);
            self.dirty = true;
        }

        ui.add_space(4.0);
        if ui.button("➕ Add parameter").clicked() {
            self.rows.push(FixedRow {
                name: String::new(),
                ty: ParamType::Bool,
                value: "true".into(),
            });
            self.dirty = true;
        }

        // Fill values from a saved profile.
        ui.add_space(6.0);
        egui::CollapsingHeader::new("Fill values from a saved profile")
            .id_salt(format!("{id_salt}_fill_header"))
            .show(ui, |ui| {
                hint(
                    ui,
                    "Looks up the parameter names above in a saved profile and copies its values.",
                );
                let with_profiles: Vec<AvatarSummary> = avatars
                    .iter()
                    .filter(|a| a.profile_count > 0)
                    .cloned()
                    .collect();
                if self
                    .fill_avatar
                    .ui(ui, &format!("{id_salt}_fill_avatar"), &with_profiles)
                {
                    self.fill_profile = None;
                }
                let profiles = self
                    .fill_avatar
                    .selected
                    .as_ref()
                    .and_then(|id| store.avatar(id).ok())
                    .map(|a| a.profile_names())
                    .unwrap_or_default();
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt(format!("{id_salt}_fill_profile"))
                        .width(240.0)
                        .selected_text(self.fill_profile.as_deref().unwrap_or("Select profile..."))
                        .show_ui(ui, |ui| {
                            for p in &profiles {
                                ui.selectable_value(&mut self.fill_profile, Some(p.clone()), p);
                            }
                        });
                    let can_fill = self.fill_profile.is_some() && !self.rows.is_empty();
                    if ui
                        .add_enabled(can_fill, egui::Button::new("Fill values"))
                        .clicked()
                    {
                        self.fill_from_profile(store);
                    }
                });
            });

        if let Some((ok, text)) = &self.info {
            ui.colored_label(if *ok { GREEN } else { RED }, text);
        }

        let mut save = None;
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(self.dirty, egui::Button::new("💾 Save"))
                .clicked()
            {
                match self
                    .rows
                    .iter()
                    .map(FixedRow::parse)
                    .collect::<Result<Vec<_>, _>>()
                {
                    Ok(params) => {
                        self.info = None;
                        save = Some(params);
                    }
                    Err(e) => self.info = Some((false, e)),
                }
            }
            if ui
                .add_enabled(self.dirty, egui::Button::new("Revert"))
                .clicked()
            {
                self.loaded_key = None;
                self.dirty = false;
                self.info = None;
            }
            if self.dirty {
                ui.colored_label(AMBER, "Unsaved changes");
            }
        });
        save
    }

    fn fill_from_profile(&mut self, store: &Store) {
        let (Some(avatar_id), Some(profile_name)) =
            (self.fill_avatar.selected.clone(), self.fill_profile.clone())
        else {
            return;
        };
        let avatar = match store.avatar(&avatar_id) {
            Ok(a) => a,
            Err(e) => {
                self.info = Some((false, e.to_string()));
                return;
            }
        };
        let Some(profile) = avatar.find_profile(&profile_name) else {
            self.info = Some((false, format!("Profile '{profile_name}' not found")));
            return;
        };
        let names: Vec<String> = self.rows.iter().map(|r| r.name.clone()).collect();
        let resolved = resolve_fixed_from_profile(profile, &names);
        let mut filled = 0;
        for row in &mut self.rows {
            if let Some(p) = resolved
                .iter()
                .find(|p| p.name.to_lowercase() == row.name.trim().to_lowercase())
            {
                row.ty = p.value.param_type();
                row.value = p.value.to_text();
                filled += 1;
            }
        }
        if filled > 0 {
            self.dirty = true;
        }
        self.info = Some((
            filled > 0,
            format!(
                "Filled {filled} of {} value(s) from '{profile_name}'",
                self.rows.len()
            ),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rows must fill the container exactly; overflowing grows the page wider every frame.
    #[test]
    fn list_editor_stays_within_container() {
        let ctx = egui::Context::default();
        let mut editor = ListEditor::default();
        editor.sync("k", &["A".into(), "B".into(), "C".into(), "D".into()]);
        let mut widths = Vec::new();
        for _ in 0..4 {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 600.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let avail = ui.available_width();
                        card(ui, "t", |ui| {
                            editor.ui(ui, "x");
                        });
                        widths.push((avail, ui.min_rect().width()));
                    });
                },
            );
        }
        for (avail, used) in widths {
            assert!(used <= avail + 0.5, "content {used} wider than {avail}");
        }
    }
}
