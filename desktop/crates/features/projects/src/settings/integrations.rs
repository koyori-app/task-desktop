//! 通知設定（このプロジェクトで記録・メール送信するイベント）と GitHub 連携。

use api::types::NotificationSettingsResponse;
use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::Button;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Disableable, Icon};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use i18n::t;

use super::{ProjectSettingsView, Section};
use crate::ui::{Colors, badge, card, heading, placeholder, row};

/// backend `common::notifications` の既知の種別（表示順）。
/// `review_round_any` は「全ラウンドを受け取る」購読印で、アプリ内通知の受信者にだけ効く。
const EVENTS: [(&str, &str, &str); 9] = [
    (
        "assigned",
        "projects.notify.assigned",
        "projects.notify.assigned_desc",
    ),
    (
        "mentioned",
        "projects.notify.mentioned",
        "projects.notify.mentioned_desc",
    ),
    (
        "status_changed",
        "projects.notify.status_changed",
        "projects.notify.status_changed_desc",
    ),
    (
        "comment_added",
        "projects.notify.comment_added",
        "projects.notify.comment_added_desc",
    ),
    (
        "deadline_soon",
        "projects.notify.deadline_soon",
        "projects.notify.deadline_soon_desc",
    ),
    (
        "pr_merged",
        "projects.notify.pr_merged",
        "projects.notify.pr_merged_desc",
    ),
    (
        "review_round_created",
        "projects.notify.review_round_created",
        "projects.notify.review_round_created_desc",
    ),
    (
        "review_finding_changed",
        "projects.notify.review_finding_changed",
        "projects.notify.review_finding_changed_desc",
    ),
    (
        "review_round_any",
        "projects.notify.review_round_any",
        "projects.notify.review_round_any_desc",
    ),
];

/// 一覧の `event` を `on` に合わせる（順序は保ち、重複させない）。
fn toggle(list: &mut Vec<String>, event: &str, on: bool) {
    list.retain(|e| e != event);
    if on {
        list.push(event.to_string());
    }
}

impl ProjectSettingsView {
    fn set_notification(
        &mut self,
        event: &'static str,
        email: bool,
        on: bool,
        cx: &mut Context<Self>,
    ) {
        let Some((client, _, project)) = self.ctx() else {
            return;
        };
        let Some(current) = self.notification_settings.clone() else {
            return;
        };
        let mut next = current.clone();
        toggle(
            if email {
                &mut next.email_events
            } else {
                &mut next.in_app_events
            },
            event,
            on,
        );
        // 先に反映し、失敗したら元へ戻す。
        self.notification_settings = Some(next.clone());
        self.call(
            cx,
            async move {
                let res = client.update_notification_settings(project, &next).await;
                Ok::<_, api::ApiError>((res, current))
            },
            move |this, (res, previous): (api::Result<NotificationSettingsResponse>, _), _| {
                match res {
                    Ok(saved) => this.notification_settings = Some(saved),
                    Err(e) => {
                        this.notification_settings = Some(previous);
                        this.notice(e.to_string());
                    }
                }
            },
        );
    }

    pub(crate) fn notifications_page(&self, c: &Colors, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let Some(settings) = &self.notification_settings else {
            return vec![card(
                c,
                vec![placeholder(
                    c,
                    if self.is_loading(Section::Notifications) {
                        t!("projects.common.loading")
                    } else {
                        t!("projects.notify.unavailable")
                    },
                )],
            )];
        };
        let header = div()
            .flex()
            .px_4()
            .py_2()
            .text_xs()
            .text_color(c.text_muted)
            .child(div().flex_1().child(t!("projects.notify.event")))
            .child(div().w(px(72.)).child(t!("projects.notify.in_app")))
            .child(div().w(px(72.)).child(t!("projects.notify.email")))
            .into_any_element();
        let mut rows = vec![header];
        for (event, label, desc) in EVENTS {
            let in_app = settings.in_app_events.iter().any(|e| e == event);
            let email = settings.email_events.iter().any(|e| e == event);
            let subscription_only = event == "review_round_any";
            rows.push(row(
                c,
                None,
                i18n::tr(label),
                Some(i18n::tr(desc).into()),
                div()
                    .flex()
                    .child(
                        div().w(px(72.)).child(
                            Switch::new(SharedString::from(format!("notify-app-{event}")))
                                .checked(in_app)
                                .on_click(cx.listener(move |this, v: &bool, _, cx| {
                                    this.set_notification(event, false, *v, cx)
                                })),
                        ),
                    )
                    .child(div().w(px(72.)).when(!subscription_only, |d| {
                        d.child(
                            Switch::new(SharedString::from(format!("notify-mail-{event}")))
                                .checked(email)
                                .on_click(cx.listener(move |this, v: &bool, _, cx| {
                                    this.set_notification(event, true, *v, cx)
                                })),
                        )
                    })),
            ));
        }
        vec![
            div()
                .text_sm()
                .text_color(c.text_muted)
                .child(t!("projects.notify.desc"))
                .into_any_element(),
            card(c, rows),
        ]
    }

    fn connect_github(&mut self, cx: &mut Context<Self>) {
        let Some((client, tenant, project)) = self.ctx() else {
            return;
        };
        self.call(
            cx,
            async move { client.start_github_install(tenant, project).await },
            |this, install, _| match ::platform::open_url(&install.url) {
                Ok(()) => this.notice(t!("projects.github.opened")),
                Err(e) => this.notice(e.to_string()),
            },
        );
    }

    pub(crate) fn github_page(&self, c: &Colors, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let busy = self.busy > 0;
        let Some(github) = &self.github else {
            return vec![card(
                c,
                vec![placeholder(
                    c,
                    if self.is_loading(Section::Github) {
                        t!("projects.common.loading")
                    } else {
                        t!("projects.github.unavailable")
                    },
                )],
            )];
        };
        let repo = match (&github.repo_owner, &github.repo_name) {
            (Some(o), Some(n)) => Some(format!("{o}/{n}")),
            _ => None,
        };
        let status = if github.connected {
            badge(c, t!("projects.github.connected"), Some(c.success))
        } else {
            badge(c, t!("projects.github.not_connected"), None)
        };
        let description = if github.connected {
            github.connected_at.map(|at| {
                t!(
                    "projects.github.connected_at",
                    at = core::timezone::local(&at).format("%Y-%m-%d %H:%M")
                )
            })
        } else {
            Some(t!("projects.github.connect_desc").into())
        };
        vec![
            card(
                c,
                vec![row(
                    c,
                    Some(Icon::new(IconName::Github).size_5().into_any_element()),
                    repo.unwrap_or_else(|| t!("projects.github.no_repo").into()),
                    description,
                    div().flex().items_center().gap_2().child(status).child(
                        Button::new("github-connect")
                            .outline()
                            .small()
                            .icon(IconName::ExternalLink)
                            .disabled(busy)
                            .label(if github.connected {
                                t!("projects.github.reconnect")
                            } else {
                                t!("projects.github.connect")
                            })
                            .on_click(cx.listener(|this, _, _, cx| this.connect_github(cx))),
                    ),
                )],
            ),
            heading(c, t!("projects.github.hint_title")),
            div()
                .text_sm()
                .text_color(c.text_muted)
                .child(t!("projects.github.hint"))
                .into_any_element(),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::{EVENTS, toggle};

    /// `tr` で引くキーは i18n の走査対象外なので、ここで存在を確かめる。
    #[test]
    fn every_event_has_text() {
        for (_, label, desc) in EVENTS {
            assert_ne!(i18n::tr(label), label);
            assert_ne!(i18n::tr(desc), desc);
        }
    }

    #[test]
    fn toggle_adds_once_and_removes() {
        let mut list = vec!["assigned".to_string()];
        toggle(&mut list, "mentioned", true);
        toggle(&mut list, "mentioned", true);
        assert_eq!(list, ["assigned", "mentioned"]);
        toggle(&mut list, "assigned", false);
        assert_eq!(list, ["mentioned"]);
    }
}
