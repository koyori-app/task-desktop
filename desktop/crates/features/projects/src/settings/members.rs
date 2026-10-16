//! メンバー: 一覧・テナントメンバーからの追加・権限変更・削除。

use api::types::{AddMemberRequest, ProjectRole};
use gpui_kit::assets::IconName;
use gpui_kit::component::Disableable;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::*;
use i18n::t;
use uuid::Uuid;

use super::{ProjectSettingsView, Section};
use crate::ui::{Colors, card, confirm, placeholder, row};

const ROLES: [ProjectRole; 3] = [ProjectRole::Admin, ProjectRole::Member, ProjectRole::Viewer];

pub(crate) fn role_label(role: ProjectRole) -> &'static str {
    match role {
        ProjectRole::Admin => t!("projects.members.role_admin"),
        ProjectRole::Member => t!("projects.members.role_member"),
        ProjectRole::Viewer => t!("projects.members.role_viewer"),
    }
}

impl ProjectSettingsView {
    fn add_member(&mut self, user: Uuid, cx: &mut Context<Self>) {
        let Some((client, tenant, project)) = self.ctx() else {
            return;
        };
        let req = AddMemberRequest {
            user_id: user,
            role: ProjectRole::Member,
        };
        self.call(
            cx,
            async move { client.add_project_member(tenant, project, &req).await },
            |this, _, cx| this.load(Section::Members, cx),
        );
    }

    fn set_member_role(&mut self, user: Uuid, role: ProjectRole, cx: &mut Context<Self>) {
        let Some((client, tenant, project)) = self.ctx() else {
            return;
        };
        self.call(
            cx,
            async move {
                client
                    .update_project_member(tenant, project, user, role)
                    .await
            },
            |this, updated, _| {
                if let Some(m) = this
                    .members
                    .iter_mut()
                    .find(|m| m.user_id == updated.user_id)
                {
                    m.role = updated.role;
                }
            },
        );
    }

    fn remove_member(&mut self, user: Uuid, cx: &mut Context<Self>) {
        let Some((client, tenant, project)) = self.ctx() else {
            return;
        };
        self.call(
            cx,
            async move { client.remove_project_member(tenant, project, user).await },
            move |this, (), _| this.members.retain(|m| m.user_id != user),
        );
    }

    pub(crate) fn members_page(&self, c: &Colors, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let busy = self.busy > 0;
        let this = cx.entity().downgrade();
        let candidates: Vec<(Uuid, String)> = self
            .tenant_members
            .iter()
            .filter(|tm| !self.members.iter().any(|m| m.user_id == tm.user_id))
            .map(|tm| (tm.user_id, tm.user.username.clone()))
            .collect();
        let add_owner = this.clone();
        let add = Button::new("member-add")
            .outline()
            .small()
            .icon(IconName::UserPlus)
            .label(t!("projects.members.add"))
            .dropdown_caret(true)
            .disabled(busy || candidates.is_empty())
            .tooltip(if candidates.is_empty() {
                t!("projects.members.no_candidates")
            } else {
                t!("projects.members.add_desc")
            })
            .dropdown_menu(move |mut menu, _, _| {
                for (id, name) in &candidates {
                    let (id, owner) = (*id, add_owner.clone());
                    menu = menu.item(PopupMenuItem::new(name.clone()).on_click(move |_, _, cx| {
                        let _ = owner.update(cx, |this, cx| this.add_member(id, cx));
                    }));
                }
                menu
            });

        let mut rows = vec![];
        if self.is_loading(Section::Members) {
            rows.push(placeholder(c, t!("projects.common.loading")));
        } else if self.members.is_empty() {
            rows.push(placeholder(c, t!("projects.members.empty")));
        }
        for m in &self.members {
            let user = m.user_id;
            let current = m.role;
            let name = m.user.username.clone();
            let role_owner = this.clone();
            let remove_owner = this.clone();
            let role_button = Button::new(SharedString::from(format!("member-role-{user}")))
                .ghost()
                .small()
                .dropdown_caret(true)
                .disabled(busy)
                .label(role_label(current))
                .dropdown_menu(move |mut menu, _, _| {
                    for role in ROLES {
                        let owner = role_owner.clone();
                        menu = menu.item(
                            PopupMenuItem::new(role_label(role))
                                .checked(role == current)
                                .on_click(move |_, _, cx| {
                                    let _ = owner.update(cx, |this, cx| {
                                        this.set_member_role(user, role, cx)
                                    });
                                }),
                        );
                    }
                    menu
                });
            let remove = Button::new(SharedString::from(format!("member-remove-{user}")))
                .ghost()
                .small()
                .icon(IconName::UserMinus)
                .tooltip(t!("projects.members.remove"))
                .disabled(busy)
                .on_click(move |_, window, cx| {
                    let owner = remove_owner.clone();
                    confirm(
                        window,
                        cx,
                        t!("projects.members.remove_title", name = name),
                        t!("projects.members.remove_desc").into(),
                        move |_, cx| {
                            let _ = owner.update(cx, |this, cx| this.remove_member(user, cx));
                        },
                    );
                });
            rows.push(row(
                c,
                Some(crate::ui::initial(c, &m.user.username).into_any_element()),
                m.user.username.clone(),
                None,
                div().flex().gap_1().child(role_button).child(remove),
            ));
        }
        vec![
            div().flex().justify_end().child(add).into_any_element(),
            card(c, rows),
        ]
    }
}
