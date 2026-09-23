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
    open_id: muda::MenuId,
    notifications_id: muda::MenuId,
    quit_id: muda::MenuId,
}

impl AppTray {
    /// `icon_png`: assets のアイコン PNG。`tooltip` は未読数等を含めてよい。
    /// メニュー構成は desktop.md §9（Quick Add は Phase 2 なので作らない）。
    pub fn new(icon_png: &[u8], tooltip: &str) -> Result<Self> {
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
            open_id: open.id().clone(),
            notifications_id: notifications.id().clone(),
            quit_id: quit.id().clone(),
        })
    }

    /// メニューイベントを 1 件だけ取り出し `TrayAction` へ変換。
    /// 未対応イベント・空は `None`。
    pub fn poll_action(&self) -> Option<TrayAction> {
        let event = MenuEvent::receiver().try_recv().ok()?;
        if event.id == self.open_id {
            Some(TrayAction::Open)
        } else if event.id == self.notifications_id {
            Some(TrayAction::ShowNotifications)
        } else if event.id == self.quit_id {
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
}

fn decode_png_icon(png: &[u8]) -> Result<tray_icon::Icon> {
    let img = image::load_from_memory_with_format(png, image::ImageFormat::Png)?.to_rgba8();
    let (w, h) = img.dimensions();
    Ok(tray_icon::Icon::from_rgba(img.into_raw(), w, h)?)
}
