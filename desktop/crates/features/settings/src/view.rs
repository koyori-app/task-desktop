//! §22 Settings。ローカル設定の編集 + Account（devices / logout）。
//! 変更は `SettingsEvent::Changed` で app 層へ通知し、app が保存・
//! テーマ適用などを行う。Token はここに置かない（credential store）。

use core::settings::Appearance;
use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::component::Theme;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

/// app 層への通知。
pub enum SettingsEvent {
    /// ローカル設定が変わった（保存・テーマ適用は app 側）。
    Changed(core::Settings),
    /// ログアウト完了（client 破棄と未ログイン画面への遷移は app 側）。
    LoggedOut,
}

const KEY_PALETTE: &str = "command_palette";
const KEY_SEARCH: &str = "quick_search";

/// この端末が auth 時に使った device 名。`PendingAuth::start` に渡す
/// 名前と同じ規則で、devices 一覧から自分を突き合わせる（§17）。
fn device_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "desktop".into())
}

pub struct SettingsView {
    store: core::SettingsStore,
    settings: core::Settings,
    client: Option<api::Client>,
    autolaunch: Option<::platform::AutoLaunchHandle>,
    devices: Vec<api::spec::Device>,
    palette_key: Entity<InputState>,
    search_key: Entity<InputState>,
    notice: Option<SharedString>,
}

impl EventEmitter<SettingsEvent> for SettingsView {}

impl SettingsView {
    pub fn new(
        settings: core::Settings,
        store: core::SettingsStore,
        client: Option<api::Client>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let palette_key = cx.new(|cx| {
            InputState::new(window, cx).default_value(key_or(&settings, KEY_PALETTE, "ctrl-k"))
        });
        let search_key = cx.new(|cx| {
            InputState::new(window, cx).default_value(key_or(&settings, KEY_SEARCH, "ctrl-p"))
        });

        // Launch at Login の実状態を OS 側から初期値に使う。
        let autolaunch = ::platform::AutoLaunchHandle::new("Koyori").ok();
        let mut settings = settings;
        if let Some(h) = &autolaunch {
            if let Ok(enabled) = h.is_enabled() {
                settings.launch_at_login = enabled;
            }
        }

        let this = Self {
            store,
            settings,
            client: client.clone(),
            autolaunch,
            devices: vec![],
            palette_key,
            search_key,
            notice: None,
        };
        if let Some(client) = client {
            this.refresh_devices(&client, cx);
        }
        this
    }

    /// 設定を変更 → 保存 → app へ通知。
    fn mutate(&mut self, f: impl FnOnce(&mut core::Settings), cx: &mut Context<Self>) {
        f(&mut self.settings);
        let _ = self.store.save(&self.settings);
        cx.emit(SettingsEvent::Changed(self.settings.clone()));
        cx.notify();
    }

    fn refresh_devices(&self, client: &api::Client, cx: &mut Context<Self>) {
        let client = client.clone();
        cx.spawn(async move |this, cx| {
            if let Ok(list) = client.list_devices().await {
                let _ = this.update(cx, |s, cx| {
                    s.devices = list.devices;
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn revoke_device(&mut self, id: uuid::Uuid, cx: &mut Context<Self>) {
        let Some(client) = self.client.clone() else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let ok = client.delete_device(id).await.is_ok();
            let _ = this.update(cx, |s, cx| {
                if ok {
                    s.devices.retain(|d| d.id != id);
                    s.notice = Some("Device removed".into());
                } else {
                    s.notice = Some("Failed to remove device".into());
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// §6 Logout: 自分の device を DELETE → credential 削除 → LoggedOut。
    /// 自分が一覧で特定できなくてもローカル token は確実に消す。
    fn logout(&mut self, cx: &mut Context<Self>) {
        let api_base = self.settings.api_base.clone();
        let own = self.own_device_id();
        cx.spawn(async move |this, cx| {
            if let Some(id) = own {
                let _ = core::auth::logout(&api_base, id).await;
            } else {
                let _ = ::platform::CredentialStore::new(::platform::CREDENTIAL_SERVICE)
                    .delete(::platform::CREDENTIAL_ACCOUNT_TOKEN);
            }
            let _ = this.update(cx, |s, cx| {
                s.client = None;
                s.devices.clear();
                cx.emit(SettingsEvent::LoggedOut);
                cx.notify();
            });
        })
        .detach();
    }

    fn own_device_id(&self) -> Option<uuid::Uuid> {
        let name = device_name();
        self.devices.iter().find(|d| d.name == name).map(|d| d.id)
    }

    fn save_keybindings(&mut self, cx: &mut Context<Self>) {
        let palette = self.palette_key.read(cx).value().trim().to_string();
        let search = self.search_key.read(cx).value().trim().to_string();
        if palette.is_empty() || search.is_empty() {
            self.notice = Some("Keybinding must not be empty".into());
            cx.notify();
            return;
        }
        self.mutate(
            |s| {
                s.keybindings.insert(KEY_PALETTE.into(), palette.clone());
                s.keybindings.insert(KEY_SEARCH.into(), search.clone());
            },
            cx,
        );
        self.notice = Some("Saved. Takes effect after restart.".into());
        cx.notify();
    }

    // ---- render helpers ----

    fn section(&self, title: &'static str, children: Vec<AnyElement>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
            .children(children)
            .into_any_element()
    }

    fn toggle(
        &self,
        id: &'static str,
        label: &'static str,
        checked: bool,
        cx: &mut Context<Self>,
        f: impl Fn(&mut core::Settings, bool) + 'static,
    ) -> AnyElement {
        let weak = cx.entity().downgrade();
        Checkbox::new(id)
            .label(label)
            .checked(checked)
            .on_click(move |v, _, cx| {
                let _ = weak.update(cx, |this, cx| {
                    this.mutate(|s| f(s, *v), cx);
                });
            })
            .into_any_element()
    }
}

fn key_or(settings: &core::Settings, key: &str, default: &str) -> String {
    settings
        .keybindings
        .get(key)
        .cloned()
        .unwrap_or_else(|| default.into())
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (muted_fg, accent) = {
            let t = Theme::global(cx);
            (t.muted_foreground, t.accent)
        };
        let s = &self.settings;

        // General
        let launch_toggle = {
            let weak = cx.entity().downgrade();
            let checked = s.launch_at_login;
            Checkbox::new("launch-at-login")
                .label("Launch at Login")
                .checked(checked)
                .on_click(move |v, _, cx| {
                    let _ = weak.update(cx, |this, cx| {
                        if let Some(h) = &this.autolaunch {
                            let _ = if *v { h.enable() } else { h.disable() };
                        }
                        this.mutate(|s| s.launch_at_login = *v, cx);
                    });
                })
                .into_any_element()
        };
        let general = self.section(
            "General",
            vec![
                launch_toggle,
                self.toggle(
                    "keep-bg",
                    "Keep Running in Background",
                    s.keep_running_in_background,
                    cx,
                    |s, v| s.keep_running_in_background = v,
                ),
            ],
        );

        // Notifications（§22: ローカルの OS 通知 ON/OFF のみ。プロジェクト毎は Web）
        let web = s.web_base.clone();
        let notifications = self.section(
            "Notifications",
            vec![
                self.toggle(
                    "notif-enabled",
                    "Enable Desktop Notifications",
                    s.notifications.enabled,
                    cx,
                    |s, v| s.notifications.enabled = v,
                ),
                self.toggle("notif-task", "Task", s.notifications.task, cx, |s, v| {
                    s.notifications.task = v
                }),
                self.toggle(
                    "notif-review",
                    "Review",
                    s.notifications.review,
                    cx,
                    |s, v| s.notifications.review = v,
                ),
                self.toggle(
                    "notif-due",
                    "Due Date",
                    s.notifications.due_date,
                    cx,
                    |s, v| s.notifications.due_date = v,
                ),
                Button::new("notif-web-link")
                    .ghost()
                    .label("Per-project notification settings (Web)")
                    .icon(Icon::new(IconName::ExternalLink))
                    .on_click(move |_, _, _| {
                        let _ = ::platform::open_url(&format!("{web}/settings/notifications"));
                    })
                    .into_any_element(),
            ],
        );

        // Appearance
        let mk_appearance = |label: &'static str, value: Appearance, cx: &mut Context<Self>| {
            let active = s.appearance == value;
            Button::new(SharedString::from(format!("appearance-{label}")))
                .when(active, |b| b.primary())
                .when(!active, |b| b.outline())
                .label(label)
                .on_click(
                    cx.listener(move |this, _, _, cx| this.mutate(|s| s.appearance = value, cx)),
                )
                .into_any_element()
        };
        let appearance = self.section(
            "Appearance",
            vec![
                div()
                    .flex()
                    .flex_row()
                    .gap_2()
                    .child(mk_appearance("Light", Appearance::Light, cx))
                    .child(mk_appearance("Dark", Appearance::Dark, cx))
                    .child(mk_appearance("System", Appearance::System, cx))
                    .into_any_element(),
            ],
        );

        // Keyboard
        let keyboard = self.section(
            "Keyboard",
            vec![
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(div().w(px(160.)).child("Command Palette"))
                    .child(Input::new(&self.palette_key).w(px(200.)))
                    .into_any_element(),
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(div().w(px(160.)).child("Quick Search"))
                    .child(Input::new(&self.search_key).w(px(200.)))
                    .into_any_element(),
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new("save-keybindings")
                            .outline()
                            .label("Apply")
                            .on_click(cx.listener(|this, _, _, cx| this.save_keybindings(cx))),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted_fg)
                            .child("Changes take effect after restart"),
                    )
                    .into_any_element(),
            ],
        );

        // Account
        let name = device_name();
        let device_rows: Vec<AnyElement> = self
            .devices
            .iter()
            .map(|d| {
                let own = d.name == name;
                let id = d.id;
                let last = d
                    .last_used_at
                    .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
                    .unwrap_or_else(|| "—".into());
                let weak = cx.entity().downgrade();
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_3()
                    .child(div().w(px(200.)).child(format!(
                        "{}{}",
                        d.name,
                        if own { " (this device)" } else { "" }
                    )))
                    .child(div().w(px(150.)).child(format!("last used {last}")))
                    .child(
                        Button::new(SharedString::from(format!("revoke-{id}")))
                            .ghost()
                            .label("Revoke")
                            .on_click(move |_, _, cx| {
                                let _ = weak.update(cx, |this, cx| this.revoke_device(id, cx));
                            }),
                    )
                    .into_any_element()
            })
            .collect();
        let mut account_children = vec![
            div()
                .text_sm()
                .text_color(muted_fg)
                .child(format!("API: {}", s.api_base))
                .into_any_element(),
            div()
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .child("Devices")
                .into_any_element(),
        ];
        if device_rows.is_empty() {
            account_children.push(
                div()
                    .text_sm()
                    .text_color(muted_fg)
                    .child("No devices")
                    .into_any_element(),
            );
        } else {
            account_children.extend(device_rows);
        }
        account_children.push(
            Button::new("logout")
                .outline()
                .label("Logout")
                .icon(Icon::new(IconName::LogOut))
                .on_click(cx.listener(|this, _, _, cx| this.logout(cx)))
                .into_any_element(),
        );
        let account = self.section("Account", account_children);

        div()
            .flex_1()
            .h_full()
            .p_6()
            .flex()
            .flex_col()
            .gap_6()
            .child(
                div()
                    .text_xl()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Settings"),
            )
            .when_some(self.notice.clone(), |d, n| {
                d.child(div().text_sm().text_color(accent).child(n))
            })
            .child(general)
            .child(notifications)
            .child(appearance)
            .child(keyboard)
            .child(account)
    }
}
