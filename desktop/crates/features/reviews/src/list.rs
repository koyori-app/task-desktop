//! §16 PR 一覧。blocking > 0 は危険色、unresolved/rounds を出す。

use api::Client;
use api::types::{CreateReviewRequest, CreateReviewRequestHeadSha, ReviewedPullRequest};
use gpui_kit::assets::IconName;
use gpui_kit::component::Theme;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub enum ReviewListEvent {
    /// PR 行クリック → Detail で summary/findings を開く。
    Select { pr: i64, title: Option<String> },
}

pub struct ReviewListView {
    client: Option<Client>,
    tenant: Option<Uuid>,
    project: Option<Uuid>,
    prs: Vec<ReviewedPullRequest>,
    loading: bool,
    error: Option<String>,
    /// レビュー作成フォーム（§16 末尾: 手動トリガ）。
    pr_input: Entity<InputState>,
    sha_input: Entity<InputState>,
    show_create: bool,
}

impl ReviewListView {
    pub fn new(
        client: Option<Client>,
        tenant: Option<Uuid>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            client,
            tenant,
            project: None,
            prs: vec![],
            loading: false,
            error: None,
            pr_input: cx.new(|cx| InputState::new(window, cx).placeholder("PR #")),
            sha_input: cx.new(|cx| InputState::new(window, cx).placeholder("head SHA")),
            show_create: false,
        }
    }

    pub fn set_client(&mut self, client: Client, tenant: Option<Uuid>) {
        self.client = Some(client);
        self.tenant = tenant;
    }

    pub fn set_project(&mut self, project: Uuid, cx: &mut Context<Self>) {
        self.project = Some(project);
        self.reload(cx);
    }

    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let (Some(client), Some(tenant), Some(project)) =
            (self.client.clone(), self.tenant, self.project)
        else {
            return;
        };
        self.loading = true;
        self.error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let res = client.list_reviewed_pull_requests(tenant, project).await;
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                match res {
                    Ok(prs) => this.prs = prs,
                    Err(e) => this.error = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn create_review(&mut self, cx: &mut Context<Self>) {
        let (Some(client), Some(tenant), Some(project)) =
            (self.client.clone(), self.tenant, self.project)
        else {
            return;
        };
        let pr_text = self.pr_input.read(cx).value().trim().to_string();
        let sha = self.sha_input.read(cx).value().trim().to_string();
        let Ok(pr_number) = pr_text.parse::<i32>() else {
            self.error = Some("PR number must be an integer".into());
            cx.notify();
            return;
        };
        let Ok(head_sha) = CreateReviewRequestHeadSha::try_from(sha.clone()) else {
            self.error = Some("Invalid head SHA".into());
            cx.notify();
            return;
        };
        cx.spawn(async move |this, cx| {
            let res = client
                .create_review(
                    tenant,
                    project,
                    &CreateReviewRequest {
                        findings: vec![],
                        head_sha,
                        pr_number,
                        summary: None,
                    },
                )
                .await;
            let _ = this.update(cx, |this, cx| {
                match res {
                    Ok(_) => {
                        this.show_create = false;
                        this.reload(cx);
                    }
                    Err(e) => this.error = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn row(&self, pr: &ReviewedPullRequest, ix: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let (c, danger, warn) = {
            let t = Theme::global(cx);
            let cc = t.semantic_tokens().colors.clone();
            (cc, t.danger, t.warning)
        };
        let blocked = pr.blocking > 0;
        let n = pr.pr_number as i64;
        let title = pr
            .pr_title
            .clone()
            .unwrap_or_else(|| format!("PR #{n}"));
        let title_opt = pr.pr_title.clone();

        div()
            .id(("pr-row", ix))
            .flex()
            .flex_row()
            .items_center()
            .gap_3()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(c.border)
            .hover(|s| s.bg(c.muted))
            .cursor_pointer()
            .on_click(cx.listener(move |_, _, _, cx| {
                cx.emit(ReviewListEvent::Select {
                    pr: n,
                    title: title_opt.clone(),
                });
            }))
            .child(
                div()
                    .text_xs()
                    .text_color(c.muted_foreground)
                    .w(px(56.))
                    .flex_shrink_0()
                    .child(format!("#{n}")),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .child(title),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(c.muted_foreground)
                    .child(pr.pr_author.clone().unwrap_or_default()),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(c.muted_foreground)
                    .child(format!("{}r", pr.rounds)),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(if blocked { danger } else { warn })
                    .w(px(90.))
                    .child(if blocked {
                        format!("{} blocking", pr.blocking)
                    } else {
                        format!("{} open", pr.unresolved)
                    }),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(c.muted_foreground)
                    .child(
                        pr.last_reviewed_at
                            .format("%m-%d %H:%M")
                            .to_string(),
                    ),
            )
    }
}

impl EventEmitter<ReviewListEvent> for ReviewListView {}

impl Render for ReviewListView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (c, danger) = {
            let t = Theme::global(cx);
            (t.semantic_tokens().colors.clone(), t.danger)
        };

        let mut list = div()
            .id("review-list")
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .overflow_y_scroll();
        for (ix, pr) in self.prs.iter().enumerate() {
            list = list.child(self.row(pr, ix, cx));
        }
        if self.prs.is_empty() && !self.loading {
            list = list.child(
                div().p_8().child(
                    div()
                        .text_sm()
                        .text_color(c.muted_foreground)
                        .child("No reviewed pull requests"),
                ),
            );
        }
        if self.loading {
            list = list.child(
                div().p_4().child(
                    div()
                        .text_xs()
                        .text_color(c.muted_foreground)
                        .child("Loading…"),
                ),
            );
        }

        div()
            .flex()
            .flex_col()
            .size_full()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .px_4()
                    .py_2()
                    .border_b_1()
                    .border_color(c.border)
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Reviews"),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("rv-refresh")
                            .ghost()
                            .icon(IconName::RefreshCcwDot)
                            .on_click(cx.listener(|this, _, _, cx| this.reload(cx))),
                    )
                    .child(
                        Button::new("rv-new")
                            .ghost()
                            .icon(IconName::Plus)
                            .label("New review")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.show_create = !this.show_create;
                                cx.notify();
                            })),
                    ),
            )
            .when(self.show_create, |d| {
                d.child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .px_4()
                        .py_2()
                        .border_b_1()
                        .border_color(c.border)
                        .child(div().w(px(100.)).child(Input::new(&self.pr_input)))
                        .child(div().flex_1().child(Input::new(&self.sha_input)))
                        .child(
                            Button::new("rv-create")
                                .ghost()
                                .label("Create")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.create_review(cx)
                                })),
                        ),
                )
            })
            .when_some(self.error.clone(), |d, e| {
                d.child(
                    div().px_4().py_2().child(
                        div().text_sm().text_color(danger).child(e),
                    ),
                )
            })
            .child(list)
    }
}
