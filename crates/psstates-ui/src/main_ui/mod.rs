use std::path::PathBuf;
use std::sync::Arc;

use psstates_core::model::FixedParam;
use psstates_core::{paths, MoveDirection, Store, Theme};
use psstates_service::{NoticeCursor, Service, Status};

use crate::data::DataCache;
use crate::widgets::{AvatarPicker, FixedEditor, ListEditor};
use crate::VrControl;

mod actions;
mod dialogs;
mod header;
mod profiles;
mod settings;
mod toasts;

use toasts::Toast;

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
    DeleteLegacy(PathBuf),
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
    DeleteLegacy(PathBuf),
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
            edit_mode: false,
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
}
