//! §15 一覧。My Tasks / Today / Upcoming / Project の 4 モードを 1 View で持つ。

use std::collections::HashMap;

use api::types::{ProjectStatusResponse, UpdateTaskRequest};
use api::{Client, MyTasksQuery, TasksQuery};
use chrono::Utc;
use gpui_kit::assets::IconName;
use gpui_kit::component::Theme;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::Icon;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use uuid::Uuid;

use crate::model::{TaskRow, due_label, parse_hex_color};

const PAGE_SIZE: u32 = 50;

#[derive(Debug, Clone, PartialEq)]
pub enum ListMode {
    MyTasks,
    Today,
    Upcoming,
    Project { id: Uuid, key: String },
}

#[derive(Debug, Clone)]
pub enum TaskListEvent {
    /// 行クリック → Detail ペインへ開く。
    Select { project: Uuid, task: Uuid },
}

pub struct TaskListView {
    client: Option<Client>,
    tenant: Option<Uuid>,
    mode: ListMode,
    rows: Vec<TaskRow>,
    /// project_id → そのプロジェクトのステータス一覧（done 判定用）。
    statuses: HashMap<Uuid, Vec<ProjectStatusResponse>>,
    next_cursor: Option<String>,
    loading: bool,
    error: Option<String>,
    create_input: Entity<InputState>,
    /// 作成成功後に render 側で input をクリアするフラグ
    /// （spawn タスクからは Window に触れないため）。
    clear_create_input: bool,
    _subs: Vec<Subscription>,
}

impl TaskListView {
    pub fn new(
        client: Option<Client>,
        tenant: Option<Uuid>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let create_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("New task title…")
        });
        let sub = cx.subscribe(&create_input, |this, _, ev: &InputEvent, cx| {
            if matches!(ev, InputEvent::PressEnter { .. }) {
                this.create_task(cx);
            }
        });
        Self {
            client,
            tenant,
            mode: ListMode::MyTasks,
            rows: vec![],
            statuses: HashMap::new(),
            next_cursor: None,
            loading: false,
            error: None,
            create_input,
            clear_create_input: false,
            _subs: vec![sub],
        }
    }

    pub fn set_client(&mut self, client: Client, tenant: Option<Uuid>, cx: &mut Context<Self>) {
        self.client = Some(client);
        self.tenant = tenant;
        self.reload(cx);
    }

    pub fn set_mode(&mut self, mode: ListMode, cx: &mut Context<Self>) {
        self.mode = mode;
        self.reload(cx);
    }

    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let (Some(client), Some(tenant)) = (self.client.clone(), self.tenant) else {
            return;
        };
        self.loading = true;
        self.error = None;
        self.next_cursor = None;
        cx.notify();
        let mode = self.mode.clone();
        cx.spawn(async move |this, cx| {
            let res = match &mode {
                ListMode::Project { id, key } => {
                    let q = TasksQuery {
                        limit: Some(PAGE_SIZE),
                        ..Default::default()
                    };
                    client.list_tasks(tenant, *id, &q).await.map(|page| {
                        (
                            page.tasks
                                .iter()
                                .map(|t| TaskRow::from_task(t, key))
                                .collect(),
                            page.next_cursor,
                        )
                    })
                }
                _ => {
                    let q = MyTasksQuery {
                        include_personal: Some(true),
                        limit: Some(200),
                        ..Default::default()
                    };
                    client.list_my_tasks(tenant, &q).await.map(|page| {
                        let all: Vec<TaskRow> =
                            page.tasks.iter().map(TaskRow::from_my).collect();
                        let today = Utc::now().date_naive();
                        let rows = match mode {
                            // §15: Today = 期限切れ含む今日まで。Upcoming = 明日以降。
                            ListMode::Today => all
                                .into_iter()
                                .filter(|r| {
                                    r.due.map(|d| d.date_naive() <= today).unwrap_or(false)
                                })
                                .collect(),
                            ListMode::Upcoming => all
                                .into_iter()
                                .filter(|r| {
                                    r.due.map(|d| d.date_naive() > today).unwrap_or(false)
                                })
                                .collect(),
                            _ => all,
                        };
                        (rows, None)
                    })
                }
            };
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                match res {
                    Ok((rows, cursor)) => {
                        this.rows = rows;
                        this.next_cursor = cursor;
                        this.resolve_done_states(cx);
                    }
                    Err(e) => this.error = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn load_more(&mut self, cx: &mut Context<Self>) {
        let (Some(client), Some(tenant), Some(cursor)) =
            (self.client.clone(), self.tenant, self.next_cursor.clone())
        else {
            return;
        };
        let ListMode::Project { id, key } = &self.mode else {
            return;
        };
        let (id, key) = (*id, key.clone());
        let q = TasksQuery {
            limit: Some(PAGE_SIZE),
            cursor: Some(cursor),
            ..Default::default()
        };
        self.loading = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let res = client.list_tasks(tenant, id, &q).await;
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                match res {
                    Ok(page) => {
                        for t in &page.tasks {
                            let row = TaskRow::from_task(t, &key);
                            if !this.rows.iter().any(|r| r.id == row.id) {
                                this.rows.push(row);
                            }
                        }
                        this.next_cursor = page.next_cursor;
                        this.resolve_done_states(cx);
                    }
                    Err(e) => this.error = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 各行の status_id が done ステータスかを解決する。
    /// 未取得のプロジェクトは statuses を取ってから再評価。
    fn resolve_done_states(&mut self, cx: &mut Context<Self>) {
        let missing: Vec<Uuid> = self
            .rows
            .iter()
            .map(|r| r.project_id)
            .filter(|p| !self.statuses.contains_key(p))
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        self.apply_statuses();
        if missing.is_empty() {
            return;
        }
        let (Some(client), Some(tenant)) = (self.client.clone(), self.tenant) else {
            return;
        };
        cx.spawn(async move |this, cx| {
            for project in missing {
                if let Ok(list) = client.list_statuses(tenant, project).await {
                    let _ = this.update(cx, |this, cx| {
                        this.statuses.insert(project, list);
                        this.apply_statuses();
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    fn apply_statuses(&mut self) {
        for row in &mut self.rows {
            row.is_done = self
                .statuses
                .get(&row.project_id)
                .and_then(|ss| ss.iter().find(|s| s.id == row.status_id))
                .map(|s| s.is_done_state)
                .unwrap_or(false);
            if row.status_name.is_empty() {
                if let Some(s) = self
                    .statuses
                    .get(&row.project_id)
                    .and_then(|ss| ss.iter().find(|s| s.id == row.status_id))
                {
                    row.status_name = s.name.clone();
                    row.status_color = s.color.clone();
                }
            }
        }
    }

    /// §15 done スイッチ + Optimistic Update（失敗で rollback + エラー表示 §23）。
    fn toggle_done(&mut self, ix: usize, cx: &mut Context<Self>) {
        let (Some(client), Some(tenant)) = (self.client.clone(), self.tenant) else {
            return;
        };
        let Some(row) = self.rows.get(ix).cloned() else {
            return;
        };
        let Some(statuses) = self.statuses.get(&row.project_id) else {
            return;
        };
        // done ↔ 最初の非 done（is_default 優先）を往復する。
        let target = if row.is_done {
            statuses
                .iter()
                .find(|s| !s.is_done_state && s.is_default)
                .or_else(|| statuses.iter().find(|s| !s.is_done_state))
        } else {
            statuses
                .iter()
                .find(|s| s.is_done_state && s.is_default_done)
                .or_else(|| statuses.iter().find(|s| s.is_done_state))
        };
        let Some(target) = target else {
            return;
        };
        let (target_id, prev_id, prev_done) = (target.id, row.status_id, row.is_done);
        let prev_name = row.status_name.clone();
        let prev_color = row.status_color.clone();
        if let Some(r) = self.rows.get_mut(ix) {
            r.is_done = !prev_done;
            r.status_id = target_id;
            r.status_name = target.name.clone();
            r.status_color = target.color.clone();
        }
        cx.notify();
        let req = UpdateTaskRequest {
            status_id: Some(target_id),
            ..Default::default()
        };
        cx.spawn(async move |this, cx| {
            let res = client
                .update_task(tenant, row.project_id, row.id, &req)
                .await;
            if let Err(e) = res {
                // §23: rollback + エラー表示。
                let _ = this.update(cx, |this, cx| {
                    if let Some(r) = this.rows.get_mut(ix) {
                        r.is_done = prev_done;
                        r.status_id = prev_id;
                        r.status_name = prev_name;
                        r.status_color = prev_color;
                    }
                    this.error = Some(e.to_string());
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn create_task(&mut self, cx: &mut Context<Self>) {
        let title = self.create_input.read(cx).value().trim().to_string();
        if title.is_empty() {
            return;
        }
        let (Some(client), Some(tenant)) = (self.client.clone(), self.tenant) else {
            return;
        };
        // My Tasks 系では personal project へ、Project ではその project へ。
        let mode = self.mode.clone();
        cx.spawn(async move |this, cx| {
            let project = match &mode {
                ListMode::Project { id, .. } => Some(*id),
                _ => client.get_personal_project(tenant).await.ok().map(|p| p.id),
            };
            let Some(project) = project else {
                return;
            };
            let status = client
                .list_statuses(tenant, project)
                .await
                .ok()
                .and_then(|ss| {
                    ss.iter()
                        .find(|s| s.is_default)
                        .or_else(|| ss.first())
                        .map(|s| s.id)
                });
            let Some(status_id) = status else {
                return;
            };
            let req = api::types::CreateTaskRequest {
                title: title.clone(),
                status_id,
                assignees: vec![],
                custom_field_values: vec![],
                description: None,
                estimated_minutes: None,
                hard_deadline: None,
                label_ids: vec![],
                milestone_id: None,
                parent_task_id: None,
                priority: None,
                progress_pct: None,
                soft_deadline: None,
                sprint_id: None,
            };
            let res = client.create_task(tenant, project, &req).await;
            let _ = this.update(cx, |this, cx| {
                match res {
                    Ok(_) => {
                        this.clear_create_input = true;
                        this.reload(cx);
                    }
                    Err(e) => this.error = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn row(&self, row: &TaskRow, ix: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let c = Theme::global(cx).semantic_tokens().colors.clone();
        let status_color = parse_hex_color(&row.status_color);
        let has_done = self.statuses.contains_key(&row.project_id);
        let due = row.due.as_ref().map(due_label);

        div()
            .id(("task-row", ix))
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(c.border)
            .hover(|s| s.bg(c.muted))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                if let Some(r) = this.rows.get(ix) {
                    cx.emit(TaskListEvent::Select {
                        project: r.project_id,
                        task: r.id,
                    });
                }
            }))
            .child(
                div()
                    .id(("done", ix))
                    .size(px(16.))
                    .rounded_sm()
                    .border_1()
                    .border_color(if row.is_done { c.accent } else { c.border })
                    .when(row.is_done, |d| d.bg(c.accent))
                    .flex_shrink_0()
                    .when(has_done, |d| {
                        d.on_click(cx.listener(move |this, _, _, cx| {
                            this.toggle_done(ix, cx);
                            cx.stop_propagation();
                        }))
                    }),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(c.muted_foreground)
                    .w(px(80.))
                    .flex_shrink_0()
                    .child(row.seq_key.clone()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .when(row.is_done, |d| {
                        d.text_color(c.muted_foreground).line_through()
                    })
                    .child(row.title.clone()),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(status_color.unwrap_or(c.muted_foreground))
                    .child(row.status_name.clone()),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(c.muted_foreground)
                    .child(format!("{}", row.priority)),
            )
            .when_some(due, |d, due| {
                d.child(
                    div()
                        .text_xs()
                        .text_color(c.muted_foreground)
                        .w(px(90.))
                        .child(due),
                )
            })
    }
}

impl EventEmitter<TaskListEvent> for TaskListView {}

impl Render for TaskListView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.clear_create_input {
            self.clear_create_input = false;
            self.create_input
                .update(cx, |s, cx| s.set_value("", window, cx));
        }
        let (c, danger) = {
            let t = Theme::global(cx);
            (t.semantic_tokens().colors.clone(), t.danger)
        };

        let mut list = div()
            .id("task-list")
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .overflow_y_scroll();
        for (ix, row) in self.rows.iter().enumerate() {
            list = list.child(self.row(row, ix, cx));
        }
        if self.rows.is_empty() && !self.loading {
            list = list.child(
                div().p_8().child(
                    div()
                        .text_sm()
                        .text_color(c.muted_foreground)
                        .child(if self.client.is_some() {
                            "No tasks"
                        } else {
                            "Sign in to see tasks"
                        }),
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
        if self.next_cursor.is_some() && !self.loading {
            list = list.child(
                div().p_2().child(
                    Button::new("tl-more")
                        .ghost()
                        .label("Load more")
                        .on_click(cx.listener(|this, _, _, cx| this.load_more(cx))),
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
                        Icon::new(IconName::Plus)
                            .size_4()
                            .text_color(c.muted_foreground),
                    )
                    .child(div().flex_1().child(Input::new(&self.create_input))),
            )
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
