//! ローカル設定（desktop.md §22）。保存先は OS の設定ディレクトリの
//! `settings.json`。Token 類はここに置かない（credential store へ、§6）。

use std::{collections::BTreeMap, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

const FILE_NAME: &str = "settings.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

/// OS 通知の ON/OFF。OFF でも Notification Center には出る（§10）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NotificationPrefs {
    /// マスタースイッチ（Enable Desktop Notifications）。
    pub enabled: bool,
    pub task: bool,
    pub review: bool,
    /// Phase 2（Due Date 通知）。
    pub due_date: bool,
}

impl Default for NotificationPrefs {
    fn default() -> Self {
        Self {
            enabled: true,
            task: true,
            review: true,
            due_date: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// `https://task.koyori.app/api` 形式。
    pub api_base: String,
    /// authorize ページを開く Web 側オリジン。
    pub web_base: String,
    pub appearance: Appearance,
    pub launch_at_login: bool,
    /// ON なら Window を閉じても常駐する（§8）。Tray 非対応環境では無効化。
    pub keep_running_in_background: bool,
    pub notifications: NotificationPrefs,
    /// キーバインド上書き（§22 Keyboard）。値は action 名 → キー列。
    pub keybindings: BTreeMap<String, String>,
    /// 前回選択していたテナント（起動時の復元用）。
    pub last_tenant_id: Option<uuid::Uuid>,
    /// 通知 catch-up の高水位カーソル（§7.1 / task.md §10）。
    pub notification_cursor: Option<String>,
    /// Dock / Resizable のレイアウト状態（§14）。中身は app 層が決める。
    pub window_layout: Option<serde_json::Value>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            api_base: "https://task.koyori.app/api".into(),
            web_base: "https://task.koyori.app".into(),
            appearance: Appearance::System,
            launch_at_login: false,
            keep_running_in_background: true,
            notifications: NotificationPrefs::default(),
            keybindings: BTreeMap::new(),
            last_tenant_id: None,
            notification_cursor: None,
            window_layout: None,
        }
    }
}

/// `settings.json` の読み書き。書き込みは tmp → rename で中途半端な
/// ファイルを残さない。
pub struct SettingsStore {
    path: PathBuf,
}

impl SettingsStore {
    /// 規定の設定ディレクトリ（Windows: %APPDATA%\koyori\Koyori 等）。
    pub fn default_location() -> Result<Self> {
        let dirs = directories::ProjectDirs::from("app", "koyori", "Koyori")
            .ok_or_else(|| Error::InvalidConfig("cannot resolve config dir".into()))?;
        Ok(Self::at(dirs.config_dir().join(FILE_NAME)))
    }

    pub fn at(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// 無い・壊れている場合は既定値（設定失いで起動不能にしない）。
    pub fn load(&self) -> Settings {
        std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, settings: &Settings) -> Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(settings)?)?;
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let dir = std::env::temp_dir().join(format!("koyori-test-{}", uuid::Uuid::new_v4()));
        let store = SettingsStore::at(dir.join("settings.json"));
        let mut s = Settings::default();
        s.notifications.review = false;
        s.notification_cursor = Some("abc".into());
        store.save(&s).unwrap();
        let loaded = store.load();
        assert!(!loaded.notifications.review);
        assert_eq!(loaded.notification_cursor.as_deref(), Some("abc"));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn missing_file_yields_default() {
        let store = SettingsStore::at(std::env::temp_dir().join("nope-xyz/settings.json"));
        assert_eq!(store.load().api_base, Settings::default().api_base);
    }

    #[test]
    fn broken_file_yields_default() {
        let dir = std::env::temp_dir().join(format!("koyori-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("settings.json");
        std::fs::write(&p, "{ not json").unwrap();
        let store = SettingsStore::at(p);
        let s = store.load();
        assert!(s.notifications.enabled);
        std::fs::remove_dir_all(dir).ok();
    }
}
