//! プロジェクト作成ダイアログ（名前・キー・説明）。

use std::rc::Rc;

use api::types::{CreateProjectRequest, ProjectResponse};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Disableable, WindowExt};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use i18n::t;
use uuid::Uuid;

use crate::ui::field;

type OnCreated = Rc<dyn Fn(ProjectResponse, &mut Window, &mut App)>;

struct CreateProjectForm {
    client: api::Client,
    tenant: Uuid,
    name: Entity<InputState>,
    key: Entity<InputState>,
    description: Entity<InputState>,
    submitting: bool,
    error: Option<String>,
    on_created: OnCreated,
    _subs: Vec<Subscription>,
}

impl CreateProjectForm {
    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.submitting {
            return;
        }
        let name = self.name.read(cx).value().trim().to_string();
        if name.is_empty() {
            self.error = Some(t!("projects.create.name_required").into());
            cx.notify();
            return;
        }
        let key = self.key.read(cx).value().trim().to_uppercase();
        let description = self.description.read(cx).value().trim().to_string();
        let req = CreateProjectRequest {
            name,
            key: (!key.is_empty()).then_some(key),
            description: (!description.is_empty()).then_some(description),
            icon_emoji: None,
            icon_url: None,
        };
        self.submitting = true;
        self.error = None;
        cx.notify();
        let client = self.client.clone();
        let tenant = self.tenant;
        cx.spawn_in(window, async move |this, cx| {
            let res = client.create_project(tenant, &req).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.submitting = false;
                match res {
                    Ok(project) => {
                        window.close_dialog(cx);
                        (this.on_created)(project, window, cx);
                    }
                    Err(e) => this.error = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for CreateProjectForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let c = ::ui::colors(cx);
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(field(
                &c,
                t!("projects.create.name"),
                Input::new(&self.name),
            ))
            .child(field(&c, t!("projects.create.key"), Input::new(&self.key)))
            .child(
                div()
                    .text_xs()
                    .text_color(c.text_muted)
                    .child(t!("projects.create.key_hint")),
            )
            .child(field(
                &c,
                t!("projects.create.description"),
                Input::new(&self.description),
            ))
            .when_some(self.error.clone(), |d, e| {
                d.child(div().text_sm().text_color(c.danger).child(e))
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("create-project-cancel")
                            .small()
                            .ghost()
                            .label(t!("projects.common.cancel"))
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("create-project-submit")
                            .small()
                            .primary()
                            .disabled(self.submitting)
                            .label(if self.submitting {
                                t!("projects.create.creating")
                            } else {
                                t!("projects.create.submit")
                            })
                            .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
                    ),
            )
    }
}

/// 作成ダイアログを開く。成功したらダイアログを閉じて `on_created` を呼ぶ。
/// Enter で作成、Esc で閉じる。
pub fn open_create_project(
    client: api::Client,
    tenant: Uuid,
    window: &mut Window,
    cx: &mut App,
    on_created: impl Fn(ProjectResponse, &mut Window, &mut App) + 'static,
) {
    let on_created: OnCreated = Rc::new(on_created);
    let form = cx.new(|cx| {
        let name =
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("projects.create.name_ph")));
        let key =
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("projects.create.key_ph")));
        let description = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("projects.create.description_ph"))
        });
        let subs = [&name, &key, &description]
            .into_iter()
            .map(|input| {
                cx.subscribe_in(
                    input,
                    window,
                    |this: &mut CreateProjectForm, _, ev, window, cx| {
                        if matches!(ev, InputEvent::PressEnter { .. }) {
                            this.submit(window, cx);
                        }
                    },
                )
            })
            .collect();
        CreateProjectForm {
            client,
            tenant,
            name,
            key,
            description,
            submitting: false,
            error: None,
            on_created,
            _subs: subs,
        }
    });
    let focus = form.read(cx).name.clone();
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .width(px(440.))
            .title(t!("projects.create.title"))
            .child(form.clone())
    });
    window.defer(cx, move |window, cx| {
        focus.update(cx, |input, cx| input.focus(window, cx));
    });
}
