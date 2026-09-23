use crate::error::Result;

/// OS ネイティブ通知 1 件分の表示内容。
/// 文言の組み立て（notification_type + payload → title/body）は core が担う
/// （desktop.md §10）。ここは表示だけ。
pub struct OsNotification {
    pub title: String,
    pub body: String,
}

/// Windows は WinRT toast（AUMID 必須）、macOS/Linux は notify-rust。
/// 通知クリックでアプリに戻る deep link は各 OS で仕組みが違うため、MVP は
/// 通知表示 + アプリ側ポーリング復帰に留める（クリック遷移は app 層の責務）。
pub struct Notifier {
    /// Windows: AUMID。それ以外: アプリ名。
    app_id: String,
}

impl Notifier {
    pub fn new(app_id: &str) -> Self {
        Self {
            app_id: app_id.to_string(),
        }
    }

    #[cfg(windows)]
    pub fn show(&self, n: &OsNotification) -> Result<()> {
        tauri_winrt_notification::Toast::new(&self.app_id)
            .title(&n.title)
            .text1(&n.body)
            .show()
            .map_err(|e| crate::error::Error::Notification(e.to_string()))
    }

    #[cfg(not(windows))]
    pub fn show(&self, n: &OsNotification) -> Result<()> {
        notify_rust::Notification::new()
            .appname(&self.app_id)
            .summary(&n.title)
            .body(&n.body)
            .show()
            .map(|_| ())
            .map_err(|e| crate::error::Error::Notification(e.to_string()))
    }
}
