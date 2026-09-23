//! OS 連携ラッパー。feature / app crate は OS 依存 crate を直接触らずここを通す
//! （desktop.md §5）。薄いラッパーに留め、ビジネスロジックは置かない。

mod autolaunch;
mod browser;
mod credential;
mod error;
mod hotkey;
mod notify;
mod tray;

pub use autolaunch::AutoLaunchHandle;
pub use browser::open_url;
pub use credential::CredentialStore;
pub use error::{Error, Result};
pub use hotkey::{GlobalHotKey, HotKeyHandle, HotKeyManager};
pub use notify::{Notifier, OsNotification};
pub use tray::{AppTray, TrayAction, poll_menu_event};

/// Device Token を入れる credential store の service 名（desktop.md §6）。
pub const CREDENTIAL_SERVICE: &str = "app.koyori.desktop";
/// Device Token の account 名。複数アカウント対応時はキーを分ける。
pub const CREDENTIAL_ACCOUNT_TOKEN: &str = "device-token";
/// Windows toast の Application User Model ID。MSIX インストーラが Start Menu
/// へ登録する値と一致させる（desktop.md §4 Windows / §10）。配布前は
/// `app.koyori.desktop` を AUMID として使い、登録が無い環境では通知が出ない。
pub const WINDOWS_AUMID: &str = "app.koyori.desktop";
