use tray_icon::menu::MenuEvent;

use crate::error::Result;

/// Tray メニューのユーザー操作（desktop.md §9）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayAction {
    Open,
    ShowNotifications,
    Quit,
}

/// System Tray。`tray-icon` + `muda` の薄いラッパー。
/// イベントは `MenuEvent::receiver()` を app 層がポーリングして
/// `TrayAction` へ変換する。macOS ではメインスレッドでのみ構築可能。
pub struct AppTray {
    _icon: tray_icon::TrayIcon,
    icon_png: Vec<u8>,
    unread: std::cell::Cell<bool>,
    open_id: muda::MenuId,
    notifications_id: muda::MenuId,
    quit_id: muda::MenuId,
}

impl AppTray {
    /// `icon_png`: assets のアイコン PNG。`tooltip` は未読数等を含めてよい。
    /// メニュー構成は desktop.md §9（Quick Add は Phase 2 なので作らない）。
    pub fn new(icon_png: &[u8], tooltip: &str) -> Result<Self> {
        #[cfg(target_os = "linux")]
        if !linux_tray_host_available() {
            return Err(crate::error::Error::Other(
                "No StatusNotifier tray host is available".into(),
            ));
        }
        use muda::{Menu, MenuItem};

        let open = MenuItem::new("Open Koyori", true, None);
        let notifications = MenuItem::new("Notifications", true, None);
        let quit = MenuItem::new("Quit Koyori", true, None);

        let menu = Menu::new();
        menu.append(&open)?;
        menu.append(&notifications)?;
        menu.append(&muda::PredefinedMenuItem::separator())?;
        menu.append(&quit)?;

        let icon = decode_png_icon(icon_png)?;
        let icon = tray_icon::TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip(tooltip)
            .with_icon(icon)
            .build()?;

        Ok(Self {
            _icon: icon,
            icon_png: icon_png.to_vec(),
            unread: std::cell::Cell::new(false),
            open_id: open.id().clone(),
            notifications_id: notifications.id().clone(),
            quit_id: quit.id().clone(),
        })
    }

    /// メニューイベントを 1 件だけ取り出し `TrayAction` へ変換。
    /// 未対応イベント・空は `None`。
    pub fn poll_action(&self) -> Option<TrayAction> {
        Self::map_event(&MenuEvent::receiver().try_recv().ok()?, &self.ids())
    }

    /// イベント受信を別スレッド/タスクに移すための MenuId 3 つ組。
    /// （hot loop で Entity を borrow すると `RefCell already borrowed`
    /// になり得るため、ID だけ切り出して受信側で解決する）
    pub fn ids(&self) -> (muda::MenuId, muda::MenuId, muda::MenuId) {
        (
            self.open_id.clone(),
            self.notifications_id.clone(),
            self.quit_id.clone(),
        )
    }

    /// `ids()` と組み合わせてイベントを `TrayAction` へ変換。
    pub fn map_event(
        event: &MenuEvent,
        (open, notifications, quit): &(muda::MenuId, muda::MenuId, muda::MenuId),
    ) -> Option<TrayAction> {
        if event.id == *open {
            Some(TrayAction::Open)
        } else if event.id == *notifications {
            Some(TrayAction::ShowNotifications)
        } else if event.id == *quit {
            Some(TrayAction::Quit)
        } else {
            None
        }
    }

    /// 未読数に応じてツールチップを更新する（desktop.md §9 のバッジ反映）。
    pub fn set_tooltip(&self, tooltip: &str) -> Result<()> {
        self._icon.set_tooltip(Some(tooltip)).map_err(Into::into)
    }

    /// 未読時のアイコン差し替え用。
    pub fn set_icon(&self, icon_png: &[u8]) -> Result<()> {
        self._icon
            .set_icon(Some(decode_png_icon(icon_png)?))
            .map_err(Into::into)
    }

    /// Put a visible unread indicator on the tray icon, not only its tooltip.
    pub fn set_unread_count(&self, count: i64) -> Result<()> {
        self.set_tooltip(&format!("Koyori · {count} unread"))?;
        let unread = count > 0;
        if self.unread.get() != unread {
            let icon = if unread {
                let mut img =
                    image::load_from_memory_with_format(&self.icon_png, image::ImageFormat::Png)?
                        .to_rgba8();
                let (w, h) = img.dimensions();
                let radius = (w.min(h) as i32 / 5).max(2);
                let cx = w as i32 - radius - 1;
                let cy = radius;
                for (x, y, pixel) in img.enumerate_pixels_mut() {
                    if (x as i32 - cx).pow(2) + (y as i32 - cy).pow(2) <= radius.pow(2) {
                        *pixel = image::Rgba([239, 68, 68, 255]);
                    }
                }
                tray_icon::Icon::from_rgba(img.into_raw(), w, h)?
            } else {
                decode_png_icon(&self.icon_png)?
            };
            self._icon.set_icon(Some(icon))?;
            self.unread.set(unread);
        }
        #[cfg(target_os = "macos")]
        {
            if let Some(main_thread) = objc2::MainThreadMarker::new() {
                let app = objc2_app_kit::NSApplication::sharedApplication(main_thread);
                let label =
                    unread.then(|| objc2_foundation::NSString::from_str(&count.to_string()));
                app.dockTile().setBadgeLabel(label.as_deref());
            }
        }
        Ok(())
    }
}

/// Tray メニューイベントを非ブロッキングで 1 件取り出す。
/// AppTray を持たない受信側（ポーリングタスク）から使う。
pub fn poll_menu_event() -> Option<MenuEvent> {
    MenuEvent::receiver().try_recv().ok()
}

fn decode_png_icon(png: &[u8]) -> Result<tray_icon::Icon> {
    let img = image::load_from_memory_with_format(png, image::ImageFormat::Png)?.to_rgba8();
    let (w, h) = img.dimensions();
    Ok(tray_icon::Icon::from_rgba(img.into_raw(), w, h)?)
}

/// A successful D-Bus item registration alone does not imply a visible tray
/// (for example GNOME without its extension). Do not hide the only window then.
#[cfg(target_os = "linux")]
fn linux_tray_host_available() -> bool {
    let check = || -> zbus::Result<bool> {
        let connection = zbus::blocking::connection::Builder::session()?
            .method_timeout(std::time::Duration::from_secs(2))
            .build()?;
        let watcher = zbus::blocking::Proxy::new(
            &connection,
            "org.kde.StatusNotifierWatcher",
            "/StatusNotifierWatcher",
            "org.kde.StatusNotifierWatcher",
        )?;
        watcher.get_property("IsStatusNotifierHostRegistered")
    };
    check().unwrap_or(false)
}
