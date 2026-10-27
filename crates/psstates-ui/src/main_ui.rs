use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{Align2, RichText, Ui};
use psstates_core::backup::{default_backup_file_name, export_backup, import_backup};
use psstates_core::legacy::{looks_like_legacy_dir, migrate_legacy_dir};
use psstates_core::model::FixedParam;
use psstates_core::rules::normalize_fixed_parameters;
use psstates_core::{paths, CoreError, MoveDirection, Store, Theme};
use psstates_service::{NoticeCursor, NoticeLevel, Service, Status, VERSION};

use crate::data::DataCache;
use crate::widgets::{card, hint, AvatarPicker, FixedEditor, ListEditor, AMBER, GREEN, RED};
use crate::VrControl;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Profiles,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettingsTab {
    General,
    Filters,
    AutoSync,
    Fixed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ListKind {
    Excluded,
    AutoSync,
}

#[derive(Debug, Clone)]
enum Dialog {
    Override(String),
    Delete(String),
    Rename { name: String, new_name: String },
    CopyPrevious,
    CopyFromAvatar { id: String, label: String },
    DeleteAvatar { id: String, label: String },
    Import(PathBuf),
    LegacyPrompt(PathBuf),
}

#[derive(Debug, Clone)]
enum Action {
    SaveNew,
    Apply(String),
    Override(String),
    Delete(String),
    Rename(String, String),
    Move(String, MoveDirection),
    CopyPrevious,
    CopyFromAvatar(String),
    DeleteAvatar(String),
    Export,
    PickImport,
    Import(PathBuf),
    PickLegacyDir,
    MigrateLegacy(PathBuf),
    SkipLegacy,
    SetTheme(Theme),
    Reconnect,
    SaveList {
        kind: ListKind,
        avatar: Option<String>,
        items: Vec<String>,
    },
    SaveFixed {
        avatar: Option<String>,
        items: Vec<FixedParam>,
    },
    Open(Dialog),
    CloseDialog,
}

struct Toast {
    level: NoticeLevel,
    text: String,
    until: Instant,
}

#[derive(Default)]
struct ScopedLists {
    global: ListEditor,
    picker: AvatarPicker,
    avatar: ListEditor,
}

#[derive(Default)]
struct ScopedFixed {
    global: FixedEditor,
    picker: AvatarPicker,
    avatar: FixedEditor,
}

/// The complete desktop UI.
pub struct MainUi {
    service: Service,
    data: DataCache,
    status: Status,
    tab: Tab,
    settings_tab: SettingsTab,
    new_profile_name: String,
    dialog: Option<Dialog>,
    actions: Vec<Action>,
    toasts: Vec<Toast>,
    applied_theme: Option<Theme>,
    filters: ScopedLists,
    auto_sync: ScopedLists,
    fixed: ScopedFixed,
    copy_picker: AvatarPicker,
    delete_picker: AvatarPicker,
    legacy_dir: Option<PathBuf>,
    legacy_checked: bool,
    notice_cursor: NoticeCursor,
    vr: Option<Arc<dyn VrControl>>,
    overlay_mode: bool,
    dialog_opened: bool,
    /// Show rename / delete / reorder buttons in the profile list.
    edit_mode: bool,
    style_applied: bool,
}

impl MainUi {
    pub fn new(service: Service) -> Self {
        Self {
            status: service.status(),
            notice_cursor: service.notice_cursor(),
            vr: None,
            dialog_opened: false,
            overlay_mode: false,
            edit_mode: true,
            style_applied: false,
            service,
            data: DataCache::default(),
            tab: Tab::Profiles,
            settings_tab: SettingsTab::General,
            new_profile_name: String::new(),
            dialog: None,
            actions: Vec::new(),
            toasts: Vec::new(),
            applied_theme: None,
            filters: ScopedLists::default(),
            auto_sync: ScopedLists::default(),
            fixed: ScopedFixed::default(),
            copy_picker: AvatarPicker::default(),
            delete_picker: AvatarPicker::default(),
            legacy_dir: paths::legacy_profiles_dir(),
            legacy_checked: false,
        }
    }

    /// Shows SteamVR related settings backed by `vr`.
    pub fn with_vr(mut self, vr: Arc<dyn VrControl>) -> Self {
        self.vr = Some(vr);
        self
    }

    /// UI rendered inside the SteamVR dashboard: hides features that need
    /// desktop file dialogs and skips the legacy import prompt.
    pub fn overlay_mode(mut self) -> Self {
        self.overlay_mode = true;
        self.legacy_checked = true;
        // Keep the list calm for laser pointing; "Edit" reveals the extra buttons.
        self.edit_mode = false;
        self
    }

    pub fn service(&self) -> &Service {
        &self.service
    }

    /// Opens the settings tab (previews and tests).
    #[doc(hidden)]
    pub fn on_settings_tab(mut self) -> Self {
        self.tab = Tab::Settings;
        self
    }

    /// Never offers to import legacy data (previews and tests).
    #[doc(hidden)]
    pub fn without_legacy_prompt(mut self) -> Self {
        self.legacy_checked = true;
        self
    }

    /// Forces the profile list's edit mode (previews and tests).
    #[doc(hidden)]
    pub fn with_edit_mode(mut self, on: bool) -> Self {
        self.edit_mode = on;
        self
    }

    /// Call when the UI is shown in a new egui context (e.g. a re-opened window).
    pub fn context_changed(&mut self) {
        self.applied_theme = None;
        self.style_applied = false;
    }

    fn store(&self) -> &Store {
        self.service.store()
    }

    /// Renders the UI into `ctx`.
    pub fn show(&mut self, ctx: &egui::Context) {
        self.status = self.service.status();
        let store = self.service.store().clone();
        self.data.refresh(&store, &self.status);

        if !self.style_applied {
            crate::apply_style(ctx, self.overlay_mode);
            self.style_applied = true;
        }
        if self.applied_theme != Some(self.data.settings.theme) {
            crate::apply_theme(ctx, self.data.settings.theme);
            self.applied_theme = Some(self.data.settings.theme);
        }
        self.check_legacy_prompt();
        self.collect_notices();

        egui::TopBottomPanel::top("header").show(ctx, |ui| self.header(ui));
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| match self.tab {
                    Tab::Profiles => self.profiles_page(ui),
                    Tab::Settings => self.settings_page(ui),
                });
        });
        self.dialog_ui(ctx);
        self.toasts_ui(ctx);

        for action in std::mem::take(&mut self.actions) {
            self.handle(action);
        }
    }

    // ------------------------------------------------------------ chrome

    fn header(&mut self, ui: &mut Ui) {
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

    fn connection_badge(&self, ui: &mut Ui) {
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

    fn collect_notices(&mut self) {
        let now = Instant::now();
        for n in self.service.notices_since(&mut self.notice_cursor) {
            let secs = if n.level == NoticeLevel::Error { 8 } else { 4 };
            self.toasts.push(Toast {
                level: n.level,
                text: n.text,
                until: now + Duration::from_secs(secs),
            });
        }
        self.toasts.retain(|t| t.until > now);
        if self.toasts.len() > 5 {
            let excess = self.toasts.len() - 5;
            self.toasts.drain(..excess);
        }
    }

    fn toasts_ui(&mut self, ctx: &egui::Context) {
        if self.toasts.is_empty() {
            return;
        }
        egui::Area::new(egui::Id::new("toasts"))
            .anchor(Align2::RIGHT_BOTTOM, egui::vec2(-12.0, -12.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                for t in &self.toasts {
                    let color = match t.level {
                        NoticeLevel::Error => RED,
                        NoticeLevel::Success => GREEN,
                        NoticeLevel::Info => ui.visuals().text_color(),
                    };
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_max_width(420.0);
                        ui.colored_label(color, &t.text);
                    });
                    ui.add_space(4.0);
                }
            });
        ctx.request_repaint_after(Duration::from_millis(500));
    }

    fn report<T>(&self, result: Result<T, CoreError>, ok: impl FnOnce(T) -> String) {
        match result {
            Ok(v) => self.service.notice(NoticeLevel::Success, ok(v)),
            Err(e) => self.service.notice(NoticeLevel::Error, e.to_string()),
        }
        self.service.notify_data_changed();
    }

    // ---------------------------------------------------------- profiles

    fn profiles_page(&mut self, ui: &mut Ui) {
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
    fn status_bar(&mut self, ui: &mut Ui, status: &Status) {
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

    // ---------------------------------------------------------- settings

    fn settings_page(&mut self, ui: &mut Ui) {
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

    fn general_settings(&mut self, ui: &mut Ui) {
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
                    if ui.button("Import old profiles").clicked() {
                        self.actions.push(Action::MigrateLegacy(dir.clone()));
                    }
                }
                None => {
                    ui.label(RichText::new("No old data found automatically.").weak());
                }
            }
            if ui.button("Choose 'Profiles' folder...").clicked() {
                self.actions.push(Action::PickLegacyDir);
            }
            hint(
                ui,
                "Existing profiles are kept; imported profiles with the same name get a '(legacy)' suffix.",
            );
        });
        ui.add_space(8.0);
        self.about_card(ui);
    }

    fn vr_card(&mut self, ui: &mut Ui, vr: &dyn VrControl) {
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

    fn about_card(&mut self, ui: &mut Ui) {
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

    fn list_settings(&mut self, ui: &mut Ui, kind: ListKind) {
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

    fn fixed_settings(&mut self, ui: &mut Ui) {
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

    // ----------------------------------------------------------- dialogs

    fn check_legacy_prompt(&mut self) {
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

    fn dialog_ui(&mut self, ctx: &egui::Context) {
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

    // ----------------------------------------------------------- actions

    fn current_avatar(&self) -> Option<String> {
        self.status.current_avatar.as_ref().map(|a| a.id.clone())
    }

    fn with_current<T>(
        &self,
        f: impl FnOnce(&Store, &str) -> Result<T, CoreError>,
        ok: impl FnOnce(T) -> String,
    ) {
        match self.current_avatar() {
            Some(id) => self.report(f(self.store(), &id), ok),
            None => self
                .service
                .notice(NoticeLevel::Error, "No current avatar detected"),
        }
    }

    fn handle(&mut self, action: Action) {
        // Every action except opening a dialog closes the current one.
        if !matches!(action, Action::Open(_)) {
            self.dialog = None;
        }
        match action {
            Action::Open(d) => {
                self.dialog = Some(d);
                self.dialog_opened = true;
            }
            Action::CloseDialog => {}
            Action::SaveNew => {
                let name = std::mem::take(&mut self.new_profile_name);
                let name = Some(name.trim().to_string()).filter(|n| !n.is_empty());
                self.service.save_profile(name);
            }
            Action::Apply(name) => self.service.apply_profile(name),
            Action::Override(name) => self.service.override_profile(name),
            Action::Delete(name) => self.with_current(
                |s, id| s.update_avatar(id, |a| a.delete_profile(&name)),
                |p| format!("Deleted profile '{}'", p.name),
            ),
            Action::Rename(name, new_name) => self.with_current(
                |s, id| s.update_avatar(id, |a| a.rename_profile(&name, &new_name)),
                |n| format!("Renamed to '{n}'"),
            ),
            Action::Move(name, dir) => {
                if let Some(id) = self.current_avatar() {
                    if let Err(e) = self
                        .store()
                        .update_avatar(&id, |a| a.move_profile(&name, dir))
                    {
                        self.service.notice(NoticeLevel::Error, e.to_string());
                    }
                    self.service.notify_data_changed();
                }
            }
            Action::CopyPrevious => self.service.copy_from_previous(),
            Action::CopyFromAvatar(id) => self.service.copy_from_avatar(id),
            Action::DeleteAvatar(id) => {
                let r = self.store().delete_avatar(&id);
                self.report(r, |_| "Deleted avatar data".to_string());
            }
            Action::Export => {
                let path = {
                    let _rt = self.service.runtime().enter();
                    rfd::FileDialog::new()
                        .set_title("Export backup")
                        .set_file_name(default_backup_file_name())
                        .add_filter("Zip archive", &["zip"])
                        .save_file()
                };
                if let Some(path) = path {
                    let r = export_backup(self.store(), &path);
                    self.report(r, |n| format!("Exported {n} file(s) to {}", path.display()));
                }
            }
            Action::PickImport => {
                let path = {
                    let _rt = self.service.runtime().enter();
                    rfd::FileDialog::new()
                        .set_title("Import backup")
                        .add_filter("Zip archive", &["zip"])
                        .pick_file()
                };
                if let Some(path) = path {
                    self.dialog = Some(Dialog::Import(path));
                }
            }
            Action::Import(path) => {
                let r = import_backup(self.store(), &path);
                self.reset_editors();
                self.report(r, |rep| rep.summary());
            }
            Action::PickLegacyDir => {
                let path = {
                    let _rt = self.service.runtime().enter();
                    rfd::FileDialog::new()
                        .set_title("Select the old 'Profiles' folder")
                        .pick_folder()
                };
                if let Some(path) = path {
                    if looks_like_legacy_dir(&path) {
                        self.handle(Action::MigrateLegacy(path));
                    } else {
                        self.service.notice(
                            NoticeLevel::Error,
                            "The selected folder does not look like an old 'Profiles' folder",
                        );
                    }
                }
            }
            Action::MigrateLegacy(path) => {
                let r = migrate_legacy_dir(self.store(), &path);
                let _ = self.store().update_settings(|s| {
                    s.legacy_import_prompted = true;
                    Ok(())
                });
                self.reset_editors();
                self.report(r, |rep| rep.summary());
            }
            Action::SkipLegacy => {
                let r = self.store().update_settings(|s| {
                    s.legacy_import_prompted = true;
                    Ok(())
                });
                if let Err(e) = r {
                    self.service.notice(NoticeLevel::Error, e.to_string());
                }
                self.service.notify_data_changed();
            }
            Action::SetTheme(theme) => {
                let r = self.store().update_settings(|s| {
                    s.theme = theme;
                    Ok(())
                });
                if let Err(e) = r {
                    self.service.notice(NoticeLevel::Error, e.to_string());
                }
                self.service.notify_data_changed();
            }
            Action::Reconnect => self.service.reconnect(),
            Action::SaveList {
                kind,
                avatar,
                items,
            } => {
                let r = match &avatar {
                    None => self.store().update_settings(|s| {
                        match kind {
                            ListKind::Excluded => s.excluded_parameters = items,
                            ListKind::AutoSync => s.auto_sync_parameters = items,
                        }
                        Ok(())
                    }),
                    Some(id) => self.store().update_avatar(id, |a| {
                        match kind {
                            ListKind::Excluded => a.excluded_parameters = items,
                            ListKind::AutoSync => a.auto_sync_parameters = items,
                        }
                        Ok(())
                    }),
                };
                self.report(r, |_| "Saved".to_string());
            }
            Action::SaveFixed { avatar, items } => {
                let r = normalize_fixed_parameters(items).and_then(|items| match &avatar {
                    None => self.store().update_settings(|s| {
                        s.fixed_parameters = items;
                        Ok(())
                    }),
                    Some(id) => self.store().update_avatar(id, |a| {
                        a.fixed_parameters = items;
                        Ok(())
                    }),
                });
                self.report(r, |_| "Saved fixed parameters".to_string());
            }
        }
    }

    fn reset_editors(&mut self) {
        self.filters = ScopedLists::default();
        self.auto_sync = ScopedLists::default();
        self.fixed = ScopedFixed::default();
        self.data.invalidate();
    }
}
