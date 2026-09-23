//! §15 Detail ペイン。Status / Priority / Assignee / DueDate / done の
//! 編集とコメント表示・投稿。更新は Optimistic + rollback（§23）。

use crate::model::due_timestamp;
use api::Client;
use api::types::{
    AssigneeInput, CommentThread, CreateCommentRequest, ProjectStatusResponse, TaskAssigneeSummary,
    TaskDetailResponse, TaskPriority, UpdateTaskRequest, UserSummary,
};
use chrono::Local;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::text::TextView;
use gpui_kit::component::{Disableable, Selectable, Theme, WindowExt};
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

#[derive(Debug, Clone)]
pub enum TaskDetailEvent {
    Updated { project: Uuid, task: Uuid },
}

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
    updating: bool,
    generation: u64,
    client_generation: u64,
    error: Option<String>,
    title_input: Entity<InputState>,
    due_input: Entity<InputState>,
    comment_input: Entity<InputState>,
    description_input: Entity<TextareaState>,
    editing_description: bool,
    posting_comment: bool,
    clear_comment_input: bool,
    shown_error: Option<String>,
    _subs: Vec<Subscription>,
    /// title input をどの task まで同期したか。
    title_synced: Option<Uuid>,
}

impl TaskDetailView {
    pub fn new(
        client: Option<Client>,
        tenant: Option<Uuid>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let title_input = cx.new(|cx| InputState::new(window, cx).placeholder("Task title"));
        let due_input = cx.new(|cx| InputState::new(window, cx).placeholder("YYYY-MM-DD"));
        let comment_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Write a comment…"));
        let subs = vec![
            cx.subscribe(&title_input, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.save_title(cx);
                }
            }),
            cx.subscribe(&due_input, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.apply_due(cx);
                }
            }),
            cx.subscribe(&comment_input, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.post_comment(cx);
                }
            }),
        ];
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
            updating: false,
            generation: 0,
            client_generation: 0,
            error: None,
            title_input,
            due_input,
            comment_input,
            description_input: cx
                .new(|cx| TextareaState::new(window, cx).placeholder("Description (Markdown)")),
            editing_description: false,
            posting_comment: false,
            clear_comment_input: false,
            shown_error: None,
            _subs: subs,
            title_synced: None,
        }
    }

    /// 一覧からの選択。各種ロードを投げる。
    pub fn open(&mut self, project: Uuid, task: Uuid, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        self.updating = false;
        self.project = Some(project);
        self.task = Some(task);
        self.detail = None;
        self.statuses.clear();
        self.assignables.clear();
        self.comments = vec![];
        self.error = None;
        self.title_synced = None;
        self.clear_comment_input = true;
        self.editing_description = false;
        self.posting_comment = false;
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
                if this.generation != generation
                    || this.project != Some(project)
                    || this.task != Some(task)
                    || this.tenant != Some(tenant)
                    || this.client.is_none()
                {
                    return;
                }
                this.loading = false;
                match detail {
                    Ok(detail) => this.detail = Some(detail),
                    Err(api::ApiError::NotFound) => {
                        this.error = Some("This task no longer exists.".into())
                    }
                    Err(e) => this.error = Some(e.to_string()),
                }
                match statuses {
                    Ok(s) => this.statuses = s,
                    Err(e) => {
                        if this.detail.is_some() {
                            this.error = Some(format!("Could not load statuses: {e}"));
                        }
                    }
                }
                match assignables {
                    Ok(users) => this.assignables = users,
                    Err(e) => {
                        if this.detail.is_some() {
                            this.error = Some(format!("Could not load assignees: {e}"));
                        }
                    }
                }
                match comments {
                    Ok(comments) => this.comments = comments.comments,
                    Err(e) => {
                        if this.detail.is_some() {
                            this.error = Some(format!("Could not load comments: {e}"));
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn set_client(&mut self, client: Client, tenant: Option<Uuid>) {
        if !self
            .client
            .as_ref()
            .is_some_and(|current| current.same_session(&client))
        {
            self.client_generation += 1;
        }
        if self.tenant != tenant {
            self.clear_client();
            self.title_synced = None;
            self.clear_comment_input = true;
            self.error = None;
            self.loading = false;
        }
        self.client = Some(client);
        self.tenant = tenant;
    }

    /// ログアウト時に呼ぶ。
    pub fn clear_client(&mut self) {
        self.client_generation += 1;
        self.generation += 1;
        self.updating = false;
        self.client = None;
        self.detail = None;
        self.comments.clear();
        self.statuses.clear();
        self.assignables.clear();
        self.project = None;
        self.task = None;
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
        if self.updating {
            return;
        }
        let (Some(client), Some(tenant), Some(project), Some(task)) =
            (self.client.clone(), self.tenant, self.project, self.task)
        else {
            return;
        };
        if let Some(d) = self.detail.as_mut() {
            patch(d);
        }
        self.updating = true;
        self.error = None;
        let generation = self.generation;
        let client_generation = self.client_generation;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let res = client.update_task(tenant, project, task, &req).await;
            let _ = this.update(cx, |this, cx| {
                if this.client_generation != client_generation
                    || this.tenant != Some(tenant)
                    || this.client.is_none()
                {
                    return;
                }
                // The mutation still changed the list even if another task is now selected.
                if res.is_ok() {
                    cx.emit(TaskDetailEvent::Updated { project, task });
                }
                if this.generation != generation
                    || this.project != Some(project)
                    || this.task != Some(task)
                    || this.tenant != Some(tenant)
                    || this.client.is_none()
                {
                    return;
                }
                this.updating = false;
                if let Err(e) = res {
                    if let Some(d) = this.detail.as_mut() {
                        undo(d);
                    }
                    this.error = Some(e.to_string());
                    this.title_synced = None;
                } else {
                    this.error = None;
                }
                cx.notify();
            });
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
        let prev = d.assignees.clone();
        let mut next = prev.clone();
        if let Some(pos) = next.iter().position(|a| a.user.id == user.id) {
            next.remove(pos);
        } else {
            next.push(TaskAssigneeSummary {
                role: "assignee".into(),
                user: user.clone(),
            });
        }
        self.update(
            move |d| d.assignees = next,
            move |d| d.assignees = prev,
            UpdateTaskRequest {
                assignees: Some(list),
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
                clear_hard_deadline: Some(true),
                ..Default::default()
            }
        } else {
            let Some(dt) = due_timestamp(&text, &Local) else {
                self.error = Some("Due date must be YYYY-MM-DD".into());
                cx.notify();
                return;
            };
            UpdateTaskRequest {
                soft_deadline: Some(dt),
                ..Default::default()
            }
        };
        let due = req.soft_deadline;
        let prev = self.detail.as_ref().and_then(|d| d.soft_deadline);
        let prev_hard = self.detail.as_ref().and_then(|d| d.hard_deadline);
        let clear_hard = req.clear_hard_deadline == Some(true);
        self.update(
            move |d| {
                d.soft_deadline = due;
                if clear_hard {
                    d.hard_deadline = None;
                }
            },
            move |d| {
                d.soft_deadline = prev;
                d.hard_deadline = prev_hard;
            },
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
        if self.posting_comment {
            return;
        }
        let body = self.comment_input.read(cx).value().trim().to_string();
        if body.is_empty() {
            return;
        }
        let (Some(client), Some(tenant), Some(project), Some(task)) =
            (self.client.clone(), self.tenant, self.project, self.task)
        else {
            return;
        };
        self.posting_comment = true;
        let generation = self.generation;
        self.error = None;
        cx.notify();
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
                if this.generation != generation
                    || this.project != Some(project)
                    || this.task != Some(task)
                    || this.tenant != Some(tenant)
                    || this.client.is_none()
                {
                    return;
                }
                this.posting_comment = false;
                if let Some(c) = comments {
                    this.comments = c.comments;
                }
                if let Err(e) = res {
                    this.error = Some(e.to_string());
                } else {
                    this.clear_comment_input = true;
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn save_description(&mut self, cx: &mut Context<Self>) {
        let text = self.description_input.read(cx).value().to_string();
        let next = text.clone();
        let prev = self.detail.as_ref().and_then(|d| d.description.clone());
        self.editing_description = false;
        self.update(
            move |d| d.description = Some(next),
            move |d| d.description = prev,
            UpdateTaskRequest {
                description: Some(text),
                ..Default::default()
            },
            cx,
        );
    }

    fn chip(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        active: bool,
        cx: &mut Context<Self>,
        on_click: impl Fn(&mut Self, &mut Context<Self>) + 'static,
    ) -> Button {
        Button::new(id)
            .compact()
            .label(label)
            .selected(active)
            .on_click(cx.listener(move |this, _, _, cx| on_click(this, cx)))
    }
}

impl Render for TaskDetailView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (c, danger) = {
            let t = Theme::global(cx);
            (t.semantic_tokens().colors, t.danger)
        };
        if self.error != self.shown_error {
            self.shown_error = self.error.clone();
            if let Some(error) = &self.error {
                window.push_notification(Notification::new().message(error.clone()), cx);
            }
        }

        let Some(detail) = self.detail.clone() else {
            return div().id("task-detail-empty").size_full().p_4().child(
                div()
                    .text_sm()
                    .text_color(c.muted_foreground)
                    .child(if self.loading {
                        "Loading…".to_string()
                    } else {
                        self.error
                            .clone()
                            .unwrap_or_else(|| "Select a task to view its details".into())
                    }),
            );
        };

        if self.clear_comment_input {
            self.clear_comment_input = false;
            self.comment_input
                .update(cx, |s, cx| s.set_value("", window, cx));
        }
        // title input は task が変わった時だけ detail のタイトルに同期。
        if self.title_synced != Some(detail.id) {
            self.title_synced = Some(detail.id);
            let t = detail.title.clone();
            self.title_input
                .update(cx, |s, cx| s.set_value(t, window, cx));
            self.due_input.update(cx, |s, cx| {
                s.set_value(
                    detail
                        .soft_deadline
                        .or(detail.hard_deadline)
                        .map(|d| d.with_timezone(&Local).format("%Y-%m-%d").to_string())
                        .unwrap_or_default(),
                    window,
                    cx,
                )
            });
            self.description_input.update(cx, |s, cx| {
                s.set_value(detail.description.clone().unwrap_or_default(), window, cx)
            });
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
            status_row = status_row.child(
                Self::chip(
                    SharedString::from(format!("st-{}", id.simple())),
                    s.name,
                    cur_status == id,
                    cx,
                    move |this, cx| this.set_status(id, cx),
                )
                .disabled(self.updating),
            );
        }

        let mut prio_row = div().flex().flex_row().flex_wrap().gap_1();
        for p in PRIORITIES {
            prio_row = prio_row.child(
                Self::chip(
                    SharedString::from(format!("pr-{p}")),
                    p.to_string(),
                    cur_priority == p,
                    cx,
                    move |this, cx| this.set_priority(p, cx),
                )
                .disabled(self.updating),
            );
        }

        let mut assignee_row = div().flex().flex_row().flex_wrap().gap_1();
        for u in self.assignables.clone() {
            let active = assignee_ids.contains(&u.id);
            assignee_row = assignee_row.child(
                Self::chip(
                    SharedString::from(format!("as-{}", u.id.simple())),
                    u.username.clone(),
                    active,
                    cx,
                    move |this, cx| this.toggle_assignee(&u, cx),
                )
                .disabled(self.updating),
            );
        }

        div()
            .id("task-detail")
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_y_scroll()
            .p_4()
            .child(
                div()
                    .flex()
                    .flex_col()
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
                            .child(
                                div().flex_1().min_w_0().child(
                                    Input::new(&self.title_input)
                                        .id("task-title")
                                        .disabled(self.updating),
                                ),
                            )
                            .child(
                                Button::new("save-title")
                                    .disabled(self.updating)
                                    .ghost()
                                    .icon(IconName::Check)
                                    .tooltip("Save title (Enter)")
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
                            .when(self.statuses.iter().any(|s| s.is_done_state), |d| {
                                d.child(
                                    Button::new("done-toggle")
                                        .disabled(self.updating)
                                        .ghost()
                                        .label(if done { "Done" } else { "Mark done" })
                                        .on_click(
                                            cx.listener(|this, _, _, cx| this.toggle_done(cx)),
                                        ),
                                )
                            }),
                    )
                    .child(section(c.muted_foreground, "Status", status_row))
                    .child(section(c.muted_foreground, "Priority", prio_row))
                    .child(section(c.muted_foreground, "Assignees", assignee_row))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .flex_wrap()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(c.muted_foreground)
                                    .w(px(70.))
                                    .child("Due"),
                            )
                            .child(
                                div().flex_1().min_w(px(120.)).child(
                                    Input::new(&self.due_input)
                                        .id("task-due")
                                        .disabled(self.updating),
                                ),
                            )
                            .child(
                                Button::new("apply-due")
                                    .disabled(self.updating)
                                    .ghost()
                                    .label("Apply")
                                    .on_click(cx.listener(|this, _, _, cx| this.apply_due(cx))),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(c.muted_foreground)
                                            .child("Description"),
                                    )
                                    .child(
                                        Button::new("edit-description")
                                            .compact()
                                            .ghost()
                                            .label(if self.editing_description {
                                                "Cancel"
                                            } else {
                                                "Edit"
                                            })
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                if this.editing_description {
                                                    let description = this
                                                        .detail
                                                        .as_ref()
                                                        .and_then(|d| d.description.clone())
                                                        .unwrap_or_default();
                                                    this.description_input.update(
                                                        cx,
                                                        |input, cx| {
                                                            input.set_value(description, window, cx)
                                                        },
                                                    );
                                                }
                                                this.editing_description =
                                                    !this.editing_description;
                                                cx.notify();
                                            })),
                                    ),
                            )
                            .when(self.editing_description, |d| {
                                d.child(Textarea::new(&self.description_input).h(px(140.)))
                                    .child(
                                        Button::new("save-description")
                                            .disabled(self.updating)
                                            .label("Save description")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.save_description(cx)
                                            })),
                                    )
                            })
                            .when(!self.editing_description, |d| {
                                d.child(
                                    TextView::markdown(
                                        "task-description",
                                        detail
                                            .description
                                            .clone()
                                            .filter(|s| !s.is_empty())
                                            .unwrap_or_else(|| "No description".into()),
                                    )
                                    .w_full(),
                                )
                            }),
                    )
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
                            .children(self.comments.iter().enumerate().map(|(ix, cm)| {
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
                                                div()
                                                    .text_xs()
                                                    .text_color(c.muted_foreground)
                                                    .child(
                                                        cm.created_at
                                                            .format("%Y-%m-%d %H:%M")
                                                            .to_string(),
                                                    ),
                                            ),
                                    )
                                    .child(
                                        TextView::markdown(
                                            ("task-comment", ix),
                                            cm.body.clone().unwrap_or_default(),
                                        )
                                        .w_full(),
                                    )
                                    .children(cm.replies.iter().map(|reply| {
                                        div()
                                            .ml_4()
                                            .pl_3()
                                            .py_2()
                                            .border_l_1()
                                            .border_color(c.border)
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(c.muted_foreground)
                                                    .child(format!(
                                                        "{} · {}",
                                                        reply.user.name,
                                                        reply.created_at.format("%Y-%m-%d %H:%M")
                                                    )),
                                            )
                                            .child(
                                                TextView::markdown(
                                                    SharedString::from(format!(
                                                        "comment-reply-{}",
                                                        reply.id
                                                    )),
                                                    reply.body.clone().unwrap_or_else(|| {
                                                        "Deleted comment".into()
                                                    }),
                                                )
                                                .w_full(),
                                            )
                                    }))
                            }))
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div().flex_1().min_w_0().child(
                                            Input::new(&self.comment_input)
                                                .id("task-comment-input")
                                                .disabled(self.posting_comment),
                                        ),
                                    )
                                    .child(
                                        Button::new("post-comment")
                                            .ghost()
                                            .label(if self.posting_comment {
                                                "Posting…"
                                            } else {
                                                "Post"
                                            })
                                            .disabled(self.posting_comment)
                                            .on_click(
                                                cx.listener(|this, _, _, cx| this.post_comment(cx)),
                                            ),
                                    ),
                            ),
                    ),
            )
    }
}

impl EventEmitter<TaskDetailEvent> for TaskDetailView {}

fn section(muted: Hsla, label: &'static str, row: Div) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(div().text_xs().text_color(muted).child(label))
        .child(row)
}
