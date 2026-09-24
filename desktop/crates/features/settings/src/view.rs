//! §22 Settings。ローカル設定の編集 + Account（devices / logout）。
//! 変更は `SettingsEvent::Changed` で app 層へ通知し、app が保存・
//! テーマ適用などを行う。Token はここに置かない（credential store）。

use core::settings::Appearance;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Disableable, Icon, Selectable, Theme};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use i18n::t;

/// app 層への通知。
pub enum SettingsEvent {
    /// ローカル設定が変わった（保存・テーマ適用は app 側）。
    Changed(core::Settings),
    /// ログアウト完了（client 破棄と未ログイン画面への遷移は app 側）。
    LoggedOut,
    /// 設定画面を閉じて元の画面へ戻る。
    Close,
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
    section: Section,
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
            section: Section::General,
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
            Err(error) => self.notice = Some(t!("settings.save_error", error = error).into()),
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
                        s.notice = Some(t!("settings.account.device_removed").into());
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
            self.notice = Some(t!("settings.keyboard.empty").into());
            cx.notify();
            return;
        }
        let valid = |key: &str| {
            key.split_whitespace()
                .all(|part| Keystroke::parse(part).is_ok())
        };
        if !valid(&palette) || !valid(&search) || palette == search {
            self.notice = Some(t!("settings.keyboard.invalid").into());
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
            self.notice = Some(t!("settings.keyboard.saved").into());
        }
        cx.notify();
    }

    // ---- render helpers ----

    fn switch(
        &self,
        id: &'static str,
        checked: bool,
        disabled: bool,
        cx: &mut Context<Self>,
        f: impl Fn(&mut core::Settings, bool) + 'static,
    ) -> AnyElement {
        let weak = cx.entity().downgrade();
        Switch::new(id)
            .checked(checked)
            .disabled(disabled)
            .on_click(move |v, _, cx| {
                let _ = weak.update(cx, |this, cx| {
                    this.mutate(|s| f(s, *v), cx);
                });
            })
            .into_any_element()
    }

    fn set_launch_at_login(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if let Some(h) = &self.autolaunch
            && let Err(error) = if enabled { h.enable() } else { h.disable() }
        {
            self.notice = Some(t!("settings.general.launch_error", error = error).into());
            cx.notify();
            return;
        }
        self.mutate(|s| s.launch_at_login = enabled, cx);
        // 保存に失敗したら OS 側も元に戻す。
        if self.settings.launch_at_login != enabled
            && let Some(h) = &self.autolaunch
        {
            let _ = if self.settings.launch_at_login {
                h.enable()
            } else {
                h.disable()
            };
        }
    }

    fn general_page(&self, c: &Colors, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let s = &self.settings;
        let weak = cx.entity().downgrade();
        let launch = Switch::new("launch-at-login")
            .checked(s.launch_at_login)
            .disabled(self.autolaunch.is_none())
            .on_click(move |v, _, cx| {
                let _ = weak.update(cx, |this, cx| this.set_launch_at_login(*v, cx));
            })
            .into_any_element();
        let background = self.switch(
            "keep-bg",
            s.keep_running_in_background && self.background_available,
            !self.background_available,
            cx,
            |s, v| s.keep_running_in_background = v,
        );
        let current = s.language;
        let owner = cx.entity().downgrade();
        let language = Button::new("language-select")
            .outline()
            .compact()
            .dropdown_caret(true)
            .label(current.native_name())
            .dropdown_menu(move |mut menu, _, _| {
                for language in i18n::Language::ALL {
                    let owner = owner.clone();
                    menu = menu.item(
                        PopupMenuItem::new(language.native_name())
                            .checked(language == current)
                            .on_click(move |_, _, cx| {
                                let _ = owner.update(cx, |this, cx| {
                                    this.mutate(|s| s.language = language, cx)
                                });
                            }),
                    );
                }
                menu
            })
            .into_any_element();
        vec![group(
            c,
            vec![
                row(
                    c,
                    t!("settings.general.language"),
                    Some(t!("settings.general.language_desc")),
                    language,
                ),
                row(
                    c,
                    t!("settings.general.launch"),
                    Some(t!("settings.general.launch_desc")),
                    launch,
                ),
                row(
                    c,
                    t!("settings.general.background"),
                    Some(if self.background_available {
                        t!("settings.general.background_desc")
                    } else {
                        t!("settings.general.background_unavailable")
                    }),
                    background,
                ),
            ],
        )]
    }

    fn notifications_page(&self, c: &Colors, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let n = self.settings.notifications.clone();
        let off = !n.enabled;
        vec![
            group(
                c,
                vec![row(
                    c,
                    t!("settings.notifications.desktop"),
                    Some(t!("settings.notifications.desktop_desc")),
                    self.switch("notif-enabled", n.enabled, false, cx, |s, v| {
                        s.notifications.enabled = v
                    }),
                )],
            ),
            group_title(c, t!("settings.notifications.notify_about")),
            group(
                c,
                vec![
                    row(
                        c,
                        t!("settings.notifications.tasks"),
                        Some(t!("settings.notifications.tasks_desc")),
                        self.switch("notif-task", n.task, off, cx, |s, v| {
                            s.notifications.task = v
                        }),
                    ),
                    row(
                        c,
                        t!("settings.notifications.reviews"),
                        Some(t!("settings.notifications.reviews_desc")),
                        self.switch("notif-review", n.review, off, cx, |s, v| {
                            s.notifications.review = v
                        }),
                    ),
                    row(
                        c,
                        t!("settings.notifications.due"),
                        Some(t!("settings.notifications.due_desc")),
                        self.switch("notif-due", n.due_date, off, cx, |s, v| {
                            s.notifications.due_date = v
                        }),
                    ),
                ],
            ),
            group(
                c,
                vec![row(
                    c,
                    t!("settings.notifications.per_project"),
                    Some(t!("settings.notifications.per_project_desc")),
                    Button::new("notif-web-link")
                        .outline()
                        .compact()
                        .label(t!("settings.notifications.open_web"))
                        .icon(Icon::new(IconName::ExternalLink))
                        .disabled(true)
                        .tooltip(t!("settings.notifications.open_web_unavailable"))
                        .into_any_element(),
                )],
            ),
        ]
    }

    fn appearance_page(&self, c: &Colors, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let current = self.settings.appearance;
        let options = [
            (
                Appearance::Light,
                t!("settings.appearance.light"),
                IconName::Sun,
            ),
            (
                Appearance::Dark,
                t!("settings.appearance.dark"),
                IconName::Moon,
            ),
            (
                Appearance::System,
                t!("settings.appearance.system"),
                IconName::Settings2,
            ),
        ];
        let mut choices = div().flex().flex_row().gap_2();
        for (value, label, icon) in options {
            choices = choices.child(
                Button::new(SharedString::from(format!("appearance-{value:?}")))
                    .outline()
                    .compact()
                    .selected(current == value)
                    .icon(icon)
                    .label(label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.mutate(|s| s.appearance = value, cx);
                    })),
            );
        }
        vec![group(
            c,
            vec![row(
                c,
                t!("settings.appearance.theme"),
                Some(t!("settings.appearance.theme_desc")),
                choices.into_any_element(),
            )],
        )]
    }

    fn keyboard_page(&self, c: &Colors, cx: &mut Context<Self>) -> Vec<AnyElement> {
        vec![
            group(
                c,
                vec![
                    row(
                        c,
                        t!("settings.keyboard.palette"),
                        Some(t!("settings.keyboard.palette_desc")),
                        Input::new(&self.palette_key).w(px(160.)).into_any_element(),
                    ),
                    row(
                        c,
                        t!("settings.keyboard.search"),
                        Some(t!("settings.keyboard.search_desc")),
                        Input::new(&self.search_key).w(px(160.)).into_any_element(),
                    ),
                ],
            ),
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_3()
                .child(
                    Button::new("save-keybindings")
                        .primary()
                        .compact()
                        .label(t!("settings.keyboard.save"))
                        .on_click(cx.listener(|this, _, _, cx| this.save_keybindings(cx))),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(c.muted)
                        .child(t!("settings.keyboard.hint")),
                )
                .into_any_element(),
        ]
    }

    fn account_page(&self, c: &Colors, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out = vec![];
        let profile_row = if let Some(profile) = &self.profile {
            let initial = profile
                .username
                .chars()
                .next()
                .map(|ch| ch.to_uppercase().to_string())
                .unwrap_or_default();
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_3()
                .p_4()
                .child(
                    div()
                        .size(px(40.))
                        .flex_shrink_0()
                        .rounded_full()
                        .bg(c.accent)
                        .text_color(c.accent_fg)
                        .flex()
                        .items_center()
                        .justify_center()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(initial),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .min_w_0()
                        .gap_0p5()
                        .child(
                            div()
                                .font_weight(FontWeight::MEDIUM)
                                .child(profile.username.clone()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(c.muted)
                                .child(profile.email.clone()),
                        )
                        .child(div().text_xs().text_color(c.muted).child(t!(
                            "settings.account.status",
                            email = if profile.email_verified {
                                t!("settings.account.verified")
                            } else {
                                t!("settings.account.not_verified")
                            },
                            totp = if profile.totp_enabled {
                                t!("settings.account.on")
                            } else {
                                t!("settings.account.off")
                            }
                        ))),
                )
                .into_any_element()
        } else {
            div()
                .p_4()
                .text_sm()
                .text_color(c.muted)
                .child(if self.profile_loading {
                    t!("settings.account.loading")
                } else if self.client.is_some() {
                    t!("settings.account.signed_in")
                } else {
                    t!("settings.account.not_signed_in")
                })
                .into_any_element()
        };
        let mut profile_rows = vec![profile_row];
        if let Some(error) = &self.profile_error {
            profile_rows.push(row(
                c,
                t!("settings.account.load_error"),
                Some(error.as_ref()),
                Button::new("reload-account")
                    .outline()
                    .compact()
                    .label(t!("settings.account.retry"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(client) = this.client.clone() {
                            this.profile_loading = true;
                            this.profile_error = None;
                            this.refresh_profile(&client, cx);
                            cx.notify();
                        }
                    }))
                    .into_any_element(),
            ));
        }
        out.push(group(c, profile_rows));

        out.push(group_title(c, t!("settings.account.devices")));
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
                let title = if own {
                    t!("settings.account.this_device", name = d.name)
                } else {
                    d.name.clone()
                };
                row_owned(
                    c,
                    title,
                    Some(t!(
                        "settings.account.device_meta",
                        last = last,
                        expires = d.expires_at.format("%Y-%m-%d")
                    )),
                    Button::new(SharedString::from(format!("revoke-{id}")))
                        .outline()
                        .compact()
                        .label(if own {
                            t!("settings.account.logout")
                        } else {
                            t!("settings.account.revoke")
                        })
                        .on_click(move |_, _, cx| {
                            let _ = weak.update(cx, |this, cx| this.revoke_device(id, cx));
                        })
                        .into_any_element(),
                )
            })
            .collect();
        if device_rows.is_empty() {
            out.push(group(
                c,
                vec![
                    div()
                        .p_4()
                        .text_sm()
                        .text_color(c.muted)
                        .child(if self.devices_loading {
                            t!("settings.account.loading_devices")
                        } else {
                            t!("settings.account.no_devices")
                        })
                        .into_any_element(),
                ],
            ));
        } else {
            out.push(group(c, device_rows));
        }

        out.push(group(
            c,
            vec![row(
                c,
                t!("settings.account.logout"),
                Some(t!("settings.account.logout_desc")),
                Button::new("logout")
                    .danger()
                    .compact()
                    .disabled(self.client.is_none())
                    .label(t!("settings.account.logout"))
                    .icon(Icon::new(IconName::LogOut))
                    .on_click(cx.listener(|this, _, _, cx| this.logout(cx)))
                    .into_any_element(),
            )],
        ));
        out
    }
}

fn key_or(settings: &core::Settings, key: &str, default: &str) -> String {
    settings
        .keybindings
        .get(key)
        .cloned()
        .unwrap_or_else(|| default.into())
}

/// render で使う色。
struct Colors {
    muted: Hsla,
    border: Hsla,
    surface: Hsla,
    hover: Hsla,
    accent: Hsla,
    accent_fg: Hsla,
}

/// 項目をまとめるカード。行の間に区切り線を入れる。
fn group(c: &Colors, rows: Vec<AnyElement>) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .flex_shrink_0()
        .w_full()
        .rounded_lg()
        .border_1()
        .border_color(c.border)
        .bg(c.surface)
        .children(rows.into_iter().enumerate().map(|(ix, row)| {
            div()
                .when(ix > 0, |d| d.border_t_1().border_color(c.border))
                .child(row)
        }))
        .into_any_element()
}

fn group_title(c: &Colors, title: &'static str) -> AnyElement {
    div()
        .pt_2()
        .text_sm()
        .font_weight(FontWeight::MEDIUM)
        .text_color(c.muted)
        .child(title)
        .into_any_element()
}

/// 左に名前と説明、右に操作部品を置く 1 行。
fn row(
    c: &Colors,
    title: &'static str,
    description: Option<&str>,
    control: AnyElement,
) -> AnyElement {
    row_owned(
        c,
        title.to_string(),
        description.map(str::to_string),
        control,
    )
}

fn row_owned(
    c: &Colors,
    title: String,
    description: Option<String>,
    control: AnyElement,
) -> AnyElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap_4()
        .px_4()
        .py_3()
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap_0p5()
                .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(title))
                .when_some(description, |d, text| {
                    d.child(div().text_xs().text_color(c.muted).child(text))
                }),
        )
        .child(div().flex_shrink_0().child(control))
        .into_any_element()
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = {
            let t = Theme::global(cx);
            let tokens = t.semantic_tokens().colors;
            Colors {
                muted: t.muted_foreground,
                border: t.border,
                surface: tokens.background,
                hover: t.secondary,
                accent: t.primary,
                accent_fg: t.primary_foreground,
            }
        };
        let section = self.section;
        let body = match section {
            Section::General => self.general_page(&colors, cx),
            Section::Notifications => self.notifications_page(&colors, cx),
            Section::Appearance => self.appearance_page(&colors, cx),
            Section::Keyboard => self.keyboard_page(&colors, cx),
            Section::Account => self.account_page(&colors, cx),
        };

        let nav = div()
            .w(px(220.))
            .h_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap_1()
            .p_3()
            .border_r_1()
            .border_color(colors.border)
            .child(
                div().flex().pb_2().child(
                    Button::new("settings-back")
                        .ghost()
                        .compact()
                        .icon(IconName::ArrowLeft)
                        .label(t!("settings.back"))
                        .tooltip(t!("settings.back_tooltip"))
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(SettingsEvent::Close))),
                ),
            )
            .child(
                div()
                    .px_2()
                    .pb_2()
                    .text_lg()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(t!("settings.title")),
            )
            .children(Section::ALL.into_iter().map(|item| {
                let active = item == section;
                div()
                    .id(SharedString::from(format!("settings-nav-{item:?}")))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_1p5()
                    .rounded_md()
                    .text_sm()
                    .cursor_pointer()
                    .when(active, |d| {
                        d.bg(colors.hover).font_weight(FontWeight::MEDIUM)
                    })
                    .when(!active, |d| {
                        d.text_color(colors.muted).hover(|d| d.bg(colors.hover))
                    })
                    .child(Icon::new(item.icon()).size_4())
                    .child(item.label())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.section = item;
                        cx.notify();
                    }))
            }));

        let content = div()
            .id("settings-scroll")
            .flex_1()
            .h_full()
            .min_w_0()
            .overflow_y_scroll()
            .child(
                div()
                    .max_w(px(720.))
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .px_8()
                    .py_6()
                    .child(
                        div()
                            .pb_2()
                            .text_xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(section.label()),
                    )
                    .when_some(self.notice.clone(), |d, n| {
                        d.child(
                            div()
                                .px_3()
                                .py_2()
                                .rounded_md()
                                .border_1()
                                .border_color(colors.border)
                                .text_sm()
                                .child(n),
                        )
                    })
                    .children(body),
            );

        div()
            .id("settings")
            .size_full()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_row()
            .child(nav)
            .child(content)
    }
}

/// 左ナビの区分（§22 の区分に対応）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    General,
    Notifications,
    Appearance,
    Keyboard,
    Account,
}

impl Section {
    const ALL: [Section; 5] = [
        Section::General,
        Section::Notifications,
        Section::Appearance,
        Section::Keyboard,
        Section::Account,
    ];

    fn label(self) -> &'static str {
        match self {
            Section::General => t!("settings.section.general"),
            Section::Notifications => t!("settings.section.notifications"),
            Section::Appearance => t!("settings.section.appearance"),
            Section::Keyboard => t!("settings.section.keyboard"),
            Section::Account => t!("settings.section.account"),
        }
    }

    fn icon(self) -> IconName {
        match self {
            Section::General => IconName::Settings,
            Section::Notifications => IconName::Bell,
            Section::Appearance => IconName::Palette,
            Section::Keyboard => IconName::SquareTerminal,
            Section::Account => IconName::CircleUser,
        }
    }
}
