//! 概要: 名前・説明・アイコン絵文字の編集と、キー入力で確認する削除。

use api::types::UpdateProjectRequest;
use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, Textarea};
use gpui_kit::component::{Disableable, Icon};
use gpui_kit::*;
use i18n::t;

use super::{ProjectSettingsEvent, ProjectSettingsView};
use crate::ui::{Colors, badge, card, field, heading, placeholder, row};

impl ProjectSettingsView {
    pub(crate) fn save_overview(&mut self, cx: &mut Context<Self>) {
        let Some((client, tenant, project)) = self.ctx() else {
            return;
        };
        let f = &self.forms;
        let name = f.name.read(cx).value().trim().to_string();
        if name.is_empty() {
            self.notice(t!("projects.overview.name_required"));
            cx.notify();
            return;
        }
        let emoji = f.emoji.read(cx).value().trim().to_string();
        let req = UpdateProjectRequest {
            name: Some(name),
            description: Some(f.description.read(cx).value().trim().to_string()),
            clear_icon_emoji: emoji.is_empty().then_some(true),
            icon_emoji: (!emoji.is_empty()).then_some(emoji),
            ..Default::default()
        };
        self.call(
            cx,
            async move { client.update_project(tenant, project, &req).await },
            |this, info, cx| {
                this.info = Some(info.clone());
                this.notice(t!("projects.common.saved"));
                cx.emit(ProjectSettingsEvent::Updated(info));
            },
        );
    }

    fn delete_project(&mut self, cx: &mut Context<Self>) {
        let Some((client, tenant, project)) = self.ctx() else {
            return;
        };
        self.call(
            cx,
            async move { client.delete_project(tenant, project).await },
            move |this, (), cx| {
                this.notice(t!("projects.overview.deleted"));
                // 消えたプロジェクトの内容を残さない（shell が別の画面へ移す）。
                this.clear_loaded();
                this.project = None;
                cx.emit(ProjectSettingsEvent::Deleted(project));
            },
        );
    }

    pub(crate) fn overview_page(&self, c: &Colors, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let Some(info) = self.info.clone() else {
            return vec![card(c, vec![placeholder(c, t!("projects.common.loading"))])];
        };
        let busy = self.busy > 0;
        let f = &self.forms;
        let mut out = vec![card(
            c,
            vec![
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .p_4()
                    .child(
                        div()
                            .flex()
                            .gap_3()
                            .child(div().w(px(96.)).child(field(
                                c,
                                t!("projects.overview.emoji"),
                                Input::new(&f.emoji),
                            )))
                            .child(div().flex_1().child(field(
                                c,
                                t!("projects.overview.name"),
                                Input::new(&f.name),
                            ))),
                    )
                    .child(field(
                        c,
                        t!("projects.overview.description"),
                        Textarea::new(&f.description).h(px(96.)),
                    ))
                    .child(
                        div().flex().justify_end().child(
                            Button::new("project-save")
                                .small()
                                .primary()
                                .icon(IconName::Check)
                                .disabled(busy)
                                .label(t!("projects.common.save"))
                                .on_click(cx.listener(|this, _, _, cx| this.save_overview(cx))),
                        ),
                    )
                    .into_any_element(),
                row(
                    c,
                    Some(Icon::new(IconName::Tag).size_4().into_any_element()),
                    t!("projects.overview.key"),
                    Some(t!("projects.overview.key_desc").into()),
                    div()
                        .flex()
                        .gap_1()
                        .child(badge(c, info.key.clone(), Some(c.accent)))
                        .children(
                            info.is_personal
                                .then(|| badge(c, t!("projects.overview.personal"), None)),
                        ),
                ),
            ],
        )];
        // 個人プロジェクトは backend が削除させない（自動生成されるため）。
        if !info.is_personal {
            let typed = f.delete_confirm.read(cx).value().trim().to_string();
            let matches = typed == info.key;
            out.push(heading(c, t!("projects.overview.danger")));
            out.push(card(
                c,
                vec![
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .p_4()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(c.danger)
                                .child(t!("projects.overview.delete")),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(c.text_muted)
                                .child(t!("projects.overview.delete_desc", key = info.key)),
                        )
                        .child(
                            div()
                                .flex()
                                .gap_2()
                                .child(div().flex_1().child(Input::new(&f.delete_confirm)))
                                .child(
                                    Button::new("project-delete")
                                        .small()
                                        .danger()
                                        .disabled(!matches || busy)
                                        .label(t!("projects.overview.delete"))
                                        .on_click(
                                            cx.listener(|this, _, _, cx| this.delete_project(cx)),
                                        ),
                                ),
                        )
                        .into_any_element(),
                ],
            ));
        }
        out
    }
}
