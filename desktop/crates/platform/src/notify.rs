use crate::error::Result;

/// OS ネイティブ通知 1 件分の表示内容。
/// 文言の組み立て（notification_type + payload → title/body）は core が担う
/// （desktop.md §10）。ここは表示だけ。
pub struct OsNotification {
    pub title: String,
    pub body: String,
}

type ActivationCallback = std::sync::Arc<dyn Fn() + Send + Sync>;

/// Windows は WinRT toast（AUMID 必須）、macOS/Linux は notify-rust。
/// クリックは OS の callback から呼び出し側へ渡す。UI はそのイベントを
/// UI スレッドで取り出して対象へ遷移する。
pub struct Notifier {
    /// Windows: AUMID。それ以外: アプリ名。
    app_id: String,
}

impl Notifier {
    pub fn new(app_id: &str) -> Self {
        #[cfg(windows)]
        let app_id = current_application_id().unwrap_or_else(|| app_id.to_owned());
        Self {
            app_id: app_id.to_string(),
        }
    }

    #[cfg(windows)]
    pub fn show(&self, n: &OsNotification) -> Result<()> {
        self.show_inner(n, None)
    }

    pub fn show_with_activation(
        &self,
        n: &OsNotification,
        activated: impl Fn() + Send + Sync + 'static,
    ) -> Result<()> {
        self.show_inner(n, Some(std::sync::Arc::new(activated)))
    }

    #[cfg(windows)]
    fn show_inner(&self, n: &OsNotification, activated: Option<ActivationCallback>) -> Result<()> {
        tauri_winrt_notification::Toast::new(&self.app_id)
            .title(&n.title)
            .text1(&n.body)
            .on_activated(move |_| {
                if let Some(callback) = &activated {
                    callback();
                }
                Ok(())
            })
            .show()
            .map_err(|e| crate::error::Error::Notification(e.to_string()))
    }

    #[cfg(not(windows))]
    pub fn show(&self, n: &OsNotification) -> Result<()> {
        self.show_inner(n, None)
    }

    #[cfg(not(windows))]
    fn show_inner(&self, n: &OsNotification, activated: Option<ActivationCallback>) -> Result<()> {
        let handle = notify_rust::Notification::new()
            .appname(&self.app_id)
            .summary(&n.title)
            .body(&n.body)
            .action("default", "Open Koyori")
            .show()
            .map_err(|e| crate::error::Error::Notification(e.to_string()))?;
        if let Some(callback) = activated {
            std::thread::spawn(move || {
                handle.wait_for_action(|action| {
                    if action == "default" {
                        callback();
                    }
                });
            });
        }
        Ok(())
    }
}

/// MSIX assigns <PackageFamilyName>!<ApplicationId>, which differs between
/// signed publishers. Resolve it at runtime and retain the dev fallback when
/// the executable is launched unpackaged.
#[cfg(windows)]
fn current_application_id() -> Option<String> {
    use windows::{
        Win32::{
            Foundation::{ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS},
            Storage::Packaging::Appx::GetCurrentApplicationUserModelId,
        },
        core::PWSTR,
    };
    let mut length = 0;
    unsafe {
        if GetCurrentApplicationUserModelId(&mut length, None) != ERROR_INSUFFICIENT_BUFFER {
            return None;
        }
        let mut buffer = vec![0u16; length as usize];
        if GetCurrentApplicationUserModelId(&mut length, Some(PWSTR(buffer.as_mut_ptr())))
            != ERROR_SUCCESS
        {
            return None;
        }
        let length = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
        String::from_utf16(&buffer[..length]).ok()
    }
}
