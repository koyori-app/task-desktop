//! §15 Detail ペイン。Status / Priority / Assignee / DueDate / done の
//! 編集とコメント表示・投稿。更新は Optimistic + rollback（§23）。

use std::collections::HashMap;

use crate::avatar::user_avatar;
use crate::model::{PRIORITIES, due_timestamp, parse_hex_color, priority_label};
use crate::ui::status_pill;
use api::Client;
use api::types::{
    AssigneeInput, CommentThread, CreateCommentRequest, ProjectStatusResponse, TaskAssigneeSummary,
    TaskDetailResponse, TaskPriority, UpdateTaskRequest, UserSummary,
};
use chrono::{Days, Local, NaiveDate};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::calendar::Date;
use gpui_kit::component::date_picker::{
    DatePicker, DatePickerEvent, DatePickerState, DateRangePreset,
};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::text::TextView;
use gpui_kit::component::{Disableable, Icon, Size, Theme, WindowExt};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use i18n::t;
use uuid::Uuid;

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
    due_picker: Entity<DatePickerState>,
    comment_input: Entity<InputState>,
    description_input: Entity<TextareaState>,
    editing_description: bool,
    posting_comment: bool,
    clear_comment_input: bool,
    shown_error: Option<String>,
    _subs: Vec<Subscription>,
    /// title input をどの task まで同期したか。
    title_synced: Option<Uuid>,
    /// project_id → key。Detail の見出しを一覧と同じ `KEY-12` 表記にする。
    project_keys: HashMap<Uuid, String>,
}

impl TaskDetailView {
    pub fn new(
        client: Option<Client>,
        tenant: Option<Uuid>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let title_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("tasks.detail.title_placeholder"))
        });
        let due_picker = cx.new(|cx| DatePickerState::new(window, cx).date_format("%Y-%m-%d"));
        let comment_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("tasks.detail.comment_placeholder"))
        });
        let subs = vec![
            cx.subscribe(&title_input, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.save_title(cx);
                }
            }),
            cx.subscribe(&due_picker, |this, _, event: &DatePickerEvent, cx| {
                let DatePickerEvent::Change(Date::Single(date)) = event else {
                    return;
                };
                this.apply_due(*date, cx);
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
            due_picker,
            comment_input,
            description_input: cx.new(|cx| {
                TextareaState::new(window, cx)
                    .placeholder(t!("tasks.detail.description_placeholder"))
            }),
            editing_description: false,
            posting_comment: false,
            clear_comment_input: false,
            shown_error: None,
            _subs: subs,
            title_synced: None,
            project_keys: HashMap::new(),
        }
    }

    pub fn set_project_keys(&mut self, keys: impl IntoIterator<Item = (Uuid, String)>) {
        self.project_keys = keys.into_iter().collect();
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
                        this.error = Some(t!("tasks.detail.not_found").into())
                    }
                    Err(e) => this.error = Some(e.to_string()),
                }
                match statuses {
                    Ok(s) => this.statuses = s,
                    Err(e) => {
                        if this.detail.is_some() {
                            this.error = Some(t!("tasks.detail.statuses_error", error = e));
                        }
                    }
                }
                match assignables {
                    Ok(users) => this.assignables = users,
                    Err(e) => {
                        if this.detail.is_some() {
                            this.error = Some(t!("tasks.detail.assignees_error", error = e));
                        }
                    }
                }
                match comments {
                    Ok(comments) => this.comments = comments.comments,
                    Err(e) => {
                        if this.detail.is_some() {
                            this.error = Some(t!("tasks.detail.comments_error", error = e));
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

    /// `None` は期限を外す（soft / hard の両方）。
    fn apply_due(&mut self, date: Option<NaiveDate>, cx: &mut Context<Self>) {
        let req = match date.and_then(|d| due_timestamp(d, &Local)) {
            Some(dt) => UpdateTaskRequest {
                soft_deadline: Some(dt),
                ..Default::default()
            },
            None => UpdateTaskRequest {
                clear_soft_deadline: Some(true),
                clear_hard_deadline: Some(true),
                ..Default::default()
            },
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
            return div()
                .id("task-detail-empty")
                .size_full()
                .p_6()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_2()
                .text_color(c.muted_foreground)
                .when(!self.loading && self.error.is_none(), |d| {
                    d.child(Icon::new(IconName::ClipboardList).size_8())
                })
                .child(div().text_sm().child(if self.loading {
                    t!("tasks.detail.loading").to_string()
                } else {
                    self.error
                        .clone()
                        .unwrap_or_else(|| t!("tasks.detail.select_task").into())
                }));
        };

        if self.clear_comment_input {
            self.clear_comment_input = false;
            self.comment_input
                .update(cx, |s, cx| s.set_value("", window, cx));
        }
        // 期限は保存値が変わるたび（失敗時の巻き戻しを含む）に合わせる。
        let due = detail
            .soft_deadline
            .or(detail.hard_deadline)
            .map(|d| d.with_timezone(&Local).date_naive());
        if self.due_picker.read(cx).date() != Date::Single(due) {
            self.due_picker
                .update(cx, |s, cx| s.set_date(Date::Single(due), window, cx));
        }
        // title input は task が変わった時だけ detail のタイトルに同期。
        if self.title_synced != Some(detail.id) {
            self.title_synced = Some(detail.id);
            let t = detail.title.clone();
            self.title_input
                .update(cx, |s, cx| s.set_value(t, window, cx));
            self.description_input.update(cx, |s, cx| {
                s.set_value(detail.description.clone().unwrap_or_default(), window, cx)
            });
        }

        let muted = c.muted_foreground;
        let current_status = self
            .statuses
            .iter()
            .find(|s| s.id == detail.status_id)
            .cloned();
        let has_done_state = self.statuses.iter().any(|s| s.is_done_state);
        let done = self.is_done();
        let cur_status = detail.status_id;
        let cur_priority = detail.priority;
        let assignee_ids: Vec<Uuid> = detail.assignees.iter().map(|a| a.user.id).collect();
        let assignee_label = if detail.assignees.is_empty() {
            t!("tasks.detail.unassigned").to_string()
        } else {
            detail
                .assignees
                .iter()
                .map(|a| a.user.username.clone())
                .collect::<Vec<_>>()
                .join(", ")
        };
        let assignee_avatars: Vec<_> = detail
            .assignees
            .iter()
            .map(|a| {
                user_avatar(
                    &a.user.username,
                    a.user.avatar_url.as_deref(),
                    Size::Small,
                    cx,
                )
            })
            .collect();
        // 描画中に cx を借りられないので、コメント欄のアイコンは先に作っておく。
        let mut comment_avatars = self
            .comments
            .iter()
            .map(|cm| {
                let author = user_avatar(
                    &cm.user.name,
                    cm.user.avatar_url.as_deref(),
                    Size::Small,
                    cx,
                );
                let replies: Vec<_> = cm
                    .replies
                    .iter()
                    .map(|reply| {
                        user_avatar(
                            &reply.user.name,
                            reply.user.avatar_url.as_deref(),
                            Size::XSmall,
                            cx,
                        )
                    })
                    .collect();
                (author, replies)
            })
            .collect::<Vec<_>>()
            .into_iter();
        let task_key = match self.project.and_then(|p| self.project_keys.get(&p)) {
            Some(key) => format!("{key}-{}", detail.seq_id),
            None => format!("#{}", detail.seq_id),
        };
        let this = cx.entity().downgrade();

        let status_statuses = self.statuses.clone();
        let status_owner = this.clone();
        let status_button = Button::new("status-select")
            .outline()
            .compact()
            .dropdown_caret(true)
            .disabled(self.updating || self.statuses.is_empty())
            .child(match &current_status {
                Some(s) => status_pill(&s.name, parse_hex_color(&s.color).unwrap_or(muted)),
                None => div().child("—"),
            })
            .dropdown_menu(move |mut menu, _, _| {
                for s in &status_statuses {
                    let (id, owner) = (s.id, status_owner.clone());
                    menu = menu.item(
                        PopupMenuItem::new(s.name.clone())
                            .checked(cur_status == id)
                            .on_click(move |_, _, cx| {
                                let _ = owner.update(cx, |this, cx| this.set_status(id, cx));
                            }),
                    );
                }
                menu
            });

        // ワークフロー順で 1 つ先のステータスへ進める（Web の「次へ」と同じ）。最後なら出さない。
        let next_status = next_status(&self.statuses, cur_status);
        let status_control = div()
            .flex()
            .items_center()
            .gap_1()
            .child(status_button)
            .when_some(next_status, |d, next| {
                d.child(
                    Button::new("status-next")
                        .ghost()
                        .compact()
                        .icon(IconName::ChevronRight)
                        .disabled(self.updating)
                        .tooltip(t!("tasks.detail.next_status", name = next.name))
                        .on_click(cx.listener(move |this, _, _, cx| this.set_status(next.id, cx))),
                )
            });

        let priority_owner = this.clone();
        let priority_button = Button::new("priority-select")
            .outline()
            .compact()
            .dropdown_caret(true)
            .disabled(self.updating)
            .label(priority_label(cur_priority))
            .dropdown_menu(move |mut menu, _, _| {
                for p in PRIORITIES {
                    let owner = priority_owner.clone();
                    menu = menu.item(
                        PopupMenuItem::new(priority_label(p))
                            .checked(cur_priority == p)
                            .on_click(move |_, _, cx| {
                                let _ = owner.update(cx, |this, cx| this.set_priority(p, cx));
                            }),
                    );
                }
                menu
            });

        let assignables = self.assignables.clone();
        let assignee_owner = this.clone();
        let assignee_button = Button::new("assignee-select")
            .outline()
            .compact()
            .dropdown_caret(true)
            .disabled(self.updating || self.assignables.is_empty())
            .when(assignee_avatars.is_empty(), |b| b.icon(IconName::User))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .when(!assignee_avatars.is_empty(), |d| {
                        // 複数人は少し重ねて並べる。
                        d.child(
                            div().flex().items_center().children(
                                assignee_avatars
                                    .into_iter()
                                    .enumerate()
                                    .map(|(ix, avatar)| {
                                        div().when(ix > 0, |d| d.ml(px(-6.))).child(avatar)
                                    }),
                            ),
                        )
                    })
                    .child(assignee_label),
            )
            .dropdown_menu(move |mut menu, _, _| {
                for u in &assignables {
                    let (user, owner) = (u.clone(), assignee_owner.clone());
                    let (name, url) = (u.username.clone(), u.avatar_url.clone());
                    menu = menu.item(
                        PopupMenuItem::element(move |_, cx| {
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(user_avatar(&name, url.as_deref(), Size::Small, cx))
                                .child(name.clone())
                        })
                        .checked(assignee_ids.contains(&u.id))
                        .on_click(move |_, _, cx| {
                            let _ = owner.update(cx, |this, cx| this.toggle_assignee(&user, cx));
                        }),
                    );
                }
                menu
            });

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
                    .gap_4()
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(muted)
                                    .child(task_key),
                            )
                            .when(has_done_state, |d| {
                                d.child(
                                    Button::new("done-toggle")
                                        .compact()
                                        .disabled(self.updating)
                                        .when(done, |b| {
                                            b.outline().label(t!("tasks.detail.reopen"))
                                        })
                                        .when(!done, |b| {
                                            b.primary().label(t!("tasks.detail.mark_done"))
                                        })
                                        .icon(IconName::Check)
                                        .on_click(
                                            cx.listener(|this, _, _, cx| this.toggle_done(cx)),
                                        ),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
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
                                    .tooltip(t!("tasks.detail.save_title"))
                                    .on_click(cx.listener(|this, _, _, cx| this.save_title(cx))),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(property(muted, t!("tasks.detail.status"), status_control))
                            .child(property(
                                muted,
                                t!("tasks.detail.priority"),
                                priority_button,
                            ))
                            .child(property(
                                muted,
                                t!("tasks.detail.assignees"),
                                assignee_button,
                            ))
                            .child(property(
                                muted,
                                t!("tasks.detail.due"),
                                // 選ぶとすぐ保存する。× で期限を外す。
                                div().w(px(180.)).child(
                                    DatePicker::new(&self.due_picker)
                                        .placeholder(t!("tasks.detail.due_placeholder"))
                                        .cleanable(true)
                                        .presets(due_presets())
                                        .disabled(self.updating),
                                ),
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .pt_3()
                            .border_t_1()
                            .border_color(c.border)
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(t!("tasks.detail.description")),
                                    )
                                    .child(
                                        Button::new("edit-description")
                                            .compact()
                                            .ghost()
                                            .label(if self.editing_description {
                                                t!("tasks.detail.cancel")
                                            } else {
                                                t!("tasks.detail.edit")
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
                                            .label(t!("tasks.detail.save_description"))
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
                                            .unwrap_or_else(|| {
                                                t!("tasks.detail.no_description").into()
                                            }),
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
                            .pt_3()
                            .border_t_1()
                            .border_color(c.border)
                            .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(
                                if self.comments.is_empty() {
                                    t!("tasks.detail.comments").to_string()
                                } else {
                                    t!("tasks.detail.comments_count", count = self.comments.len())
                                },
                            ))
                            .children(self.comments.iter().enumerate().map(|(ix, cm)| {
                                let (author_avatar, reply_avatars) = comment_avatars
                                    .next()
                                    .map(|(author, replies)| (Some(author), replies))
                                    .unwrap_or_default();
                                let mut reply_avatars = reply_avatars.into_iter();
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .py_1()
                                    .child(
                                        div()
                                            .flex()
                                            .flex_row()
                                            .items_center()
                                            .gap_2()
                                            .children(author_avatar)
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
                                                    .flex()
                                                    .items_center()
                                                    .gap_1p5()
                                                    .children(reply_avatars.next())
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
                                                        t!("tasks.detail.deleted_comment").into()
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
                                                t!("tasks.detail.posting")
                                            } else {
                                                t!("tasks.detail.post")
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

/// ラベル列を揃えた 1 行分のプロパティ。
/// 期限のよく使う候補（今日 / 明日 / 1 週間後）。
fn due_presets() -> Vec<DateRangePreset> {
    let today = Local::now().date_naive();
    [
        (t!("tasks.detail.due_today"), 0),
        (t!("tasks.detail.due_tomorrow"), 1),
        (t!("tasks.detail.due_next_week"), 7),
    ]
    .into_iter()
    .filter_map(|(label, days)| {
        let date = today.checked_add_days(Days::new(days))?;
        Some(DateRangePreset::single(label, date))
    })
    .collect()
}

/// `current` の次のステータス（position 順）。最後か見つからなければ None。
fn next_status(statuses: &[ProjectStatusResponse], current: Uuid) -> Option<ProjectStatusResponse> {
    let mut ordered: Vec<&ProjectStatusResponse> = statuses.iter().collect();
    ordered.sort_by_key(|s| s.position);
    let ix = ordered.iter().position(|s| s.id == current)?;
    ordered.get(ix + 1).map(|s| (*s).clone())
}

fn property(muted: Hsla, label: &'static str, value: impl IntoElement) -> Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap_3()
        .min_h(px(32.))
        .child(
            div()
                .w(px(80.))
                .flex_shrink_0()
                .text_sm()
                .text_color(muted)
                .child(label),
        )
        .child(div().min_w_0().child(value))
}

#[cfg(test)]
mod tests {
    // `super::*` だと gpui の `#[test]` マクロが std のものを隠すので個別に import する。
    use super::next_status;
    use api::types::ProjectStatusResponse;
    use uuid::Uuid;

    fn status(position: i32) -> ProjectStatusResponse {
        serde_json::from_value(serde_json::json!({
            "id": Uuid::new_v4(), "project_id": Uuid::nil(), "name": format!("s{position}"),
            "color": "#000000", "position": position, "is_default": false,
            "is_done_state": false, "is_default_done": false,
            "created_at": "2026-01-01T00:00:00Z",
        }))
        .unwrap()
    }

    #[test]
    fn next_status_follows_position_order() {
        // 並びは position 順（配列の順ではない）。
        let (a, b, c) = (status(0), status(1), status(2));
        let statuses = vec![c.clone(), a.clone(), b.clone()];
        assert_eq!(next_status(&statuses, a.id).map(|s| s.id), Some(b.id));
        assert_eq!(next_status(&statuses, b.id).map(|s| s.id), Some(c.id));
        assert!(next_status(&statuses, c.id).is_none());
        assert!(next_status(&statuses, Uuid::nil()).is_none());
    }
}
