//! §15 一覧。My Tasks / Today / Upcoming / Project の 4 モードを 1 View で持つ。

use std::collections::{HashMap, HashSet};

use api::types::{ProjectStatusResponse, UpdateTaskRequest};
use api::{Client, MyTasksQuery, TasksQuery};
use chrono::Local;
use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::component::IndexPath;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::list::{List, ListDelegate, ListEvent, ListItem, ListState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{Disableable, Theme, WindowExt};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use uuid::Uuid;

use crate::model::{TaskRow, due_label, parse_hex_color};

const PAGE_SIZE: u32 = 50;

struct TaskRows {
    rows: Vec<TaskRow>,
    completable: HashSet<Uuid>,
    pending: HashSet<Uuid>,
    owner: WeakEntity<TaskListView>,
    loading: bool,
    signed_in: bool,
    more: bool,
}

impl ListDelegate for TaskRows {
    type Item = ListItem;
    fn items_count(&self, _: usize, _: &App) -> usize {
        self.rows.len()
    }
    fn set_selected_index(
        &mut self,
        _: Option<IndexPath>,
        _: &mut Window,
        _: &mut Context<ListState<Self>>,
    ) {
    }
    fn loading(&self, _: &App) -> bool {
        self.loading && self.rows.is_empty()
    }
    fn has_more(&self, _: &App) -> bool {
        self.more && !self.loading
    }
    fn load_more(&mut self, _: &mut Window, cx: &mut Context<ListState<Self>>) {
        let owner = self.owner.clone();
        cx.defer(move |cx| {
            let _ = owner.update(cx, |view, cx| view.load_more(cx));
        });
    }
    fn render_empty(
        &mut self,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> impl IntoElement {
        div()
            .p_4()
            .text_sm()
            .text_color(Theme::global(cx).semantic_tokens().colors.muted_foreground)
            .child(if self.signed_in {
                "No tasks"
            } else {
                "Sign in to see tasks"
            })
    }
    fn render_item(
        &mut self,
        ix: IndexPath,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<ListItem> {
        let row = self.rows.get(ix.row)?;
        let c = Theme::global(cx).semantic_tokens().colors;
        let id = row.id;
        let owner = self.owner.clone();
        Some(
            ListItem::new(("task-row", ix.row))
                .h(px(76.))
                .w_full()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .when(self.completable.contains(&row.project_id), |d| {
                                    d.child(
                                        Button::new(("done", ix.row))
                                            .compact()
                                            .ghost()
                                            .disabled(self.pending.contains(&id))
                                            .label(if row.is_done { "✓" } else { "○" })
                                            .tooltip(if row.is_done {
                                                "Reopen task"
                                            } else {
                                                "Mark done"
                                            })
                                            .on_click(move |_, _, cx| {
                                                let _ = owner.update(cx, |view, cx| {
                                                    if let Some(ix) = view
                                                        .rows
                                                        .iter()
                                                        .position(|row| row.id == id)
                                                    {
                                                        view.toggle_done(ix, cx);
                                                    }
                                                });
                                                cx.stop_propagation();
                                            }),
                                    )
                                })
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .text_sm()
                                        .text_ellipsis()
                                        .when(row.is_done, |d| {
                                            d.text_color(c.muted_foreground).line_through()
                                        })
                                        .child(row.title.clone()),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .min_w_0()
                                .text_xs()
                                .text_color(c.muted_foreground)
                                .child(row.seq_key.clone())
                                .child(
                                    div()
                                        .text_color(
                                            parse_hex_color(&row.status_color)
                                                .unwrap_or(c.muted_foreground),
                                        )
                                        .child(row.status_name.clone()),
                                )
                                .child(div().flex_1().min_w_0().text_ellipsis().child(format!(
                                    "{}{}",
                                    row.priority,
                                    row.due
                                        .as_ref()
                                        .map(|due| format!(" · {}", due_label(due)))
                                        .unwrap_or_default()
                                ))),
                        ),
                ),
        )
    }
}

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
    shown_error: Option<String>,
    create_input: Entity<InputState>,
    /// 作成成功後に render 側で input をクリアするフラグ
    /// （spawn タスクからは Window に触れないため）。
    clear_create_input: bool,
    creating: bool,
    list_state: Entity<ListState<TaskRows>>,
    selected: Option<usize>,
    generation: u64,
    context_generation: u64,
    pending_done: HashSet<Uuid>,
    _subs: Vec<Subscription>,
}

impl TaskListView {
    pub fn new(
        client: Option<Client>,
        tenant: Option<Uuid>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let create_input = cx.new(|cx| InputState::new(window, cx).placeholder("New task title…"));
        let sub = cx.subscribe(&create_input, |this, _, ev: &InputEvent, cx| {
            if matches!(ev, InputEvent::PressEnter { .. }) {
                this.create_task(cx);
            }
        });
        let owner = cx.weak_entity();
        let list_state = cx.new(|cx| {
            ListState::new(
                TaskRows {
                    rows: vec![],
                    completable: HashSet::new(),
                    pending: HashSet::new(),
                    owner,
                    loading: false,
                    signed_in: false,
                    more: false,
                },
                window,
                cx,
            )
        });
        let _ = list_state.read(cx).focus_handle(cx).tab_stop(true);
        let list_sub = cx.subscribe(&list_state, |this, _, event: &ListEvent, cx| {
            if let ListEvent::Select(ix) | ListEvent::Confirm(ix) = event {
                this.selected = Some(ix.row);
                if let Some(row) = this.rows.get(ix.row) {
                    cx.emit(TaskListEvent::Select {
                        project: row.project_id,
                        task: row.id,
                    });
                }
                cx.notify();
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
            shown_error: None,
            create_input,
            clear_create_input: false,
            creating: false,
            list_state,
            selected: None,
            generation: 0,
            context_generation: 0,
            pending_done: HashSet::new(),
            _subs: vec![sub, list_sub],
        }
    }

    pub fn set_client(&mut self, client: Client, tenant: Option<Uuid>, cx: &mut Context<Self>) {
        if self.tenant != tenant {
            self.context_generation += 1;
            self.creating = false;
            self.clear_create_input = true;
            self.rows.clear();
            self.statuses.clear();
            self.selected = None;
            self.mode = ListMode::MyTasks;
        }
        self.client = Some(client);
        self.tenant = tenant;
        self.reload(cx);
    }

    /// ログアウト時に呼ぶ。
    pub fn clear_client(&mut self, cx: &mut Context<Self>) {
        self.context_generation += 1;
        self.creating = false;
        self.clear_create_input = true;
        self.client = None;
        self.rows.clear();
        self.statuses.clear();
        self.generation += 1;
        self.loading = false;
        cx.notify();
    }

    pub fn focus_create(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.create_input
            .update(cx, |input, cx| input.focus(window, cx));
    }

    pub fn set_mode(&mut self, mode: ListMode, cx: &mut Context<Self>) {
        if self.mode != mode {
            self.context_generation += 1;
            self.creating = false;
            self.clear_create_input = true;
        }
        self.mode = mode;
        self.rows.clear();
        self.selected = None;
        self.reload(cx);
    }

    /// Quick Search (§20) 用に現在ロード済みの行を返す。
    pub fn rows_snapshot(&self) -> Vec<TaskRow> {
        self.rows.clone()
    }

    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let (Some(client), Some(tenant)) = (self.client.clone(), self.tenant) else {
            return;
        };
        self.loading = true;
        self.error = None;
        self.next_cursor = None;
        self.generation += 1;
        let generation = self.generation;
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
                    async {
                        let mut all = Vec::new();
                        let mut offset = 0;
                        loop {
                            let q = MyTasksQuery {
                                include_personal: Some(true),
                                limit: Some(200),
                                offset: Some(offset),
                                ..Default::default()
                            };
                            let page = client.list_my_tasks(tenant, &q).await?;
                            let count = page.tasks.len();
                            offset += count as u64;
                            all.extend(page.tasks.iter().map(TaskRow::from_my));
                            if count == 0 || offset >= page.total.max(0) as u64 {
                                break;
                            }
                        }
                        let today = Local::now().date_naive();
                        let rows = match mode {
                            // §15: Today = 期限切れ含む今日まで。Upcoming = 明日以降。
                            ListMode::Today => all
                                .into_iter()
                                .filter(|r| {
                                    r.due
                                        .map(|d| d.with_timezone(&Local).date_naive() <= today)
                                        .unwrap_or(false)
                                })
                                .collect(),
                            ListMode::Upcoming => all
                                .into_iter()
                                .filter(|r| {
                                    r.due
                                        .map(|d| d.with_timezone(&Local).date_naive() > today)
                                        .unwrap_or(false)
                                })
                                .collect(),
                            _ => all,
                        };
                        Ok::<_, api::ApiError>((rows, None))
                    }
                    .await
                }
            };
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
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
        if self.loading {
            return;
        }
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
        let generation = self.generation;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let res = client.list_tasks(tenant, id, &q).await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
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
                        if this.tenant != Some(tenant) || this.client.is_none() {
                            return;
                        }
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
            if row.status_name.is_empty()
                && let Some(s) = self
                    .statuses
                    .get(&row.project_id)
                    .and_then(|ss| ss.iter().find(|s| s.id == row.status_id))
            {
                row.status_name = s.name.clone();
                row.status_color = s.color.clone();
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
        if self.pending_done.contains(&row.id) {
            return;
        }
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
        self.pending_done.insert(row.id);
        let context_generation = self.context_generation;
        let generation = self.generation;
        cx.notify();
        let req = UpdateTaskRequest {
            status_id: Some(target_id),
            ..Default::default()
        };
        cx.spawn(async move |this, cx| {
            let res = client
                .update_task(tenant, row.project_id, row.id, &req)
                .await;
            let _ = this.update(cx, |this, cx| {
                this.pending_done.remove(&row.id);
                if this.context_generation != context_generation
                    || this.tenant != Some(tenant)
                    || this.client.is_none()
                {
                    return;
                }
                if let Err(e) = res {
                    // Locate by identity: the list may have been reloaded or reordered.
                    if this.generation != generation {
                        this.reload(cx);
                    } else if let Some(r) = this.rows.iter_mut().find(|r| r.id == row.id) {
                        r.is_done = prev_done;
                        r.status_id = prev_id;
                        r.status_name = prev_name;
                        r.status_color = prev_color;
                    }
                    this.error = Some(e.to_string());
                } else if this
                    .selected
                    .and_then(|ix| this.rows.get(ix))
                    .is_some_and(|selected| selected.id == row.id)
                {
                    cx.emit(TaskListEvent::Select {
                        project: row.project_id,
                        task: row.id,
                    });
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn create_task(&mut self, cx: &mut Context<Self>) {
        if self.creating {
            return;
        }
        let title = self.create_input.read(cx).value().trim().to_string();
        if title.is_empty() {
            return;
        }
        let (Some(client), Some(tenant)) = (self.client.clone(), self.tenant) else {
            return;
        };
        // My Tasks 系では personal project へ、Project ではその project へ。
        let mode = self.mode.clone();
        let context_generation = self.context_generation;
        self.creating = true;
        self.error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let project = match &mode {
                ListMode::Project { id, .. } => Some(*id),
                _ => client.get_personal_project(tenant).await.ok().map(|p| p.id),
            };
            let Some(project) = project else {
                let _ = this.update(cx, |this, cx| {
                    if this.context_generation != context_generation || this.client.is_none() {
                        return;
                    }
                    this.creating = false;
                    this.error = Some("Could not load the personal project. Try again.".into());
                    cx.notify();
                });
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
                let _ = this.update(cx, |this, cx| {
                    if this.context_generation != context_generation || this.client.is_none() {
                        return;
                    }
                    this.creating = false;
                    this.error = Some("Could not load a task status. Try again.".into());
                    cx.notify();
                });
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
                if this.context_generation != context_generation
                    || this.tenant != Some(tenant)
                    || this.client.is_none()
                {
                    return;
                }
                this.creating = false;
                match res {
                    Ok(task) => {
                        this.clear_create_input = true;
                        this.reload(cx);
                        cx.emit(TaskListEvent::Select {
                            project,
                            task: task.id,
                        });
                    }
                    Err(e) => this.error = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }
}

impl EventEmitter<TaskListEvent> for TaskListView {}

impl Render for TaskListView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.error != self.shown_error {
            self.shown_error = self.error.clone();
            if let Some(error) = &self.error {
                window.push_notification(Notification::new().message(error.clone()), cx);
            }
        }
        if self.clear_create_input {
            self.clear_create_input = false;
            self.create_input
                .update(cx, |s, cx| s.set_value("", window, cx));
        }
        let (c, danger) = {
            let t = Theme::global(cx);
            (t.semantic_tokens().colors, t.danger)
        };

        self.list_state.update(cx, |state, cx| {
            let delegate = state.delegate_mut();
            delegate.rows = self.rows.clone();
            delegate.completable = self
                .statuses
                .iter()
                .filter(|(_, statuses)| statuses.iter().any(|s| s.is_done_state))
                .map(|(id, _)| *id)
                .collect();
            delegate.pending = self.pending_done.clone();
            delegate.loading = self.loading;
            delegate.signed_in = self.client.is_some();
            delegate.more = self.next_cursor.is_some();
            state.set_selected_index(self.selected.map(IndexPath::new), window, cx);
            cx.notify();
        });
        let list = List::new(&self.list_state);
        div()
            .id("task-list-view")
            .flex()
            .flex_col()
            .size_full()
            .min_w_0()
            .min_h_0()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_shrink_0()
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
                    .child(
                        div().flex_1().min_w_0().child(
                            Input::new(&self.create_input)
                                .id("new-task-title")
                                .disabled(self.creating),
                        ),
                    )
                    .child(
                        Button::new("create-task")
                            .compact()
                            .label(if self.creating { "Creating…" } else { "Add" })
                            .disabled(self.creating)
                            .on_click(cx.listener(|this, _, _, cx| this.create_task(cx))),
                    ),
            )
            .when_some(self.error.clone(), |d, e| {
                d.child(
                    div()
                        .px_4()
                        .py_2()
                        .child(div().text_sm().text_color(danger).child(e)),
                )
            })
            .child(div().flex_1().min_h_0().child(list))
    }
}
