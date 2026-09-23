//! §15 Detail ペイン。Status / Priority / Assignee / DueDate / done の
//! 編集とコメント表示・投稿。更新は Optimistic + rollback（§23）。

use api::Client;
use api::types::{
    AssigneeInput, CommentThread, CreateCommentRequest, ProjectStatusResponse, TaskDetailResponse,
    TaskPriority, UpdateTaskRequest, UserSummary,
};
use chrono::{DateTime, NaiveDate, Utc};
use gpui_kit::assets::IconName;
use gpui_kit::component::Theme;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use uuid::Uuid;

const PRIORITIES: [TaskPriority; 6] = [
    TaskPriority::CriticalFire,
    TaskPriority::Critical,
    TaskPriority::High,
    TaskPriority::Medium,
    TaskPriority::Low,
    TaskPriority::Trivial,
];

pub struct TaskDetailView {
    client: Option<Client>,
    tenant: Option<Uuid>,
    project: Option<Uuid>,
    task: Option<Uuid>,
    detail: Option<TaskDetailResponse>,
    statuses: Vec<ProjectStatusResponse>,
    assignables: Vec<UserSummary>,
    comments: Vec<CommentThread>,
    loading: bool,
    error: Option<String>,
    title_input: Entity<InputState>,
    due_input: Entity<InputState>,
    comment_input: Entity<InputState>,
    /// title input をどの task まで同期したか。
    title_synced: Option<Uuid>,
    /// open() から render 側へ due input クリアを要求（subscribe 内では
    /// Window に触れないため）。
    clear_due_input: bool,
}

impl TaskDetailView {
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
            task: None,
            detail: None,
            statuses: vec![],
            assignables: vec![],
            comments: vec![],
            loading: false,
            error: None,
            title_input: cx.new(|cx| InputState::new(window, cx)),
            due_input: cx.new(|cx| InputState::new(window, cx).placeholder("YYYY-MM-DD")),
            comment_input: cx.new(|cx| InputState::new(window, cx).placeholder("Write a comment…")),
            title_synced: None,
            clear_due_input: false,
        }
    }

    /// 一覧からの選択。各種ロードを投げる。
    pub fn open(&mut self, project: Uuid, task: Uuid, cx: &mut Context<Self>) {
        self.project = Some(project);
        self.task = Some(task);
        self.detail = None;
        self.comments = vec![];
        self.error = None;
        self.title_synced = None;
        self.clear_due_input = true;
        let (Some(client), Some(tenant)) = (self.client.clone(), self.tenant) else {
            return;
        };
        self.loading = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let detail = client.get_task(tenant, project, task).await;
            let statuses = client.list_statuses(tenant, project).await;
            let assignables = client.list_assignable_users(tenant, project, None).await;
            let comments = client.list_comments(tenant, project, task).await;
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                this.detail = detail.ok();
                this.statuses = statuses.unwrap_or_default();
                this.assignables = assignables.unwrap_or_default();
                this.comments = comments.map(|c| c.comments).unwrap_or_default();
                if this.detail.is_none() {
                    this.error = Some("Failed to load task".into());
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn set_client(&mut self, client: Client, tenant: Option<Uuid>) {
        self.client = Some(client);
        self.tenant = tenant;
    }

    /// ログアウト時に呼ぶ。
    pub fn clear_client(&mut self) {
        self.client = None;
        self.detail = None;
    }

    fn is_done(&self) -> bool {
        self.detail
            .as_ref()
            .and_then(|d| {
                self.statuses
                    .iter()
                    .find(|s| s.id == d.status_id)
                    .map(|s| s.is_done_state)
            })
            .unwrap_or(false)
    }

    /// フィールド更新の共通経路。`patch` は表示用 detail への即時反映、
    /// `req` は UpdateTaskRequest への差分。失敗時は `undo` で戻す。
    fn update<F, G>(&mut self, patch: F, undo: G, req: UpdateTaskRequest, cx: &mut Context<Self>)
    where
        F: FnOnce(&mut TaskDetailResponse),
        G: FnOnce(&mut TaskDetailResponse) + Send + 'static,
    {
        let (Some(client), Some(tenant), Some(project), Some(task)) =
            (self.client.clone(), self.tenant, self.project, self.task)
        else {
            return;
        };
        if let Some(d) = self.detail.as_mut() {
            patch(d);
        }
        cx.notify();
        cx.spawn(async move |this, cx| {
            let res = client.update_task(tenant, project, task, &req).await;
            if let Err(e) = res {
                let _ = this.update(cx, |this, cx| {
                    if let Some(d) = this.detail.as_mut() {
                        undo(d);
                    }
                    this.error = Some(e.to_string());
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn set_status(&mut self, status_id: Uuid, cx: &mut Context<Self>) {
        let prev = self.detail.as_ref().map(|d| d.status_id);
        self.update(
            move |d| d.status_id = status_id,
            move |d| {
                if let Some(p) = prev {
                    d.status_id = p;
                }
            },
            UpdateTaskRequest {
                status_id: Some(status_id),
                ..Default::default()
            },
            cx,
        );
    }

    fn toggle_done(&mut self, cx: &mut Context<Self>) {
        let done = self.is_done();
        let target = self
            .statuses
            .iter()
            .find(|s| {
                if done {
                    !s.is_done_state && s.is_default
                } else {
                    s.is_done_state && s.is_default_done
                }
            })
            .or_else(|| {
                self.statuses.iter().find(|s| {
                    if done {
                        !s.is_done_state
                    } else {
                        s.is_done_state
                    }
                })
            })
            .map(|s| s.id);
        if let Some(id) = target {
            self.set_status(id, cx);
        }
    }

    fn set_priority(&mut self, priority: TaskPriority, cx: &mut Context<Self>) {
        let prev = self.detail.as_ref().map(|d| d.priority);
        self.update(
            move |d| d.priority = priority,
            move |d| {
                if let Some(p) = prev {
                    d.priority = p;
                }
            },
            UpdateTaskRequest {
                priority: Some(priority),
                ..Default::default()
            },
            cx,
        );
    }

    /// 担当者の追加/削除。assignees は全体置換 API。
    fn toggle_assignee(&mut self, user: &UserSummary, cx: &mut Context<Self>) {
        let Some(d) = self.detail.as_ref() else {
            return;
        };
        let mut list: Vec<AssigneeInput> = d
            .assignees
            .iter()
            .map(|a| AssigneeInput {
                role: a.role.clone(),
                user_id: a.user.id,
            })
            .collect();
        if let Some(pos) = list.iter().position(|a| a.user_id == user.id) {
            list.remove(pos);
        } else {
            list.push(AssigneeInput {
                role: "assignee".into(),
                user_id: user.id,
            });
        }
        let snapshot = list.clone();
        self.update(
            |d| {
                // assignees の表示側 patch は UserSummary が要るため省略
                // （成功後の reload/次回 open で整合）。
                let _ = d;
            },
            move |d| {
                let _ = d;
            },
            UpdateTaskRequest {
                assignees: Some(snapshot),
                ..Default::default()
            },
            cx,
        );
    }

    fn apply_due(&mut self, cx: &mut Context<Self>) {
        let text = self.due_input.read(cx).value().trim().to_string();
        let req = if text.is_empty() {
            UpdateTaskRequest {
                clear_soft_deadline: Some(true),
                ..Default::default()
            }
        } else {
            let Ok(d) = NaiveDate::parse_from_str(&text, "%Y-%m-%d") else {
                self.error = Some("Due date must be YYYY-MM-DD".into());
                cx.notify();
                return;
            };
            let dt =
                DateTime::<Utc>::from_naive_utc_and_offset(d.and_hms_opt(0, 0, 0).unwrap(), Utc);
            UpdateTaskRequest {
                soft_deadline: Some(dt),
                ..Default::default()
            }
        };
        let due = req.soft_deadline;
        let prev = self.detail.as_ref().and_then(|d| d.soft_deadline);
        self.update(
            move |d| d.soft_deadline = due,
            move |d| d.soft_deadline = prev,
            req,
            cx,
        );
    }

    fn save_title(&mut self, cx: &mut Context<Self>) {
        let title = self.title_input.read(cx).value().trim().to_string();
        if title.is_empty() {
            return;
        }
        let t = title.clone();
        let prev = self.detail.as_ref().map(|d| d.title.clone());
        self.update(
            move |d| d.title = t.clone(),
            move |d| {
                if let Some(p) = prev.clone() {
                    d.title = p;
                }
            },
            UpdateTaskRequest {
                title: Some(title),
                ..Default::default()
            },
            cx,
        );
    }

    fn post_comment(&mut self, cx: &mut Context<Self>) {
        let body = self.comment_input.read(cx).value().trim().to_string();
        if body.is_empty() {
            return;
        }
        let (Some(client), Some(tenant), Some(project), Some(task)) =
            (self.client.clone(), self.tenant, self.project, self.task)
        else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let res = client
                .create_comment(
                    tenant,
                    project,
                    task,
                    &CreateCommentRequest {
                        body,
                        parent_comment_id: None,
                    },
                )
                .await;
            let comments = if res.is_ok() {
                client.list_comments(tenant, project, task).await.ok()
            } else {
                None
            };
            let _ = this.update(cx, |this, cx| {
                if let Some(c) = comments {
                    this.comments = c.comments;
                }
                if let Err(e) = res {
                    this.error = Some(e.to_string());
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn chip(
        label: impl Into<SharedString>,
        active: bool,
        cx: &mut Context<Self>,
        on_click: impl Fn(&mut Self, &mut Context<Self>) + 'static,
    ) -> Stateful<Div> {
        let c = Theme::global(cx).semantic_tokens().colors;
        div()
            .id(ElementId::Name(label.into()))
            .px_2()
            .py_1()
            .rounded_md()
            .text_xs()
            .cursor_pointer()
            .when(active, |d| d.bg(c.accent).text_color(c.accent_foreground))
            .when(!active, |d| {
                d.bg(c.secondary)
                    .text_color(c.secondary_foreground)
                    .hover(|s| s.bg(c.muted))
            })
            .on_click(cx.listener(move |this, _, _, cx| on_click(this, cx)))
    }
}

impl Render for TaskDetailView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (c, danger) = {
            let t = Theme::global(cx);
            (t.semantic_tokens().colors, t.danger)
        };

        let Some(detail) = self.detail.clone() else {
            return div().id("task-detail-empty").size_full().p_4().child(
                div()
                    .text_sm()
                    .text_color(c.muted_foreground)
                    .child(if self.loading {
                        "Loading…"
                    } else {
                        "Select an item"
                    }),
            );
        };

        if self.clear_due_input {
            self.clear_due_input = false;
            self.due_input
                .update(cx, |s, cx| s.set_value("", window, cx));
        }
        // title input は task が変わった時だけ detail のタイトルに同期。
        if self.title_synced != Some(detail.id) {
            self.title_synced = Some(detail.id);
            let t = detail.title.clone();
            self.title_input
                .update(cx, |s, cx| s.set_value(t, window, cx));
        }

        let status_name = self
            .statuses
            .iter()
            .find(|s| s.id == detail.status_id)
            .map(|s| s.name.clone())
            .unwrap_or_default();
        let done = self.is_done();
        let cur_status = detail.status_id;
        let cur_priority = detail.priority;
        let assignee_ids: Vec<Uuid> = detail.assignees.iter().map(|a| a.user.id).collect();

        let mut status_row = div().flex().flex_row().flex_wrap().gap_1();
        for s in self.statuses.clone() {
            let id = s.id;
            status_row = status_row.child(Self::chip(
                format!("{}-st-{}", s.name, id.simple()),
                cur_status == id,
                cx,
                move |this, cx| this.set_status(id, cx),
            ));
        }

        let mut prio_row = div().flex().flex_row().flex_wrap().gap_1();
        for p in PRIORITIES {
            prio_row = prio_row.child(Self::chip(
                format!("{}-pr", p),
                cur_priority == p,
                cx,
                move |this, cx| this.set_priority(p, cx),
            ));
        }

        let mut assignee_row = div().flex().flex_row().flex_wrap().gap_1();
        for u in self
            .assignables
            .iter()
            .take(16)
            .cloned()
            .collect::<Vec<_>>()
        {
            let active = assignee_ids.contains(&u.id);
            assignee_row = assignee_row.child(Self::chip(
                format!("{}-as-{}", u.username, u.id.simple()),
                active,
                cx,
                move |this, cx| this.toggle_assignee(&u, cx),
            ));
        }

        div()
            .id("task-detail")
            .flex()
            .flex_col()
            .size_full()
            .overflow_y_scroll()
            .p_4()
            .gap_3()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .text_xs()
                            .text_color(c.muted_foreground)
                            .child(format!("#{}", detail.seq_id)),
                    )
                    .child(div().flex_1().child(Input::new(&self.title_input)))
                    .child(
                        Button::new("save-title")
                            .ghost()
                            .icon(IconName::Check)
                            .on_click(cx.listener(|this, _, _, cx| this.save_title(cx))),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .text_xs()
                            .text_color(c.muted_foreground)
                            .w(px(70.))
                            .child("Status"),
                    )
                    .child(div().text_sm().child(status_name))
                    .child(
                        Button::new("done-toggle")
                            .ghost()
                            .label(if done { "Done" } else { "Mark done" })
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_done(cx))),
                    ),
            )
            .child(section(c.muted_foreground, "Status", status_row))
            .child(section(c.muted_foreground, "Priority", prio_row))
            .child(section(c.muted_foreground, "Assignees", assignee_row))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .text_xs()
                            .text_color(c.muted_foreground)
                            .w(px(70.))
                            .child("Due"),
                    )
                    .child(div().w(px(140.)).child(Input::new(&self.due_input)))
                    .child(
                        Button::new("apply-due")
                            .ghost()
                            .label("Apply")
                            .on_click(cx.listener(|this, _, _, cx| this.apply_due(cx))),
                    ),
            )
            .when_some(detail.description.clone(), |d, desc| {
                d.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_xs()
                                .text_color(c.muted_foreground)
                                .child("Description"),
                        )
                        .child(div().text_sm().child(desc)),
                )
            })
            .when_some(self.error.clone(), |d, e| {
                d.child(div().text_sm().text_color(danger).child(e))
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .pt_2()
                    .border_t_1()
                    .border_color(c.border)
                    .child(
                        div()
                            .text_xs()
                            .text_color(c.muted_foreground)
                            .child("Comments"),
                    )
                    .children(self.comments.iter().map(|cm| {
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .py_1()
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_xs()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(cm.user.name.clone()),
                                    )
                                    .child(
                                        div().text_xs().text_color(c.muted_foreground).child(
                                            cm.created_at.format("%Y-%m-%d %H:%M").to_string(),
                                        ),
                                    ),
                            )
                            .child(div().text_sm().child(cm.body.clone().unwrap_or_default()))
                    }))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .child(div().flex_1().child(Input::new(&self.comment_input)))
                            .child(
                                Button::new("post-comment")
                                    .ghost()
                                    .label("Post")
                                    .on_click(cx.listener(|this, _, _, cx| this.post_comment(cx))),
                            ),
                    ),
            )
    }
}

fn section(muted: Hsla, label: &'static str, row: Div) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(div().text_xs().text_color(muted).child(label))
        .child(row)
}
