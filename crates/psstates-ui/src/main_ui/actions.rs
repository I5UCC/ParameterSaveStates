//! Executes the actions collected while drawing a frame.

use psstates_core::backup::{default_backup_file_name, export_backup, import_backup};
use psstates_core::legacy::{delete_legacy_dir, looks_like_legacy_dir, migrate_legacy_dir};
use psstates_core::rules::normalize_fixed_parameters;
use psstates_core::{CoreError, Store};
use psstates_service::NoticeLevel;

use super::*;

impl MainUi {
    pub(super) fn report<T>(&self, result: Result<T, CoreError>, ok: impl FnOnce(T) -> String) {
        match result {
            Ok(v) => self.service.notice(NoticeLevel::Success, ok(v)),
            Err(e) => self.service.notice(NoticeLevel::Error, e.to_string()),
        }
        self.service.notify_data_changed();
    }

    pub(super) fn current_avatar(&self) -> Option<String> {
        self.status.current_avatar.as_ref().map(|a| a.id.clone())
    }

    pub(super) fn with_current<T>(
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

    pub(super) fn handle(&mut self, action: Action) {
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
            Action::DeleteLegacy(path) => {
                let r = delete_legacy_dir(&path);
                if r.is_ok() && self.legacy_dir.as_ref() == Some(&path) {
                    self.legacy_dir = None;
                }
                self.report(r, |_| "Deleted old folder".to_string());
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

    pub(super) fn reset_editors(&mut self) {
        self.filters = ScopedLists::default();
        self.auto_sync = ScopedLists::default();
        self.fixed = ScopedFixed::default();
        self.data.invalidate();
    }
}
