//! §15 一覧。My Tasks / Today / Upcoming / Project の 4 モードを 1 View で持つ。

use std::collections::{HashMap, HashSet};

use api::types::{ProjectStatusResponse, TaskPriority, UpdateTaskRequest};
use api::{Client, MyTasksQuery, TasksQuery};
use chrono::Local;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{Disableable, Icon, Sizable, Size, Theme, WindowExt};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use i18n::t;
use uuid::Uuid;

use crate::avatar::user_avatar;
use crate::model::{
    PRIORITIES, TaskRow, due_label, is_overdue, parse_hex_color, priority_color, priority_label,
};

const PAGE_SIZE: u32 = 50;

/// 列幅（Web の TASK_ROW_GRID: 名前 | 担当 7rem | 期限 7rem | 優先度 6rem）。
const ASSIGNEE_W: f32 = 112.;
const DUE_W: f32 = 112.;
const PRIORITY_W: f32 = 104.;
const ROW_H: f32 = 36.;
/// 担当者のアイコンはこの数まで並べ、残りは「+N」。
const MAX_AVATARS: usize = 3;

/// ステータス 1 つ分の塊（Web の TaskGroup）。
struct Group {
    /// 折りたたみ状態の鍵。Project は status id、My Tasks はステータス名。
    key: String,
    name: String,
    color: Option<Hsla>,
    rows: Vec<usize>,
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
    selected: Option<usize>,
    /// 折りたたんだグループの鍵。
    collapsed: HashSet<String>,
    focus_handle: FocusHandle,
    scroll_handle: ScrollHandle,
    /// 直近の描画での (行 index, スクロール領域内の子 index)。キー操作で使う。
    visible_rows: Vec<(usize, usize)>,
    generation: u64,
    context_generation: u64,
    pending: HashSet<Uuid>,
    /// mode 変更後に render 側で作成欄の placeholder を差し替えるフラグ。
    placeholder_dirty: bool,
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
            InputState::new(window, cx).placeholder(t!("tasks.list.new_task_placeholder"))
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
            shown_error: None,
            create_input,
            clear_create_input: false,
            creating: false,
            selected: None,
            collapsed: HashSet::new(),
            focus_handle: cx.focus_handle().tab_stop(true),
            scroll_handle: ScrollHandle::new(),
            visible_rows: vec![],
            generation: 0,
            context_generation: 0,
            pending: HashSet::new(),
            placeholder_dirty: true,
            _subs: vec![sub],
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
            self.placeholder_dirty = true;
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
            self.placeholder_dirty = true;
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
        // Project は空でもステータスのグループを出すので、行が無くても取る。
        let current_project = match &self.mode {
            ListMode::Project { id, .. } => Some(*id),
            _ => None,
        };
        let missing: Vec<Uuid> = self
            .rows
            .iter()
            .map(|r| r.project_id)
            .chain(current_project)
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

    fn set_row_status(&mut self, id: Uuid, status: ProjectStatusResponse, cx: &mut Context<Self>) {
        self.mutate_row(
            id,
            move |row| {
                row.status_id = status.id;
                row.status_name = status.name.clone();
                row.status_color = status.color.clone();
                row.is_done = status.is_done_state;
            },
            UpdateTaskRequest {
                status_id: Some(status.id),
                ..Default::default()
            },
            cx,
        );
    }

    fn set_row_priority(&mut self, id: Uuid, priority: TaskPriority, cx: &mut Context<Self>) {
        self.mutate_row(
            id,
            move |row| row.priority = priority,
            UpdateTaskRequest {
                priority: Some(priority),
                ..Default::default()
            },
            cx,
        );
    }

    /// 行からの変更 + Optimistic Update（失敗で rollback + エラー表示 §23）。
    fn mutate_row(
        &mut self,
        id: Uuid,
        apply: impl FnOnce(&mut TaskRow),
        req: UpdateTaskRequest,
        cx: &mut Context<Self>,
    ) {
        let (Some(client), Some(tenant)) = (self.client.clone(), self.tenant) else {
            return;
        };
        if self.pending.contains(&id) {
            return;
        }
        let Some(row) = self.rows.iter_mut().find(|row| row.id == id) else {
            return;
        };
        let previous = row.clone();
        apply(row);
        self.pending.insert(id);
        let context_generation = self.context_generation;
        let generation = self.generation;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let res = client
                .update_task(tenant, previous.project_id, id, &req)
                .await;
            let _ = this.update(cx, |this, cx| {
                this.pending.remove(&id);
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
                    } else if let Some(row) = this.rows.iter_mut().find(|row| row.id == id) {
                        *row = previous;
                    }
                    this.error = Some(e.to_string());
                } else if this
                    .selected
                    .and_then(|ix| this.rows.get(ix))
                    .is_some_and(|selected| selected.id == id)
                {
                    // Detail を開き直して一覧の変更を反映する。
                    cx.emit(TaskListEvent::Select {
                        project: previous.project_id,
                        task: id,
                    });
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn select_row(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(row) = self.rows.get(ix) else {
            return;
        };
        self.selected = Some(ix);
        cx.emit(TaskListEvent::Select {
            project: row.project_id,
            task: row.id,
        });
        if let Some((_, child)) = self.visible_rows.iter().find(|(row, _)| *row == ix) {
            self.scroll_handle.scroll_to_item(*child);
        }
        cx.notify();
    }

    /// ↑↓ で表示中の行を移動する（折りたたんだグループは飛ばす）。
    fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.visible_rows.is_empty() {
            return;
        }
        let current = self
            .selected
            .and_then(|ix| self.visible_rows.iter().position(|(row, _)| *row == ix));
        let next = match current {
            Some(pos) => (pos as isize + delta).clamp(0, self.visible_rows.len() as isize - 1),
            None if delta > 0 => 0,
            None => self.visible_rows.len() as isize - 1,
        } as usize;
        self.select_row(self.visible_rows[next].0, cx);
    }

    fn toggle_group(&mut self, key: String, cx: &mut Context<Self>) {
        if !self.collapsed.remove(&key) {
            self.collapsed.insert(key);
        }
        cx.notify();
    }

    /// ステータスごとにまとめる。Project はそのプロジェクトの全ステータスを
    /// 並び順どおりに出し（空のグループも出す、Web と同じ）、My Tasks は
    /// プロジェクトをまたぐのでステータス名でまとめる。
    fn groups(&self) -> Vec<Group> {
        if let ListMode::Project { id, .. } = &self.mode
            && let Some(statuses) = self.statuses.get(id)
        {
            let mut statuses = statuses.clone();
            statuses.sort_by_key(|s| s.position);
            let mut groups: Vec<Group> = statuses
                .iter()
                .map(|s| Group {
                    key: s.id.to_string(),
                    name: s.name.clone(),
                    color: parse_hex_color(&s.color),
                    rows: vec![],
                })
                .collect();
            for (ix, row) in self.rows.iter().enumerate() {
                let key = row.status_id.to_string();
                match groups.iter_mut().find(|g| g.key == key) {
                    Some(group) => group.rows.push(ix),
                    None => groups.push(Group {
                        key,
                        name: row.status_name.clone(),
                        color: parse_hex_color(&row.status_color),
                        rows: vec![ix],
                    }),
                }
            }
            return groups;
        }
        let mut groups: Vec<(Group, (bool, i32))> = vec![];
        for (ix, row) in self.rows.iter().enumerate() {
            let key = row.status_name.to_lowercase();
            if let Some((group, _)) = groups.iter_mut().find(|(g, _)| g.key == key) {
                group.rows.push(ix);
                continue;
            }
            // 完了系は後ろ、それ以外はプロジェクトでの並び順。
            let rank = self
                .statuses
                .get(&row.project_id)
                .and_then(|ss| ss.iter().find(|s| s.id == row.status_id))
                .map(|s| (s.is_done_state, s.position))
                .unwrap_or((row.is_done, i32::MAX));
            groups.push((
                Group {
                    key,
                    name: row.status_name.clone(),
                    color: parse_hex_color(&row.status_color),
                    rows: vec![ix],
                },
                rank,
            ));
        }
        groups.sort_by_key(|(_, rank)| *rank);
        groups.into_iter().map(|(group, _)| group).collect()
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
                    this.error = Some(t!("tasks.list.personal_project_error").into());
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
                    this.error = Some(t!("tasks.list.status_error").into());
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
        if self.placeholder_dirty {
            self.placeholder_dirty = false;
            // 作成先が見えないと My Tasks で作ったタスクの行き先が分からない。
            let placeholder = match &self.mode {
                ListMode::Project { key, .. } => t!("tasks.list.add_to_project", key = key),
                _ => t!("tasks.list.add_to_personal").into(),
            };
            self.create_input
                .update(cx, |s, cx| s.set_placeholder(placeholder, window, cx));
        }
        let (c, danger, hover, selected_bg) = {
            let t = Theme::global(cx);
            (
                t.semantic_tokens().colors,
                t.danger,
                t.secondary.opacity(0.5),
                t.secondary,
            )
        };
        let muted = c.muted_foreground;
        let empty_text = match self.mode {
            ListMode::MyTasks => t!("tasks.list.empty_my"),
            ListMode::Today => t!("tasks.list.empty_today"),
            ListMode::Upcoming => t!("tasks.list.empty_upcoming"),
            ListMode::Project { .. } => t!("tasks.list.empty_project"),
        };
        // My Tasks は全て自分の担当なので担当列を出さない。
        let show_assignee = matches!(self.mode, ListMode::Project { .. });
        let owner = cx.entity().downgrade();

        let groups = self.groups();
        let mut body: Vec<AnyElement> = vec![];
        let mut visible_rows = vec![];
        for group in &groups {
            let collapsed = self.collapsed.contains(&group.key);
            let color = group.color.unwrap_or(muted);
            let key = group.key.clone();
            body.push(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .h(px(ROW_H))
                    .px_2()
                    .mt_2()
                    .child(
                        Button::new(SharedString::from(format!("group-{}", group.key)))
                            .ghost()
                            .xsmall()
                            .icon(if collapsed {
                                IconName::ChevronRight
                            } else {
                                IconName::ChevronDown
                            })
                            .tooltip(if collapsed {
                                t!("tasks.list.group_expand", name = group.name)
                            } else {
                                t!("tasks.list.group_collapse", name = group.name)
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.toggle_group(key.clone(), cx)
                            })),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .h(px(20.))
                            .px_2()
                            .rounded_md()
                            .border_1()
                            .border_color(color)
                            .text_size(px(11.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(div().size(px(8.)).rounded_full().bg(color))
                            .child(if group.name.is_empty() {
                                "—".to_string()
                            } else {
                                group.name.clone()
                            }),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(group.rows.len().to_string()),
                    )
                    .into_any_element(),
            );
            if collapsed {
                continue;
            }
            if group.rows.is_empty() {
                body.push(
                    div()
                        .px_3()
                        .py_2()
                        .text_sm()
                        .text_color(muted)
                        .child(t!("tasks.list.group_empty"))
                        .into_any_element(),
                );
                continue;
            }
            for &ix in &group.rows {
                visible_rows.push((ix, body.len()));
                let row = self.rows[ix].clone();
                body.push(
                    self.render_row(
                        ix,
                        row,
                        show_assignee,
                        &owner,
                        muted,
                        danger,
                        hover,
                        selected_bg,
                        c.border,
                        cx,
                    )
                    .into_any_element(),
                );
            }
        }
        if self.next_cursor.is_some() {
            body.push(
                div()
                    .px_2()
                    .py_1()
                    .child(
                        Button::new("load-more")
                            .ghost()
                            .compact()
                            .label(t!("tasks.list.more"))
                            .disabled(self.loading)
                            .on_click(cx.listener(|this, _, _, cx| this.load_more(cx))),
                    )
                    .into_any_element(),
            );
        }
        self.visible_rows = visible_rows;

        let empty = if self.client.is_none() {
            Some(t!("tasks.list.empty_signed_out"))
        } else if self.rows.is_empty() && self.loading {
            Some(t!("tasks.list.loading"))
        } else if groups.is_empty() {
            Some(empty_text)
        } else {
            None
        };

        let header = div()
            .flex()
            .flex_shrink_0()
            .items_center()
            .h(px(32.))
            .px_2()
            .border_b_1()
            .border_color(c.border)
            .text_xs()
            .font_weight(FontWeight::MEDIUM)
            .text_color(muted)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .px_2()
                    .child(t!("tasks.list.column.name")),
            )
            .when(show_assignee, |d| {
                d.child(
                    div()
                        .w(px(ASSIGNEE_W))
                        .px_2()
                        .child(t!("tasks.list.column.assignee")),
                )
            })
            .child(div().w(px(DUE_W)).px_2().child(t!("tasks.list.column.due")))
            .child(
                div()
                    .w(px(PRIORITY_W))
                    .px_2()
                    .child(t!("tasks.list.column.priority")),
            );

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
                    .child(Icon::new(IconName::Plus).size_4().text_color(muted))
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
                            .label(if self.creating {
                                t!("tasks.list.creating")
                            } else {
                                t!("tasks.list.add")
                            })
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
            .map(|d| match empty {
                Some(text) => d.child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap_2()
                        .p_6()
                        .text_color(muted)
                        .child(Icon::new(IconName::Inbox).size_8())
                        .child(div().text_sm().child(text)),
                ),
                None => d.child(header).child(
                    div()
                        .id("task-rows")
                        .track_focus(&self.focus_handle)
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .track_scroll(&self.scroll_handle)
                        .pb_4()
                        .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                            match event.keystroke.key.as_str() {
                                "up" => this.move_selection(-1, cx),
                                "down" => this.move_selection(1, cx),
                                _ => return,
                            }
                            cx.stop_propagation();
                        }))
                        .children(body),
                ),
            })
    }
}

impl TaskListView {
    #[allow(clippy::too_many_arguments)] // 行の描画に使う色をまとめて受け取る
    fn render_row(
        &self,
        ix: usize,
        row: TaskRow,
        show_assignee: bool,
        owner: &WeakEntity<Self>,
        muted: Hsla,
        danger: Hsla,
        hover: Hsla,
        selected_bg: Hsla,
        border: Hsla,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let id = row.id;
        let busy = self.pending.contains(&id);
        let status_color = parse_hex_color(&row.status_color).unwrap_or(muted);
        let statuses = self
            .statuses
            .get(&row.project_id)
            .cloned()
            .unwrap_or_default();

        // 名前の左の丸からステータスを変える（Web と同じ）。
        let status_owner = owner.clone();
        let current_status = row.status_id;
        let status_button = Button::new(("row-status", ix))
            .ghost()
            .xsmall()
            .disabled(busy || statuses.is_empty())
            .tooltip(t!("tasks.list.status_tooltip", name = row.status_name))
            .child(
                div()
                    .size(px(16.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .border_2()
                    .border_color(status_color)
                    .child(div().size(px(6.)).rounded_full().bg(status_color)),
            )
            .dropdown_menu(move |mut menu, _, _| {
                for status in &statuses {
                    let (status, owner) = (status.clone(), status_owner.clone());
                    menu = menu.item(
                        PopupMenuItem::new(status.name.clone())
                            .checked(status.id == current_status)
                            .on_click(move |_, _, cx| {
                                let status = status.clone();
                                let _ = owner
                                    .update(cx, |this, cx| this.set_row_status(id, status, cx));
                            }),
                    );
                }
                menu
            });

        let priority_owner = owner.clone();
        let current_priority = row.priority;
        let priority_button = Button::new(("row-priority", ix))
            .ghost()
            .compact()
            .disabled(busy)
            .child(priority_chip(current_priority))
            .dropdown_menu(move |mut menu, _, _| {
                for priority in PRIORITIES {
                    let owner = priority_owner.clone();
                    menu = menu.item(
                        PopupMenuItem::element(move |_, _| priority_chip(priority))
                            .checked(priority == current_priority)
                            .on_click(move |_, _, cx| {
                                let _ = owner
                                    .update(cx, |this, cx| this.set_row_priority(id, priority, cx));
                            }),
                    );
                }
                menu
            });

        let extra = row.assignees.len().saturating_sub(MAX_AVATARS);
        let avatars: Vec<_> = row
            .assignees
            .iter()
            .take(MAX_AVATARS)
            .map(|a| user_avatar(&a.name, a.avatar_url.as_deref(), Size::Small, cx))
            .collect();

        let selected = self.selected == Some(ix);
        div()
            .id(("task-row", ix))
            .flex()
            .items_center()
            .h(px(ROW_H))
            .px_2()
            .border_b_1()
            .border_color(border.opacity(0.6))
            .cursor_pointer()
            .when(selected, |d| d.bg(selected_bg))
            .when(!selected, |d| d.hover(|d| d.bg(hover)))
            .on_click(cx.listener(move |this, _, window, cx| {
                window.focus(&this.focus_handle, cx);
                this.select_row(ix, cx);
            }))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .gap_2()
                    .px_1()
                    .child(status_button)
                    .child(
                        div()
                            .min_w_0()
                            .text_sm()
                            .text_ellipsis()
                            .when(row.is_done, |d| d.text_color(muted))
                            .child(row.title.clone()),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(muted)
                            .child(row.seq_key.clone()),
                    ),
            )
            .when(show_assignee, |d| {
                d.child(
                    div()
                        .w(px(ASSIGNEE_W))
                        .px_2()
                        .flex()
                        .items_center()
                        .children(avatars.into_iter().enumerate().map(|(ix, avatar)| {
                            div().when(ix > 0, |d| d.ml(px(-6.))).child(avatar)
                        }))
                        .when(extra > 0, |d| {
                            d.child(
                                div()
                                    .ml_1()
                                    .text_xs()
                                    .text_color(muted)
                                    .child(format!("+{extra}")),
                            )
                        }),
                )
            })
            .child(
                div()
                    .w(px(DUE_W))
                    .px_2()
                    .text_sm()
                    .when_some(row.due, |d, due| {
                        d.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_1p5()
                                .text_color(if is_overdue(&due) && !row.is_done {
                                    danger
                                } else {
                                    muted
                                })
                                .child(Icon::new(IconName::Calendar).size_4())
                                .child(div().text_ellipsis().child(due_label(&due))),
                        )
                    }),
            )
            .child(div().w(px(PRIORITY_W)).px_1().child(priority_button))
    }
}

/// 旗アイコン + 優先度名。アイコンと文字の両方に優先度の色を載せる（Web と同じ）。
fn priority_chip(priority: TaskPriority) -> Div {
    div()
        .flex()
        .items_center()
        .gap_1p5()
        .text_sm()
        .text_color(priority_color(priority))
        .child(Icon::new(IconName::Flag).size_4())
        .child(priority_label(priority))
}
