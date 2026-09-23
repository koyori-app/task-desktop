//! §22 Settings。ローカル設定の編集 + Account（devices / logout）。
//! 変更は `SettingsEvent::Changed` で app 層へ通知し、app が保存・
//! テーマ適用などを行う。Token はここに置かない（credential store）。

use core::settings::Appearance;
use gpui_kit::assets::IconName;
use gpui_kit::component::Theme;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::radio::RadioGroup;
use gpui_kit::component::{Disableable, Icon};
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
const DEFAULT_PALETTE: &str = if cfg!(target_os = "macos") {
    "cmd-k"
} else {
    "ctrl-k"
};
const DEFAULT_SEARCH: &str = if cfg!(target_os = "macos") {
    "cmd-p"
} else {
    "ctrl-p"
};

/// devices 一覧から自分を突き合わせる device 名（core::auth と同一規則）。
fn device_name() -> String {
    core::auth::default_device_name()
}

pub struct SettingsView {
    store: core::SettingsStore,
    settings: core::Settings,
    client: Option<api::Client>,
    autolaunch: Option<::platform::AutoLaunchHandle>,
    devices: Vec<api::spec::Device>,
    profile: Option<api::types::UserResponse>,
    profile_loading: bool,
    profile_error: Option<SharedString>,
    palette_key: Entity<InputState>,
    search_key: Entity<InputState>,
    notice: Option<SharedString>,
    device_generation: u64,
    devices_loading: bool,
    background_available: bool,
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
            InputState::new(window, cx).default_value(key_or(
                &settings,
                KEY_PALETTE,
                DEFAULT_PALETTE,
            ))
        });
        let search_key = cx.new(|cx| {
            InputState::new(window, cx).default_value(key_or(&settings, KEY_SEARCH, DEFAULT_SEARCH))
        });

        // Launch at Login の実状態を OS 側から初期値に使う。
        let autolaunch = ::platform::AutoLaunchHandle::new("Koyori").ok();
        let mut settings = settings;
        if let Some(h) = &autolaunch
            && let Ok(enabled) = h.is_enabled()
        {
            settings.launch_at_login = enabled;
        }

        let this = Self {
            store,
            settings,
            client: client.clone(),
            autolaunch,
            devices: vec![],
            profile: None,
            profile_loading: client.is_some(),
            profile_error: None,
            palette_key,
            search_key,
            notice: None,
            device_generation: 0,
            devices_loading: client.is_some(),
            background_available: true,
        };
        if let Some(client) = client {
            this.refresh_devices(&client, cx);
            this.refresh_profile(&client, cx);
        }
        this
    }

    /// ログイン後にクライアントが出来た時に差し替えて devices を再取得。
    pub fn set_client(&mut self, client: api::Client, cx: &mut Context<Self>) {
        self.device_generation += 1;
        self.devices_loading = true;
        self.profile_loading = true;
        self.profile = None;
        self.profile_error = None;
        self.client = Some(client.clone());
        self.refresh_devices(&client, cx);
        self.refresh_profile(&client, cx);
    }

    pub fn clear_client(&mut self, cx: &mut Context<Self>) {
        self.device_generation += 1;
        self.client = None;
        self.devices.clear();
        self.devices_loading = false;
        self.profile = None;
        self.profile_loading = false;
        self.profile_error = None;
        self.notice = None;
        cx.notify();
    }

    pub fn set_background_available(&mut self, available: bool, cx: &mut Context<Self>) {
        self.background_available = available;
        cx.notify();
    }

    /// 設定を変更 → 保存 → app へ通知。
    fn mutate(&mut self, f: impl FnOnce(&mut core::Settings), cx: &mut Context<Self>) {
        let mut updated = self.settings.clone();
        f(&mut updated);
        // Navigation and notification sync may have saved newer state meanwhile.
        let latest = self.store.load();
        updated.notification_cursor = latest.notification_cursor;
        updated.window_layout = latest.window_layout;
        updated.last_tenant_id = latest.last_tenant_id;
        match self.store.save(&updated) {
            Ok(()) => {
                self.settings = updated;
                self.notice = None;
                cx.emit(SettingsEvent::Changed(self.settings.clone()));
            }
            Err(error) => self.notice = Some(format!("Could not save settings: {error}").into()),
        }
        cx.notify();
    }

    fn refresh_devices(&self, client: &api::Client, cx: &mut Context<Self>) {
        let client = client.clone();
        let generation = self.device_generation;
        cx.spawn(async move |this, cx| {
            let result = client.list_devices().await;
            let _ = this.update(cx, |s, cx| {
                if s.device_generation != generation {
                    return;
                }
                s.devices_loading = false;
                match result {
                    Ok(list) => s.devices = list.devices,
                    Err(error) => s.notice = Some(error.to_string().into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn refresh_profile(&self, client: &api::Client, cx: &mut Context<Self>) {
        let client = client.clone();
        let generation = self.device_generation;
        cx.spawn(async move |this, cx| {
            let result = client.get_me().await;
            let _ = this.update(cx, |this, cx| {
                if this.device_generation != generation {
                    return;
                }
                this.profile_loading = false;
                match result {
                    Ok(profile) => this.profile = Some(profile),
                    Err(error) => this.profile_error = Some(error.to_string().into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn revoke_device(&mut self, id: uuid::Uuid, cx: &mut Context<Self>) {
        if self.own_device_id() == Some(id) {
            self.logout(cx);
            return;
        }
        let Some(client) = self.client.clone() else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let result = client.delete_device(id).await;
            let _ = this.update(cx, |s, cx| {
                match result {
                    Ok(()) => {
                        s.devices.retain(|d| d.id != id);
                        s.notice = Some("Device removed".into());
                    }
                    Err(error) => s.notice = Some(error.to_string().into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// §6 Logout: 自分の device を DELETE → credential 削除 → LoggedOut。
    /// 自分が一覧で特定できなくてもローカル token は確実に消す。
    fn logout(&mut self, cx: &mut Context<Self>) {
        let own = self.own_device_id();
        let client = self.client.clone();
        cx.spawn(async move |this, cx| {
            if let (Some(client), Some(id)) = (client, own) {
                let _ = client.delete_device(id).await;
            }
            if std::env::var_os("KOYORI_DEV_TOKEN").is_none() {
                let _ = ::platform::CredentialStore::new(::platform::CREDENTIAL_SERVICE)
                    .delete(::platform::CREDENTIAL_ACCOUNT_TOKEN);
            }
            let _ = this.update(cx, |s, cx| {
                s.clear_client(cx);
                cx.emit(SettingsEvent::LoggedOut);
                cx.notify();
            });
        })
        .detach();
    }

    fn own_device_id(&self) -> Option<uuid::Uuid> {
        let name = device_name();
        let mut matches = self.devices.iter().filter(|d| d.name == name);
        let first = matches.next()?;
        // Until the backend supplies an explicit current-device identity, do
        // not revoke an arbitrary device when several share this hostname.
        matches.next().is_none().then_some(first.id)
    }

    fn save_keybindings(&mut self, cx: &mut Context<Self>) {
        let palette = self.palette_key.read(cx).value().trim().to_string();
        let search = self.search_key.read(cx).value().trim().to_string();
        if palette.is_empty() || search.is_empty() {
            self.notice = Some("Keybinding must not be empty".into());
            cx.notify();
            return;
        }
        let valid = |key: &str| {
            key.split_whitespace()
                .all(|part| Keystroke::parse(part).is_ok())
        };
        if !valid(&palette) || !valid(&search) || palette == search {
            self.notice =
                Some("Use two different valid shortcuts, such as ctrl-k and ctrl-p".into());
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
        if self.notice.is_none() {
            self.notice = Some("Saved. Takes effect after restart.".into());
        }
        cx.notify();
    }

    // ---- render helpers ----

    fn section(&self, title: &'static str, children: Vec<AnyElement>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .min_w_0()
            .w_full()
            .items_start()
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
            .disabled(id == "keep-bg" && !self.background_available)
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
            (t.muted_foreground, t.primary)
        };
        let s = &self.settings;

        // General
        let launch_toggle = {
            let weak = cx.entity().downgrade();
            let checked = s.launch_at_login;
            Checkbox::new("launch-at-login")
                .label("Launch at Login")
                .checked(checked)
                .disabled(self.autolaunch.is_none())
                .on_click(move |v, _, cx| {
                    let _ = weak.update(cx, |this, cx| {
                        if let Some(h) = &this.autolaunch
                            && let Err(error) = if *v { h.enable() } else { h.disable() }
                        {
                            this.notice =
                                Some(format!("Could not update login startup: {error}").into());
                            cx.notify();
                            return;
                        }
                        this.mutate(|s| s.launch_at_login = *v, cx);
                        if this.settings.launch_at_login != *v
                            && let Some(h) = &this.autolaunch
                        {
                            let _ = if this.settings.launch_at_login {
                                h.enable()
                            } else {
                                h.disable()
                            };
                        }
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
                    if self.background_available {
                        "Keep Running in Background"
                    } else {
                        "Background mode unavailable (no system tray)"
                    },
                    s.keep_running_in_background && self.background_available,
                    cx,
                    |s, v| s.keep_running_in_background = v,
                ),
            ],
        );

        // Notifications（§22: ローカルの OS 通知 ON/OFF のみ。プロジェクト毎は Web）
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
                    .label("Project notification settings (Web)")
                    .icon(Icon::new(IconName::ExternalLink))
                    .disabled(true)
                    .tooltip("Project notification settings are not available on the website yet")
                    .into_any_element(),
            ],
        );

        // Appearance
        let appearances = [Appearance::Light, Appearance::Dark, Appearance::System];
        let appearance = self.section(
            "Appearance",
            vec![
                RadioGroup::horizontal("appearance")
                    .children(["Light", "Dark", "System"])
                    .selected_index(appearances.iter().position(|value| *value == s.appearance))
                    .on_change(cx.listener(move |this, index: &usize, _, cx| {
                        if let Some(appearance) = appearances.get(*index) {
                            this.mutate(|settings| settings.appearance = *appearance, cx);
                        }
                    }))
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
                    .flex_wrap()
                    .w_full()
                    .items_center()
                    .gap_2()
                    .child(div().w(px(160.)).flex_shrink_0().child("Command Palette"))
                    .child(Input::new(&self.palette_key).w(px(200.)))
                    .into_any_element(),
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .w_full()
                    .items_center()
                    .gap_2()
                    .child(div().w(px(160.)).flex_shrink_0().child("Quick Search"))
                    .child(Input::new(&self.search_key).w(px(200.)))
                    .into_any_element(),
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
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
        let own_device_id = self.own_device_id();
        let device_rows: Vec<AnyElement> = self
            .devices
            .iter()
            .map(|d| {
                let own = Some(d.id) == own_device_id;
                let id = d.id;
                let last = d
                    .last_used_at
                    .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
                    .unwrap_or_else(|| "—".into());
                let weak = cx.entity().downgrade();
                div()
                    .flex()
                    .flex_row()
                    .w_full()
                    .min_w_0()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(div().text_sm().child(format!(
                                "{}{}",
                                d.name,
                                if own { " (this device)" } else { "" }
                            )))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(muted_fg)
                                    .child(format!("Last used {last}")),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(muted_fg)
                                    .child(format!("Expires {}", d.expires_at.format("%Y-%m-%d"))),
                            ),
                    )
                    .child(
                        Button::new(SharedString::from(format!("revoke-{id}")))
                            .ghost()
                            .label(if own { "Logout" } else { "Revoke" })
                            .on_click(move |_, _, cx| {
                                let _ = weak.update(cx, |this, cx| this.revoke_device(id, cx));
                            }),
                    )
                    .into_any_element()
            })
            .collect();
        let mut account_children = vec![];
        if let Some(profile) = &self.profile {
            account_children.push(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .w_full()
                    .min_w_0()
                    .child(
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .child(profile.username.clone()),
                    )
                    .child(div().text_color(muted_fg).child(profile.email.clone()))
                    .child(div().text_xs().text_color(muted_fg).child(format!(
                        "Email {} · Two-factor authentication {}",
                        if profile.email_verified {
                            "verified"
                        } else {
                            "not verified"
                        },
                        if profile.totp_enabled {
                            "enabled"
                        } else {
                            "disabled"
                        }
                    )))
                    .into_any_element(),
            );
        } else {
            account_children.push(
                div()
                    .text_sm()
                    .text_color(muted_fg)
                    .child(if self.profile_loading {
                        "Loading account information…"
                    } else if self.client.is_some() {
                        "Signed in to Koyori"
                    } else {
                        "Not signed in"
                    })
                    .into_any_element(),
            );
        }
        if let Some(error) = &self.profile_error {
            account_children.push(
                div()
                    .text_sm()
                    .child(format!("Could not load account: {error}"))
                    .into_any_element(),
            );
            account_children.push(
                Button::new("reload-account")
                    .ghost()
                    .label("Retry account information")
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(client) = this.client.clone() {
                            this.profile_loading = true;
                            this.profile_error = None;
                            this.refresh_profile(&client, cx);
                            cx.notify();
                        }
                    }))
                    .into_any_element(),
            );
        }
        account_children.push(
            div()
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .child("Devices")
                .into_any_element(),
        );
        if device_rows.is_empty() {
            account_children.push(
                div()
                    .text_sm()
                    .text_color(muted_fg)
                    .child(if self.devices_loading {
                        "Loading devices…"
                    } else {
                        "No devices"
                    })
                    .into_any_element(),
            );
        } else {
            account_children.extend(device_rows);
        }
        account_children.push(
            Button::new("logout")
                .outline()
                .disabled(self.client.is_none())
                .label("Logout")
                .icon(Icon::new(IconName::LogOut))
                .on_click(cx.listener(|this, _, _, cx| this.logout(cx)))
                .into_any_element(),
        );
        let account = self.section("Account", account_children);

        div()
            .id("settings-scroll")
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_y_scroll()
            .p_6()
            .flex()
            .flex_col()
            .gap_6()
            .text_sm()
            .child(
                div()
                    .flex_shrink_0()
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
