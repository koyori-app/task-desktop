//! Account のプロフィール編集と Workspace（テナント情報・メンバー）。
//! `SettingsView` の非公開フィールドを使うので view の子モジュールに置く。

use api::types::{TenantMemberResponse, TenantRole, UpdateProfileRequest, UpdateTenantRequest};
use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::{Disableable, Icon, WindowExt};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use i18n::t;

use super::{Colors, SettingsView, group, group_title, row, row_owned};

pub(super) const ROLES: [TenantRole; 3] =
    [TenantRole::Admin, TenantRole::Member, TenantRole::Viewer];

pub(super) fn role_label(role: TenantRole) -> &'static str {
    match role {
        TenantRole::Admin => t!("settings.workspace.role_admin"),
        TenantRole::Member => t!("settings.workspace.role_member"),
        TenantRole::Viewer => t!("settings.workspace.role_viewer"),
    }
}

/// 編集欄の入力値。空欄は「変えない」ではなく「空にする」意味なので、そのまま送る。
pub(super) struct Inputs {
    pub username: Entity<InputState>,
    pub bio: Entity<InputState>,
    pub avatar: Entity<InputState>,
    pub tenant_name: Entity<InputState>,
    pub tenant_description: Entity<InputState>,
    pub tenant_emoji: Entity<InputState>,
}

impl Inputs {
    pub fn new(window: &mut Window, cx: &mut App) -> Self {
        let input = |placeholder: &'static str, window: &mut Window, cx: &mut App| {
            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
        };
        Self {
            username: input(t!("settings.account.username_placeholder"), window, cx),
            bio: input(t!("settings.account.bio_placeholder"), window, cx),
            avatar: input(t!("settings.account.avatar_placeholder"), window, cx),
            tenant_name: input(t!("settings.workspace.name_placeholder"), window, cx),
            tenant_description: input(t!("settings.workspace.description_placeholder"), window, cx),
            tenant_emoji: input(t!("settings.workspace.emoji_placeholder"), window, cx),
        }
    }
}

/// 変更のあった項目だけを入れた更新要求（何も変わっていなければ None）。
pub(super) fn profile_patch(
    current: &api::types::UserResponse,
    username: &str,
    bio: &str,
    avatar: &str,
) -> Option<UpdateProfileRequest> {
    let mut req = UpdateProfileRequest::default();
    if username != current.username {
        req.username = Some(username.to_string());
    }
    if bio != current.bio.as_deref().unwrap_or_default() {
        req.bio = Some(bio.to_string());
    }
    if avatar != current.avatar_url.as_deref().unwrap_or_default() {
        if avatar.is_empty() {
            req.clear_avatar_url = Some(true);
        } else {
            req.avatar_url = Some(avatar.to_string());
        }
    }
    let changed = req.username.is_some()
        || req.bio.is_some()
        || req.avatar_url.is_some()
        || req.clear_avatar_url.is_some();
    changed.then_some(req)
}

impl SettingsView {
    /// ヘッダーのテナント切り替えに合わせる。Workspace 区分の中身を取り直す。
    pub fn set_tenant(&mut self, tenant: Option<uuid::Uuid>, cx: &mut Context<Self>) {
        if self.tenant == tenant {
            return;
        }
        self.tenant = tenant;
        self.reload_workspace(cx);
    }

    pub(super) fn reload_workspace(&mut self, cx: &mut Context<Self>) {
        self.workspace_generation += 1;
        self.tenant_info = None;
        self.tenant_error = None;
        self.members.clear();
        self.members_error = None;
        self.tenant_synced = false;
        let (Some(client), Some(tenant)) = (self.client.clone(), self.tenant) else {
            self.workspace_loading = false;
            cx.notify();
            return;
        };
        self.workspace_loading = true;
        let generation = self.workspace_generation;
        cx.spawn(async move |this, cx| {
            let info = client.get_tenant(tenant).await;
            let members = client.list_tenant_members(tenant).await;
            let _ = this.update(cx, |this, cx| {
                if this.workspace_generation != generation {
                    return;
                }
                this.workspace_loading = false;
                match info {
                    Ok(info) => this.tenant_info = Some(info),
                    // 客分（Guest）はテナント取得の口が開いていない。
                    Err(api::ApiError::Forbidden { .. } | api::ApiError::NotFound) => {
                        this.tenant_error = Some(t!("settings.workspace.no_access").into())
                    }
                    Err(e) => this.tenant_error = Some(e.to_string().into()),
                }
                match members {
                    Ok(mut members) => {
                        members.sort_by(|a, b| a.user.username.cmp(&b.user.username));
                        this.members = members;
                    }
                    Err(e) => this.members_error = Some(e.to_string().into()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn save_profile(&mut self, cx: &mut Context<Self>) {
        let (Some(client), Some(current)) = (self.client.clone(), self.profile.clone()) else {
            return;
        };
        if self.profile_saving {
            return;
        }
        let read = |input: &Entity<InputState>, cx: &App| input.read(cx).value().trim().to_string();
        let username = read(&self.inputs.username, cx);
        if username.is_empty() {
            self.notice = Some(t!("settings.account.username_required").into());
            cx.notify();
            return;
        }
        let Some(req) = profile_patch(
            &current,
            &username,
            &read(&self.inputs.bio, cx),
            &read(&self.inputs.avatar, cx),
        ) else {
            self.notice = Some(t!("settings.account.no_changes").into());
            cx.notify();
            return;
        };
        self.profile_saving = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = client.update_me(&req).await;
            let _ = this.update(cx, |this, cx| {
                this.profile_saving = false;
                match result {
                    Ok(profile) => {
                        this.profile = Some(profile);
                        this.profile_synced = false;
                        this.notice = Some(t!("settings.account.profile_saved").into());
                    }
                    Err(e) => this.notice = Some(e.to_string().into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn save_tenant(&mut self, cx: &mut Context<Self>) {
        let (Some(client), Some(tenant), Some(current)) =
            (self.client.clone(), self.tenant, self.tenant_info.clone())
        else {
            return;
        };
        if self.tenant_saving {
            return;
        }
        let read = |input: &Entity<InputState>, cx: &App| input.read(cx).value().trim().to_string();
        let name = read(&self.inputs.tenant_name, cx);
        if name.is_empty() {
            self.notice = Some(t!("settings.workspace.name_required").into());
            cx.notify();
            return;
        }
        let description = read(&self.inputs.tenant_description, cx);
        let emoji = read(&self.inputs.tenant_emoji, cx);
        let req = UpdateTenantRequest {
            name: (name != current.name).then_some(name),
            description: (description != current.description).then_some(description),
            icon_emoji: (emoji != current.icon_emoji.clone().unwrap_or_default()).then_some(emoji),
            icon_url: None,
        };
        if req.name.is_none() && req.description.is_none() && req.icon_emoji.is_none() {
            self.notice = Some(t!("settings.account.no_changes").into());
            cx.notify();
            return;
        }
        self.tenant_saving = true;
        cx.notify();
        let generation = self.workspace_generation;
        cx.spawn(async move |this, cx| {
            let result = client.update_tenant(tenant, &req).await;
            let _ = this.update(cx, |this, cx| {
                this.tenant_saving = false;
                if this.workspace_generation != generation {
                    return;
                }
                match result {
                    Ok(info) => {
                        this.tenant_info = Some(info.clone());
                        this.tenant_synced = false;
                        this.notice = Some(t!("settings.workspace.saved").into());
                        cx.emit(super::SettingsEvent::TenantUpdated {
                            id: info.id,
                            name: info.name,
                        });
                    }
                    Err(e) => this.notice = Some(e.to_string().into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn set_member_role(&mut self, user: uuid::Uuid, role: TenantRole, cx: &mut Context<Self>) {
        let (Some(client), Some(tenant)) = (self.client.clone(), self.tenant) else {
            return;
        };
        let generation = self.workspace_generation;
        cx.spawn(async move |this, cx| {
            let result = client.update_tenant_member(tenant, user, role).await;
            let _ = this.update(cx, |this, cx| {
                if this.workspace_generation != generation {
                    return;
                }
                match result {
                    Ok(updated) => {
                        if let Some(m) = this.members.iter_mut().find(|m| m.user_id == user) {
                            m.role = updated.role;
                        }
                    }
                    Err(e) => this.notice = Some(e.to_string().into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn confirm_remove_member(
        &mut self,
        member: TenantMemberResponse,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let weak = cx.entity().downgrade();
        let name = member.user.username.clone();
        let user = member.user_id;
        window.open_alert_dialog(cx, move |alert, _, _| {
            let weak = weak.clone();
            alert
                .confirm()
                .title(t!("settings.workspace.remove_title"))
                .description(t!("settings.workspace.remove_desc", name = name))
                .button_props(
                    gpui_kit::component::dialog::DialogButtonProps::default()
                        .ok_text(t!("settings.workspace.remove"))
                        .ok_variant(ButtonVariant::Danger)
                        .cancel_text(t!("settings.workspace.cancel"))
                        .show_cancel(true),
                )
                .on_ok(move |_, _, cx| {
                    let _ = weak.update(cx, |this, cx| this.remove_member(user, cx));
                    true
                })
        });
    }

    fn remove_member(&mut self, user: uuid::Uuid, cx: &mut Context<Self>) {
        let (Some(client), Some(tenant)) = (self.client.clone(), self.tenant) else {
            return;
        };
        let generation = self.workspace_generation;
        cx.spawn(async move |this, cx| {
            let result = client.remove_tenant_member(tenant, user).await;
            let _ = this.update(cx, |this, cx| {
                if this.workspace_generation != generation {
                    return;
                }
                match result {
                    Ok(()) => {
                        this.members.retain(|m| m.user_id != user);
                        this.notice = Some(t!("settings.workspace.removed").into());
                    }
                    Err(e) => this.notice = Some(e.to_string().into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 取得した値を編集欄へ入れる（render でしか Window を借りられない）。
    pub(super) fn sync_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.profile_synced
            && let Some(p) = self.profile.clone()
        {
            self.profile_synced = true;
            let set =
                |input: &Entity<InputState>, value: String, window: &mut Window, cx: &mut App| {
                    input.update(cx, |s, cx| s.set_value(value, window, cx));
                };
            set(&self.inputs.username, p.username, window, cx);
            set(&self.inputs.bio, p.bio.unwrap_or_default(), window, cx);
            set(
                &self.inputs.avatar,
                p.avatar_url.unwrap_or_default(),
                window,
                cx,
            );
        }
        if !self.tenant_synced
            && let Some(info) = self.tenant_info.clone()
        {
            self.tenant_synced = true;
            let set =
                |input: &Entity<InputState>, value: String, window: &mut Window, cx: &mut App| {
                    input.update(cx, |s, cx| s.set_value(value, window, cx));
                };
            set(&self.inputs.tenant_name, info.name, window, cx);
            set(
                &self.inputs.tenant_description,
                info.description,
                window,
                cx,
            );
            set(
                &self.inputs.tenant_emoji,
                info.icon_emoji.unwrap_or_default(),
                window,
                cx,
            );
        }
    }

    /// Account の先頭に置くプロフィール編集欄。
    pub(super) fn profile_form(&self, c: &Colors, cx: &mut Context<Self>) -> Option<AnyElement> {
        self.profile.as_ref()?;
        let saving = self.profile_saving;
        Some(group(
            c,
            vec![
                field_row(
                    c,
                    t!("settings.account.username"),
                    None,
                    &self.inputs.username,
                    "profile-username",
                ),
                field_row(
                    c,
                    t!("settings.account.bio"),
                    None,
                    &self.inputs.bio,
                    "profile-bio",
                ),
                field_row(
                    c,
                    t!("settings.account.avatar"),
                    Some(t!("settings.account.avatar_desc")),
                    &self.inputs.avatar,
                    "profile-avatar",
                ),
                div()
                    .flex()
                    .justify_end()
                    .px_4()
                    .py_3()
                    .child(
                        Button::new("save-profile")
                            .primary()
                            .small()
                            .icon(Icon::new(IconName::Save))
                            .disabled(saving)
                            .label(if saving {
                                t!("settings.account.saving")
                            } else {
                                t!("settings.account.save_profile")
                            })
                            .on_click(cx.listener(|this, _, _, cx| this.save_profile(cx))),
                    )
                    .into_any_element(),
            ],
        ))
    }

    pub(super) fn workspace_page(&self, c: &Colors, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out = vec![];
        if self.tenant.is_none() || self.client.is_none() {
            out.push(group(c, vec![message(c, t!("settings.workspace.none"))]));
            return out;
        }
        match (&self.tenant_info, &self.tenant_error) {
            (Some(info), _) => {
                let saving = self.tenant_saving;
                out.push(group(
                    c,
                    vec![
                        row_owned(
                            c,
                            t!("settings.workspace.id").to_string(),
                            Some(t!("settings.workspace.id_desc").to_string()),
                            div()
                                .text_sm()
                                .child(info.display_id.clone())
                                .into_any_element(),
                        ),
                        field_row(
                            c,
                            t!("settings.workspace.name"),
                            None,
                            &self.inputs.tenant_name,
                            "tenant-name",
                        ),
                        field_row(
                            c,
                            t!("settings.workspace.description"),
                            None,
                            &self.inputs.tenant_description,
                            "tenant-description",
                        ),
                        field_row(
                            c,
                            t!("settings.workspace.emoji"),
                            None,
                            &self.inputs.tenant_emoji,
                            "tenant-emoji",
                        ),
                        row(
                            c,
                            t!("settings.workspace.require_2fa"),
                            Some(t!("settings.workspace.require_2fa_desc")),
                            div()
                                .text_sm()
                                .child(if info.require_2fa {
                                    t!("settings.account.on")
                                } else {
                                    t!("settings.account.off")
                                })
                                .into_any_element(),
                        ),
                        div()
                            .flex()
                            .justify_end()
                            .px_4()
                            .py_3()
                            .child(
                                Button::new("save-tenant")
                                    .primary()
                                    .small()
                                    .icon(Icon::new(IconName::Save))
                                    .disabled(saving)
                                    .label(if saving {
                                        t!("settings.account.saving")
                                    } else {
                                        t!("settings.workspace.save")
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| this.save_tenant(cx))),
                            )
                            .into_any_element(),
                    ],
                ));
            }
            (None, Some(error)) => out.push(group(
                c,
                vec![row_owned(
                    c,
                    t!("settings.workspace.load_error").to_string(),
                    Some(error.to_string()),
                    retry_button(cx),
                )],
            )),
            (None, None) => out.push(group(c, vec![message(c, t!("settings.workspace.loading"))])),
        }

        out.push(group_title(c, t!("settings.workspace.members")));
        if let Some(error) = &self.members_error {
            out.push(group(
                c,
                vec![row_owned(
                    c,
                    t!("settings.workspace.members_error").to_string(),
                    Some(error.to_string()),
                    retry_button(cx),
                )],
            ));
            return out;
        }
        if self.members.is_empty() {
            out.push(group(
                c,
                vec![message(
                    c,
                    if self.workspace_loading {
                        t!("settings.workspace.loading")
                    } else {
                        t!("settings.workspace.no_members")
                    },
                )],
            ));
            return out;
        }
        let owner = self.tenant_info.as_ref().map(|i| i.owner_id);
        let me = self.profile.as_ref().map(|p| p.id);
        let rows = self
            .members
            .iter()
            .map(|m| {
                let user = m.user_id;
                let is_owner = owner == Some(user);
                let mut title = m.user.username.clone();
                if me == Some(user) {
                    title = t!("settings.workspace.you", name = title);
                }
                let weak = cx.entity().downgrade();
                let role = m.role;
                let role_button = Button::new(SharedString::from(format!("member-role-{user}")))
                    .outline()
                    .small()
                    .dropdown_caret(true)
                    .disabled(is_owner)
                    .label(if is_owner {
                        t!("settings.workspace.owner")
                    } else {
                        role_label(role)
                    })
                    .dropdown_menu(move |mut menu, _, _| {
                        for r in ROLES {
                            let weak = weak.clone();
                            menu = menu.item(
                                PopupMenuItem::new(role_label(r))
                                    .checked(r == role)
                                    .on_click(move |_, _, cx| {
                                        let _ = weak.update(cx, |this, cx| {
                                            this.set_member_role(user, r, cx)
                                        });
                                    }),
                            );
                        }
                        menu
                    });
                let member = m.clone();
                let remove = Button::new(SharedString::from(format!("member-remove-{user}")))
                    .ghost()
                    .small()
                    .icon(Icon::new(IconName::UserMinus))
                    .tooltip(t!("settings.workspace.remove"))
                    .disabled(is_owner)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.confirm_remove_member(member.clone(), window, cx)
                    }));
                row_owned(
                    c,
                    title,
                    None,
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(role_button)
                        .child(remove)
                        .into_any_element(),
                )
            })
            .collect();
        out.push(group(c, rows));
        out
    }
}

fn retry_button(cx: &mut Context<SettingsView>) -> AnyElement {
    Button::new("reload-workspace")
        .outline()
        .small()
        .label(t!("settings.account.retry"))
        .on_click(cx.listener(|this, _, _, cx| this.reload_workspace(cx)))
        .into_any_element()
}

fn message(c: &Colors, text: &'static str) -> AnyElement {
    div()
        .p_4()
        .text_sm()
        .text_color(c.text_muted)
        .child(text)
        .into_any_element()
}

/// 名前の下に入力欄を置く 1 行（長い値も切れずに入る）。
fn field_row(
    c: &Colors,
    title: &'static str,
    description: Option<&'static str>,
    input: &Entity<InputState>,
    id: &'static str,
) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_1p5()
        .px_4()
        .py_3()
        .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(title))
        .when_some(description, |d, text| {
            d.child(div().text_xs().text_color(c.text_muted).child(text))
        })
        .child(Input::new(input).id(id))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::profile_patch;

    fn me() -> api::types::UserResponse {
        serde_json::from_value(serde_json::json!({
            "id": uuid::Uuid::nil(), "username": "yupix", "email": "y@example.com",
            "email_verified": true, "has_password": true, "is_admin": false,
            "is_suspended": false, "totp_enabled": false, "bio": "hi",
            "avatar_url": "https://example.com/a.png",
        }))
        .unwrap()
    }

    #[test]
    fn profile_patch_sends_only_changed_fields() {
        let me = me();
        assert!(profile_patch(&me, "yupix", "hi", "https://example.com/a.png").is_none());
        let req = profile_patch(&me, "yupix2", "hi", "https://example.com/a.png").unwrap();
        assert_eq!(req.username.as_deref(), Some("yupix2"));
        assert!(req.bio.is_none() && req.avatar_url.is_none() && req.clear_avatar_url.is_none());
    }

    #[test]
    fn empty_avatar_clears_instead_of_sending_empty_url() {
        let req = profile_patch(&me(), "yupix", "hi", "").unwrap();
        assert_eq!(req.clear_avatar_url, Some(true));
        assert!(req.avatar_url.is_none());
    }
}
