//! §15 一覧。My Tasks / Project の 2 モードを 1 View で持つ。
//!
//! 見た目と操作は Web（koyori-app/task の TaskGroupedList / TaskGroupedRow）に合わせる:
//! ステータス別グループ、列見出しからの並べ替え、行からの担当者・期限・優先度・
//! ラベル・ステータスの変更、その場でのコメント追加、サブタスクの展開。

use std::collections::{HashMap, HashSet};

use api::types::{
    AssigneeInput, CreateCommentRequest, LabelResponse, ProjectStatusResponse, TaskPriority,
    UpdateTaskRequest, UserSummary,
};
use api::{Client, MyTasksQuery, TasksQuery};
use chrono::{Local, NaiveDate, TimeZone, Utc};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::calendar::{Calendar, CalendarEvent, CalendarState, Date};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::popover::Popover;
use gpui_kit::component::{Disableable, Icon, Selectable, Sizable, Size, Theme, WindowExt};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use i18n::t;
use uuid::Uuid;

use crate::avatar::user_avatar;
use crate::model::{
    PRIORITIES, RowAssignee, TaskRow, due_label, is_overdue, parse_hex_color, priority_color,
    priority_label,
};

const PAGE_SIZE: u32 = 50;

/// 列幅（Web の TASK_ROW_GRID: 名前 | 担当 7rem | 期限 7rem | 優先度 6rem | コメント 4rem）。
const ASSIGNEE_W: f32 = 112.;
const DUE_W: f32 = 112.;
const PRIORITY_W: f32 = 104.;
const COMMENT_W: f32 = 36.;
const ROW_H: f32 = 36.;
/// 担当者のアイコンはこの数まで並べ、残りは「+N」。
const MAX_AVATARS: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SortColumn {
    Title,
    Assignee,
    Due,
    Priority,
}

impl SortColumn {
    fn label(self) -> &'static str {
        match self {
            SortColumn::Title => t!("tasks.list.column.name"),
            SortColumn::Assignee => t!("tasks.list.column.assignee"),
            SortColumn::Due => t!("tasks.list.column.due"),
            SortColumn::Priority => t!("tasks.list.column.priority"),
        }
    }

    fn direction_labels(self) -> (&'static str, &'static str) {
        match self {
            SortColumn::Title => (
                t!("tasks.list.sort.title_asc"),
                t!("tasks.list.sort.title_desc"),
            ),
            SortColumn::Assignee => (
                t!("tasks.list.sort.assignee_asc"),
                t!("tasks.list.sort.assignee_desc"),
            ),
            SortColumn::Due => (
                t!("tasks.list.sort.due_asc"),
                t!("tasks.list.sort.due_desc"),
            ),
            SortColumn::Priority => (
                t!("tasks.list.sort.priority_asc"),
                t!("tasks.list.sort.priority_desc"),
            ),
        }
    }

    /// API の `sort`（Web の API_SORT_BY_COLUMN と同じ対応）。
    fn api_name(self) -> &'static str {
        match self {
            SortColumn::Title => "title",
            SortColumn::Assignee => "assignee",
            SortColumn::Due => "deadline",
            SortColumn::Priority => "priority",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Sort {
    column: SortColumn,
    desc: bool,
}

impl Sort {
    fn api_value(self) -> String {
        format!(
            "{}_{}",
            self.column.api_name(),
            if self.desc { "desc" } else { "asc" }
        )
    }

    /// My Tasks はプロジェクトをまたぐので手元で並べる。
    fn compare(self, a: &TaskRow, b: &TaskRow) -> std::cmp::Ordering {
        use std::cmp::Ordering;
        // 値が無いものは向きに関係なく末尾（API と同じ）。
        fn nulls_last<T: Ord>(a: Option<T>, b: Option<T>, desc: bool) -> Ordering {
            match (a, b) {
                (Some(a), Some(b)) if desc => b.cmp(&a),
                (Some(a), Some(b)) => a.cmp(&b),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            }
        }
        let rank = |p: TaskPriority| PRIORITIES.iter().position(|x| *x == p);
        match self.column {
            SortColumn::Title => nulls_last(
                Some(a.title.to_lowercase()),
                Some(b.title.to_lowercase()),
                self.desc,
            ),
            SortColumn::Assignee => nulls_last(
                a.assignees.first().map(|x| x.name.to_lowercase()),
                b.assignees.first().map(|x| x.name.to_lowercase()),
                self.desc,
            ),
            SortColumn::Due => nulls_last(a.due, b.due, self.desc),
            SortColumn::Priority => nulls_last(rank(a.priority), rank(b.priority), self.desc),
        }
    }
}

/// Project のステータス 1 つ分（Web の TaskGroup）。件数はサーバの total。
struct StatusGroup {
    status: ProjectStatusResponse,
    ids: Vec<Uuid>,
    total: i64,
    next_cursor: Option<String>,
    loading: bool,
    failed: bool,
}

/// 展開したタスクのサブタスク。
#[derive(Default)]
struct Children {
    ids: Vec<Uuid>,
    loading: bool,
    failed: bool,
}

/// 一覧の 1 項目。行数が多くても見えている分だけ描画する（`gpui::list`）。
#[derive(Debug, Clone, PartialEq)]
enum Item {
    Header {
        group: usize,
        collapsed: bool,
    },
    More(usize),
    GroupEmpty(usize),
    Failed(usize),
    Row {
        id: Uuid,
        depth: u8,
        status: Option<Uuid>,
    },
    Comment(Uuid),
    SubtasksEmpty(Uuid),
    Bottom,
}

/// 描画するグループ。
struct GroupView {
    /// 折りたたみ状態の鍵。Project は status id、My Tasks は `due:<区分>`。
    key: String,
    name: String,
    color: Option<Hsla>,
    ids: Vec<Uuid>,
    count: i64,
    /// Project のみ: このグループのステータス（続きの取得・サブタスクの絞り込みに使う）。
    status_id: Option<Uuid>,
    more: Option<i64>,
    loading: bool,
    failed: bool,
    /// 既定順（API の新しい順を反転して古い順）では続きが上に増えるので、ボタンも上。
    more_on_top: bool,
}

/// My Tasks の期限区分（鍵, 色）。並びは表示順で、`due_bucket` の返す index と対応する。
const DUE_BUCKETS: [(&str, Option<&str>); 5] = [
    ("overdue", Some("#e5484d")),
    ("today", Some("#1f6feb")),
    ("upcoming", Some("#8b5cf6")),
    ("no_due", None),
    ("done", Some("#238636")),
];

fn due_bucket_name(ix: usize) -> &'static str {
    match ix {
        0 => t!("tasks.list.group_overdue"),
        1 => t!("tasks.list.group_today"),
        2 => t!("tasks.list.group_upcoming"),
        3 => t!("tasks.list.group_no_due"),
        _ => t!("tasks.list.group_done"),
    }
}

fn due_bucket(row: &TaskRow, today: NaiveDate) -> usize {
    if row.is_done {
        return 4;
    }
    match row.due.map(|d| d.with_timezone(&Local).date_naive()) {
        Some(d) if d < today => 0,
        Some(d) if d == today => 1,
        Some(_) => 2,
        None => 3,
    }
}

#[derive(Clone, Copy)]
struct Palette {
    muted: Hsla,
    danger: Hsla,
    hover: Hsla,
    selected: Hsla,
    border: Hsla,
}

impl Palette {
    fn new(cx: &App) -> Self {
        let t = Theme::global(cx);
        Self {
            muted: t.muted_foreground,
            danger: t.danger,
            hover: t.secondary.opacity(0.5),
            selected: t.secondary,
            border: t.border,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ListMode {
    MyTasks,
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
    /// 取得済みの全行（サブタスクを含む）。
    rows: HashMap<Uuid, TaskRow>,
    /// My Tasks 系の並び（取得順）。
    order: Vec<Uuid>,
    /// Project のステータス別グループ。Web と同じくグループごとに取得する。
    project_groups: Vec<StatusGroup>,
    children: HashMap<Uuid, Children>,
    expanded: HashSet<Uuid>,
    /// project_id → ステータス / 担当者候補 / ラベル。
    statuses: HashMap<Uuid, Vec<ProjectStatusResponse>>,
    members: HashMap<Uuid, Vec<UserSummary>>,
    labels: HashMap<Uuid, Vec<LabelResponse>>,
    sort: Option<Sort>,
    loading: bool,
    error: Option<String>,
    shown_error: Option<String>,
    create_input: Entity<InputState>,
    /// 作成成功後に render 側で input をクリアするフラグ
    /// （spawn タスクからは Window に触れないため）。
    clear_create_input: bool,
    creating: bool,
    selected: Option<Uuid>,
    /// 折りたたんだグループの鍵。
    collapsed: HashSet<String>,
    focus_handle: FocusHandle,
    list_state: ListState,
    /// 直近の描画での項目と、その元になったグループ。
    items: Vec<Item>,
    view_groups: Vec<GroupView>,
    /// 直近の描画での (行, 項目 index)。キー操作で使う。
    visible_rows: Vec<(Uuid, usize)>,
    generation: u64,
    context_generation: u64,
    pending: HashSet<Uuid>,
    /// mode 変更後に render 側で作成欄の placeholder を差し替えるフラグ。
    placeholder_dirty: bool,
    /// 期限を編集中の行（カレンダーのポップオーバーを開いている）。
    calendar: Entity<CalendarState>,
    due_editing: Option<Uuid>,
    /// コメント欄を開いている行。
    comment_open: Option<Uuid>,
    comment_input: Entity<TextareaState>,
    comment_posting: bool,
    clear_comment_input: bool,
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
        let calendar = cx.new(|cx| CalendarState::new(window, cx));
        let calendar_sub = cx.subscribe(&calendar, |this, _, ev: &CalendarEvent, cx| {
            let CalendarEvent::Selected(Date::Single(Some(date))) = ev else {
                return;
            };
            if let Some(id) = this.due_editing.take() {
                this.set_row_due(id, Some(*date), cx);
            }
        });
        let comment_input =
            cx.new(|cx| TextareaState::new(window, cx).placeholder(t!("tasks.list.add_comment")));
        Self {
            client,
            tenant,
            mode: ListMode::MyTasks,
            rows: HashMap::new(),
            order: vec![],
            project_groups: vec![],
            children: HashMap::new(),
            expanded: HashSet::new(),
            statuses: HashMap::new(),
            members: HashMap::new(),
            labels: HashMap::new(),
            sort: None,
            loading: false,
            error: None,
            shown_error: None,
            create_input,
            clear_create_input: false,
            creating: false,
            selected: None,
            collapsed: HashSet::new(),
            focus_handle: cx.focus_handle().tab_stop(true),
            list_state: ListState::new(0, ListAlignment::Top, px(200.)),
            items: vec![],
            view_groups: vec![],
            visible_rows: vec![],
            generation: 0,
            context_generation: 0,
            pending: HashSet::new(),
            placeholder_dirty: true,
            calendar,
            due_editing: None,
            comment_open: None,
            comment_input,
            comment_posting: false,
            clear_comment_input: false,
            _subs: vec![sub, calendar_sub],
        }
    }

    pub fn set_client(&mut self, client: Client, tenant: Option<Uuid>, cx: &mut Context<Self>) {
        if self.tenant != tenant {
            self.context_generation += 1;
            self.creating = false;
            self.clear_create_input = true;
            self.clear_loaded();
            self.statuses.clear();
            self.members.clear();
            self.labels.clear();
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
        self.clear_loaded();
        self.statuses.clear();
        self.members.clear();
        self.labels.clear();
        self.generation += 1;
        self.loading = false;
        cx.notify();
    }

    fn clear_loaded(&mut self) {
        self.items.clear();
        self.list_state.reset(0);
        self.rows.clear();
        self.order.clear();
        self.project_groups.clear();
        self.children.clear();
        self.expanded.clear();
        self.selected = None;
        self.due_editing = None;
        self.comment_open = None;
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
        self.clear_loaded();
        self.reload(cx);
    }

    /// Quick Search (§20) 用に現在ロード済みの行を返す。
    pub fn rows_snapshot(&self) -> Vec<TaskRow> {
        let mut rows: Vec<TaskRow> = self.rows.values().cloned().collect();
        rows.sort_by(|a, b| a.seq_key.cmp(&b.seq_key));
        rows
    }

    fn project(&self) -> Option<(Uuid, String)> {
        match &self.mode {
            ListMode::Project { id, key } => Some((*id, key.clone())),
            _ => None,
        }
    }

    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let (Some(client), Some(tenant)) = (self.client.clone(), self.tenant) else {
            return;
        };
        self.loading = true;
        self.error = None;
        self.generation += 1;
        let generation = self.generation;
        cx.notify();
        if let Some((project, key)) = self.project() {
            self.reload_project(client, tenant, project, key, generation, cx);
            return;
        }
        cx.spawn(async move |this, cx| {
            let res = async {
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
                Ok::<_, api::ApiError>(all)
            }
            .await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.loading = false;
                match res {
                    Ok(rows) => {
                        this.order = rows.iter().map(|r| r.id).collect();
                        this.rows = rows.into_iter().map(|r| (r.id, r)).collect();
                        this.resolve_statuses(cx);
                    }
                    Err(e) => this.error = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Project: ステータスを取ってから、ステータスごとに 1 ページ目を取る（Web と同じ）。
    fn reload_project(
        &mut self,
        client: Client,
        tenant: Uuid,
        project: Uuid,
        key: String,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        self.load_project_catalog(project, cx);
        let known = self.statuses.get(&project).cloned();
        cx.spawn(async move |this, cx| {
            let statuses = match known {
                Some(statuses) => Ok(statuses),
                None => client.list_statuses(tenant, project).await,
            };
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.loading = false;
                match statuses {
                    Ok(mut statuses) => {
                        this.statuses.insert(project, statuses.clone());
                        statuses.sort_by_key(|s| s.position);
                        this.rows.clear();
                        this.project_groups = statuses
                            .into_iter()
                            .map(|status| StatusGroup {
                                status,
                                ids: vec![],
                                total: 0,
                                next_cursor: None,
                                loading: true,
                                failed: false,
                            })
                            .collect();
                        for ix in 0..this.project_groups.len() {
                            this.fetch_group(ix, &key, None, cx);
                        }
                        // 開いていたサブタスクは取り直す。
                        let expanded: Vec<(Uuid, Option<Uuid>)> = this
                            .expanded
                            .iter()
                            .map(|id| (*id, this.children_status(*id)))
                            .collect();
                        this.children.clear();
                        for (id, status) in expanded {
                            this.fetch_children(id, status, cx);
                        }
                    }
                    Err(e) => this.error = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 担当者候補とラベル（行のメニューで使う）。プロジェクトごとに一度だけ取る。
    fn load_project_catalog(&mut self, project: Uuid, cx: &mut Context<Self>) {
        let (Some(client), Some(tenant)) = (self.client.clone(), self.tenant) else {
            return;
        };
        if !self.members.contains_key(&project) {
            let client = client.clone();
            cx.spawn(async move |this, cx| {
                if let Ok(users) = client.list_assignable_users(tenant, project, None).await {
                    let _ = this.update(cx, |this, cx| {
                        if this.tenant == Some(tenant) {
                            this.members.insert(project, users);
                            cx.notify();
                        }
                    });
                }
            })
            .detach();
        }
        if !self.labels.contains_key(&project) {
            cx.spawn(async move |this, cx| {
                if let Ok(labels) = client.list_labels(tenant, project).await {
                    let _ = this.update(cx, |this, cx| {
                        if this.tenant == Some(tenant) {
                            this.labels.insert(project, labels);
                            cx.notify();
                        }
                    });
                }
            })
            .detach();
        }
    }

    fn fetch_group(
        &mut self,
        ix: usize,
        key: &str,
        cursor: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let (Some(client), Some(tenant), Some((project, _))) =
            (self.client.clone(), self.tenant, self.project())
        else {
            return;
        };
        let sort = self.sort.map(Sort::api_value);
        let Some(group) = self.project_groups.get_mut(ix) else {
            return;
        };
        group.loading = true;
        group.failed = false;
        let status = group.status.id;
        let q = TasksQuery {
            status_id: Some(status),
            root_only: Some(true),
            is_archived: Some(false),
            sort,
            limit: Some(PAGE_SIZE),
            cursor,
            ..Default::default()
        };
        let generation = self.generation;
        let key = key.to_string();
        cx.spawn(async move |this, cx| {
            let res = client.list_tasks(tenant, project, &q).await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                let Some(ix) = this
                    .project_groups
                    .iter()
                    .position(|g| g.status.id == status)
                else {
                    return;
                };
                match res {
                    Ok(page) => {
                        for task in &page.tasks {
                            let row = TaskRow::from_task(task, &key);
                            let group = &mut this.project_groups[ix];
                            if !group.ids.contains(&row.id) {
                                group.ids.push(row.id);
                            }
                            this.rows.insert(row.id, row);
                        }
                        let group = &mut this.project_groups[ix];
                        group.total = page.total;
                        group.next_cursor = page.next_cursor;
                        group.loading = false;
                        this.apply_statuses();
                    }
                    Err(e) => {
                        let group = &mut this.project_groups[ix];
                        group.loading = false;
                        group.failed = true;
                        this.error = Some(e.to_string());
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn load_more_group(&mut self, status: Uuid, cx: &mut Context<Self>) {
        let Some((_, key)) = self.project() else {
            return;
        };
        let Some(ix) = self
            .project_groups
            .iter()
            .position(|g| g.status.id == status)
        else {
            return;
        };
        let group = &self.project_groups[ix];
        if group.loading {
            return;
        }
        // 失敗したページは同じカーソルで取り直す。
        let cursor = group.next_cursor.clone();
        if cursor.is_none() && !group.failed {
            return;
        }
        self.fetch_group(ix, &key, cursor, cx);
    }

    /// My Tasks: 各行の status が done かを解決する（未取得のプロジェクトは取る）。
    fn resolve_statuses(&mut self, cx: &mut Context<Self>) {
        let missing: Vec<Uuid> = self
            .rows
            .values()
            .map(|r| r.project_id)
            .filter(|p| !self.statuses.contains_key(p))
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        self.apply_statuses();
        let (Some(client), Some(tenant)) = (self.client.clone(), self.tenant) else {
            return;
        };
        for project in missing {
            let client = client.clone();
            cx.spawn(async move |this, cx| {
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
            })
            .detach();
        }
    }

    fn apply_statuses(&mut self) {
        for row in self.rows.values_mut() {
            let Some(status) = self
                .statuses
                .get(&row.project_id)
                .and_then(|ss| ss.iter().find(|s| s.id == row.status_id))
            else {
                continue;
            };
            row.is_done = status.is_done_state;
            if row.status_name.is_empty() {
                row.status_name = status.name.clone();
                row.status_color = status.color.clone();
            }
        }
    }

    // ---- 並べ替え ----

    fn set_sort(&mut self, sort: Option<Sort>, cx: &mut Context<Self>) {
        if self.sort == sort {
            return;
        }
        self.sort = sort;
        // Project はサーバに並べてもらう（ページの切れ目が変わるので取り直す）。
        if self.project().is_some() {
            self.reload(cx);
        }
        cx.notify();
    }

    // ---- サブタスク ----

    /// サブタスクを絞り込むステータス（親が入っているグループ）。
    fn children_status(&self, id: Uuid) -> Option<Uuid> {
        self.project_groups
            .iter()
            .find(|g| g.ids.contains(&id))
            .map(|g| g.status.id)
            .or_else(|| self.rows.get(&id).map(|r| r.status_id))
    }

    fn toggle_subtasks(&mut self, id: Uuid, status: Option<Uuid>, cx: &mut Context<Self>) {
        if self.expanded.remove(&id) {
            cx.notify();
            return;
        }
        self.expanded.insert(id);
        self.fetch_children(id, status, cx);
        cx.notify();
    }

    /// Web と同じく、親と同じグループ（ステータス）のサブタスクを出す。
    fn fetch_children(&mut self, id: Uuid, status: Option<Uuid>, cx: &mut Context<Self>) {
        let (Some(client), Some(tenant), Some((project, key))) =
            (self.client.clone(), self.tenant, self.project())
        else {
            return;
        };
        let entry = self.children.entry(id).or_default();
        entry.loading = true;
        entry.failed = false;
        let q = TasksQuery {
            parent_task_id: Some(id),
            status_id: status,
            is_archived: Some(false),
            sort: self.sort.map(Sort::api_value),
            limit: Some(100),
            ..Default::default()
        };
        let generation = self.generation;
        cx.spawn(async move |this, cx| {
            let res = client.list_tasks(tenant, project, &q).await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                let children = this.children.entry(id).or_default();
                children.loading = false;
                match res {
                    Ok(page) => {
                        children.ids = page.tasks.iter().map(|t| t.id).collect();
                        for task in &page.tasks {
                            let row = TaskRow::from_task(task, &key);
                            this.rows.insert(row.id, row);
                        }
                        this.apply_statuses();
                    }
                    Err(e) => {
                        children.failed = true;
                        this.error = Some(e.to_string());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    // ---- 行からの変更 ----

    fn set_row_status(&mut self, id: Uuid, status: ProjectStatusResponse, cx: &mut Context<Self>) {
        let status_id = status.id;
        self.mutate_row(
            id,
            move |row| {
                row.status_id = status.id;
                row.status_name = status.name.clone();
                row.status_color = status.color.clone();
                row.is_done = status.is_done_state;
            },
            UpdateTaskRequest {
                status_id: Some(status_id),
                ..Default::default()
            },
            // グループ（= ステータス）が変わるので取り直す。
            true,
            cx,
        );
    }

    fn set_row_priority(&mut self, id: Uuid, priority: TaskPriority, cx: &mut Context<Self>) {
        let regroup = self.sort.is_some_and(|s| s.column == SortColumn::Priority);
        self.mutate_row(
            id,
            move |row| row.priority = priority,
            UpdateTaskRequest {
                priority: Some(priority),
                ..Default::default()
            },
            regroup,
            cx,
        );
    }

    fn toggle_row_assignee(&mut self, id: Uuid, user: UserSummary, cx: &mut Context<Self>) {
        let Some(row) = self.rows.get(&id) else {
            return;
        };
        let mut next = row.assignees.clone();
        if let Some(pos) = next.iter().position(|a| a.id == user.id) {
            next.remove(pos);
        } else {
            next.push(RowAssignee {
                id: user.id,
                role: "assignee".into(),
                name: user.username.clone(),
                avatar_url: user.avatar_url.clone(),
            });
        }
        // 担当者は全体置換の API。
        let assignees = next
            .iter()
            .map(|a| AssigneeInput {
                role: a.role.clone(),
                user_id: a.id,
            })
            .collect();
        let regroup = self.sort.is_some_and(|s| s.column == SortColumn::Assignee);
        self.mutate_row(
            id,
            move |row| row.assignees = next,
            UpdateTaskRequest {
                assignees: Some(assignees),
                ..Default::default()
            },
            regroup,
            cx,
        );
    }

    fn toggle_row_label(&mut self, id: Uuid, label: LabelResponse, cx: &mut Context<Self>) {
        let Some(row) = self.rows.get(&id) else {
            return;
        };
        let has = row.labels.iter().any(|l| l.id == label.id);
        let label_id = label.id;
        let req = if has {
            UpdateTaskRequest {
                remove_label_ids: vec![label_id],
                ..Default::default()
            }
        } else {
            UpdateTaskRequest {
                add_label_ids: vec![label_id],
                ..Default::default()
            }
        };
        self.mutate_row(
            id,
            move |row| {
                if has {
                    row.labels.retain(|l| l.id != label_id);
                } else {
                    row.labels.push(label);
                }
            },
            req,
            false,
            cx,
        );
    }

    fn set_row_due(&mut self, id: Uuid, date: Option<NaiveDate>, cx: &mut Context<Self>) {
        let due = date.and_then(|d| {
            Local
                .from_local_datetime(&d.and_hms_opt(0, 0, 0)?)
                .earliest()
                .map(|d| d.with_timezone(&Utc))
        });
        let req = match due {
            Some(due) => UpdateTaskRequest {
                soft_deadline: Some(due),
                ..Default::default()
            },
            // Detail と同じく、外す時は soft / hard の両方を外す。
            None => UpdateTaskRequest {
                clear_soft_deadline: Some(true),
                clear_hard_deadline: Some(true),
                ..Default::default()
            },
        };
        let regroup = self.sort.is_some_and(|s| s.column == SortColumn::Due);
        self.mutate_row(id, move |row| row.due = due, req, regroup, cx);
    }

    /// 行からの変更 + Optimistic Update（失敗で rollback + エラー表示 §23）。
    /// `regroup` は並び・グループが変わる変更（成功後に取り直す）。
    fn mutate_row(
        &mut self,
        id: Uuid,
        apply: impl FnOnce(&mut TaskRow),
        req: UpdateTaskRequest,
        regroup: bool,
        cx: &mut Context<Self>,
    ) {
        let (Some(client), Some(tenant)) = (self.client.clone(), self.tenant) else {
            return;
        };
        if self.pending.contains(&id) {
            return;
        }
        let Some(row) = self.rows.get_mut(&id) else {
            return;
        };
        let previous = row.clone();
        apply(row);
        self.pending.insert(id);
        let context_generation = self.context_generation;
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
                match res {
                    Err(e) => {
                        if let Some(row) = this.rows.get_mut(&id) {
                            *row = previous;
                        }
                        this.error = Some(e.to_string());
                    }
                    Ok(_) => {
                        if this.selected == Some(id) {
                            // Detail を開き直して一覧の変更を反映する。
                            cx.emit(TaskListEvent::Select {
                                project: previous.project_id,
                                task: id,
                            });
                        }
                        if regroup {
                            this.reload(cx);
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    // ---- 期限のカレンダー ----

    fn set_due_editing(&mut self, id: Option<Uuid>, window: &mut Window, cx: &mut Context<Self>) {
        self.due_editing = id;
        if let Some(row) = id.and_then(|id| self.rows.get(&id)) {
            let date = row.due.map(|d| d.with_timezone(&Local).date_naive());
            self.calendar
                .update(cx, |c, cx| c.set_date(Date::Single(date), window, cx));
        }
        cx.notify();
    }

    // ---- コメント ----

    fn toggle_comment(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        if self.comment_open == Some(id) {
            self.comment_open = None;
        } else {
            self.comment_open = Some(id);
            self.clear_comment_input = true;
            self.comment_input
                .update(cx, |input, cx| input.focus(window, cx));
        }
        cx.notify();
    }

    fn post_comment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(client), Some(tenant), Some(id)) =
            (self.client.clone(), self.tenant, self.comment_open)
        else {
            return;
        };
        let body = self.comment_input.read(cx).value().trim().to_string();
        if body.is_empty() || self.comment_posting {
            return;
        }
        let Some(project) = self.rows.get(&id).map(|r| r.project_id) else {
            return;
        };
        self.comment_posting = true;
        cx.notify();
        let context_generation = self.context_generation;
        let window_handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            let res = client
                .create_comment(
                    tenant,
                    project,
                    id,
                    &CreateCommentRequest {
                        body,
                        parent_comment_id: None,
                    },
                )
                .await;
            let _ = this.update(cx, |this, cx| {
                this.comment_posting = false;
                if this.context_generation != context_generation {
                    return;
                }
                match res {
                    // 失敗したら下書きを残したまま欄を開けておく（Web と同じ）。
                    Err(e) => this.error = Some(e.to_string()),
                    Ok(_) => {
                        this.comment_open = None;
                        this.clear_comment_input = true;
                        let _ = cx.update_window(window_handle, |_, window, cx| {
                            window.push_notification(
                                Notification::new().message(t!("tasks.list.comment_posted")),
                                cx,
                            );
                        });
                        if this.selected == Some(id) {
                            cx.emit(TaskListEvent::Select { project, task: id });
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    // ---- 選択 ----

    fn select_row(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let Some(row) = self.rows.get(&id) else {
            return;
        };
        self.selected = Some(id);
        cx.emit(TaskListEvent::Select {
            project: row.project_id,
            task: id,
        });
        if let Some((_, ix)) = self.visible_rows.iter().find(|(row, _)| *row == id) {
            self.list_state.scroll_to_reveal_item(*ix);
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
            .and_then(|id| self.visible_rows.iter().position(|(row, _)| *row == id));
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

    /// 描画用のグループ。Project はステータス別に取得したもの、My Tasks は期限別。
    fn groups(&self) -> Vec<GroupView> {
        if self.project().is_some() {
            return self
                .project_groups
                .iter()
                .map(|g| {
                    let mut ids = g.ids.clone();
                    // 既定順は API の新しい順を反転して古い順にする（Web と同じ）。
                    if self.sort.is_none() {
                        ids.reverse();
                    }
                    GroupView {
                        key: g.status.id.to_string(),
                        name: g.status.name.clone(),
                        color: parse_hex_color(&g.status.color),
                        count: g.total,
                        more: g
                            .next_cursor
                            .as_ref()
                            .map(|_| (g.total - g.ids.len() as i64).max(0)),
                        loading: g.loading,
                        failed: g.failed,
                        more_on_top: self.sort.is_none(),
                        status_id: Some(g.status.id),
                        ids,
                    }
                })
                .collect();
        }
        // My Tasks はプロジェクトをまたぐので期限で分ける（旧 Today / Upcoming を統合）。
        let today = Local::now().date_naive();
        let mut buckets: [Vec<Uuid>; DUE_BUCKETS.len()] = Default::default();
        for id in &self.order {
            if let Some(row) = self.rows.get(id) {
                buckets[due_bucket(row, today)].push(*id);
            }
        }
        DUE_BUCKETS
            .iter()
            .zip(buckets)
            .enumerate()
            .filter(|(_, (_, ids))| !ids.is_empty())
            .map(|(ix, ((key, color), mut ids))| {
                match self.sort {
                    Some(sort) => ids.sort_by(|a, b| sort.compare(&self.rows[a], &self.rows[b])),
                    // 既定は期限の近い順（期限なし・完了は取得順のまま）。
                    None => ids.sort_by_key(|id| self.rows[id].due),
                }
                GroupView {
                    key: format!("due:{key}"),
                    name: due_bucket_name(ix).into(),
                    color: color.and_then(parse_hex_color),
                    count: ids.len() as i64,
                    ids,
                    status_id: None,
                    more: None,
                    loading: false,
                    failed: false,
                    more_on_top: false,
                }
            })
            .collect()
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
        if self.clear_comment_input {
            self.clear_comment_input = false;
            self.comment_input
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
        let c = Theme::global(cx).semantic_tokens().colors;
        let palette = Palette::new(cx);
        let muted = palette.muted;
        let empty_text = match self.mode {
            ListMode::MyTasks => t!("tasks.list.empty_my"),
            ListMode::Project { .. } => t!("tasks.list.empty_project"),
        };
        let project_mode = self.project().is_some();

        let groups = self.groups();
        let items = self.build_items(&groups);
        self.sync_items(items);
        self.view_groups = groups;

        let empty = if self.client.is_none() {
            Some(t!("tasks.list.empty_signed_out"))
        } else if self.view_groups.is_empty() && self.loading {
            Some(t!("tasks.list.loading"))
        } else if self.view_groups.is_empty() {
            Some(empty_text)
        } else {
            None
        };

        let mut header = div()
            .flex()
            .flex_shrink_0()
            .items_center()
            .h(px(32.))
            .px_2()
            .border_b_1()
            .border_color(c.border)
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .pl(px(46.))
                    .child(self.sort_header(SortColumn::Title, cx)),
            );
        if project_mode {
            header = header.child(
                div()
                    .flex()
                    .w(px(ASSIGNEE_W))
                    .child(self.sort_header(SortColumn::Assignee, cx)),
            );
        }
        header = header
            .child(
                div()
                    .flex()
                    .w(px(DUE_W))
                    .child(self.sort_header(SortColumn::Due, cx)),
            )
            .child(
                div()
                    .flex()
                    .w(px(PRIORITY_W))
                    .child(self.sort_header(SortColumn::Priority, cx)),
            )
            .child(div().w(px(COMMENT_W)));

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
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_h_0()
                        .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                            match event.keystroke.key.as_str() {
                                "up" => this.move_selection(-1, cx),
                                "down" => this.move_selection(1, cx),
                                // → / ← でサブタスクの開閉（Project のみ）。
                                key @ ("right" | "left") if this.project().is_some() => {
                                    let Some(id) = this.selected else { return };
                                    if (key == "right") != this.expanded.contains(&id) {
                                        let status = this.children_status(id);
                                        this.toggle_subtasks(id, status, cx);
                                    }
                                }
                                _ => return,
                            }
                            cx.stop_propagation();
                        }))
                        .child(
                            list(
                                self.list_state.clone(),
                                // list は項目を内容の幅で組むので、幅いっぱいに伸ばす。
                                cx.processor(|this, ix, window, cx| {
                                    div()
                                        .w_full()
                                        .flex()
                                        .flex_col()
                                        .child(this.render_item(ix, window, cx))
                                        .into_any_element()
                                }),
                            )
                            .flex_1()
                            .w_full(),
                        ),
                ),
            })
    }
}

impl TaskListView {
    fn render_group_header(
        &self,
        group: &GroupView,
        collapsed: bool,
        muted: Hsla,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let color = group.color.unwrap_or(muted);
        let key = group.key.clone();
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
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.toggle_group(key.clone(), cx)),
                    ),
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
                    .child(group.count.to_string()),
            )
            .into_any_element()
    }

    fn render_more_button(
        &self,
        group: &GroupView,
        remaining: i64,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let status = group.status_id;
        div()
            .flex()
            .pl(px(46.))
            .py_1()
            .child(
                Button::new(SharedString::from(format!("more-{}", group.key)))
                    .ghost()
                    .compact()
                    .label(t!("tasks.list.more_remaining", count = remaining))
                    .disabled(group.loading)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(status) = status {
                            this.load_more_group(status, cx);
                        }
                    })),
            )
            .into_any_element()
    }

    /// 列見出し。押すと昇順 / 降順 / クリアを選べる（Web の TaskListSortHeader）。
    fn sort_header(&self, column: SortColumn, cx: &mut Context<Self>) -> impl IntoElement {
        let current = self.sort.filter(|s| s.column == column);
        let (asc_label, desc_label) = column.direction_labels();
        let owner = cx.entity().downgrade();
        Button::new(SharedString::from(format!("sort-{column:?}")))
            .ghost()
            .xsmall()
            .text_color(Theme::global(cx).muted_foreground)
            .label(column.label())
            .when_some(current, |b, sort| {
                b.icon(if sort.desc {
                    IconName::ArrowDown
                } else {
                    IconName::ArrowUp
                })
            })
            .dropdown_menu(move |mut menu, _, _| {
                for (desc, label, icon) in [
                    (false, asc_label, IconName::ArrowUp),
                    (true, desc_label, IconName::ArrowDown),
                ] {
                    let owner = owner.clone();
                    menu = menu.item(
                        PopupMenuItem::new(label)
                            .icon(icon)
                            .checked(current.is_some_and(|s| s.desc == desc))
                            .on_click(move |_, _, cx| {
                                let _ = owner.update(cx, |this, cx| {
                                    this.set_sort(Some(Sort { column, desc }), cx)
                                });
                            }),
                    );
                }
                let owner = owner.clone();
                menu.separator().item(
                    PopupMenuItem::new(t!("tasks.list.sort.clear"))
                        .icon(IconName::X)
                        .disabled(current.is_none())
                        .on_click(move |_, _, cx| {
                            let _ = owner.update(cx, |this, cx| this.set_sort(None, cx));
                        }),
                )
            })
    }

    /// 行（とサブタスク・コメント欄）を `body` に積む。
    #[allow(clippy::too_many_arguments)] // 描画の文脈をまとめて受け取る
    /// グループを項目の並びに展開する。要素は作らないので全件でも軽い。
    fn build_items(&self, groups: &[GroupView]) -> Vec<Item> {
        let mut items = vec![];
        for (ix, group) in groups.iter().enumerate() {
            let collapsed = self.collapsed.contains(&group.key);
            items.push(Item::Header {
                group: ix,
                collapsed,
            });
            if collapsed {
                continue;
            }
            let more = group.more.is_some() && !group.failed;
            if group.more_on_top && more {
                items.push(Item::More(ix));
            }
            if group.ids.is_empty() && !group.failed {
                items.push(Item::GroupEmpty(ix));
            }
            for id in &group.ids {
                self.push_row_items(*id, 0, group.status_id, &mut items);
            }
            if !group.more_on_top && more {
                items.push(Item::More(ix));
            }
            if group.failed {
                items.push(Item::Failed(ix));
            }
        }
        items.push(Item::Bottom);
        items
    }

    fn push_row_items(&self, id: Uuid, depth: u8, status: Option<Uuid>, items: &mut Vec<Item>) {
        if !self.rows.contains_key(&id) {
            return;
        }
        items.push(Item::Row { id, depth, status });
        if self.comment_open == Some(id) {
            items.push(Item::Comment(id));
        }
        if depth == 0 && self.expanded.contains(&id) {
            let ids = self
                .children
                .get(&id)
                .map(|c| c.ids.clone())
                .unwrap_or_default();
            if ids.is_empty() {
                items.push(Item::SubtasksEmpty(id));
            }
            for child in ids {
                self.push_row_items(child, 1, status, items);
            }
        }
    }

    /// 変わった範囲だけ list に伝える（全体を差し替えるとスクロール位置が先頭に戻る）。
    fn sync_items(&mut self, items: Vec<Item>) {
        self.visible_rows = items
            .iter()
            .enumerate()
            .filter_map(|(ix, item)| match item {
                Item::Row { id, .. } => Some((*id, ix)),
                _ => None,
            })
            .collect();
        if items == self.items {
            return;
        }
        let (start, old_end, new_end) = changed_range(&self.items, &items);
        self.list_state.splice(start..old_end, new_end - start);
        self.items = items;
    }

    fn render_item(&mut self, ix: usize, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let palette = Palette::new(cx);
        let Some(item) = self.items.get(ix).cloned() else {
            return div().into_any_element();
        };
        let group = |g: usize| self.view_groups.get(g);
        match item {
            Item::Header {
                group: g,
                collapsed,
            } => match group(g) {
                Some(group) => self.render_group_header(group, collapsed, palette.muted, cx),
                None => div().into_any_element(),
            },
            Item::More(g) => match group(g) {
                Some(group) => {
                    let remaining = group.more.unwrap_or(0);
                    self.render_more_button(group, remaining, cx)
                }
                None => div().into_any_element(),
            },
            Item::GroupEmpty(g) => div()
                .px_3()
                .py_2()
                .text_sm()
                .text_color(palette.muted)
                .child(if group(g).is_some_and(|g| g.loading) {
                    t!("tasks.list.loading")
                } else {
                    t!("tasks.list.group_empty")
                })
                .into_any_element(),
            Item::Failed(g) => {
                let status = group(g).and_then(|g| g.status_id);
                self.render_group_failed(status, palette, cx)
            }
            Item::Row { id, depth, status } => match self.rows.get(&id).cloned() {
                Some(row) => {
                    let project_mode = self.project().is_some();
                    self.render_row(row, depth, status, project_mode, palette, cx)
                }
                None => div().into_any_element(),
            },
            Item::Comment(_) => self.render_comment_box(palette, cx),
            Item::SubtasksEmpty(id) => {
                let text = match self.children.get(&id) {
                    Some(c) if c.failed => t!("tasks.list.load_failed"),
                    Some(c) if !c.loading => t!("tasks.list.subtasks_empty"),
                    _ => t!("tasks.list.loading"),
                };
                div()
                    .pl(px(64.))
                    .py_1p5()
                    .text_xs()
                    .text_color(palette.muted)
                    .child(text)
                    .into_any_element()
            }
            Item::Bottom => div().h_4().into_any_element(),
        }
    }

    fn render_group_failed(
        &self,
        status: Option<Uuid>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .child(
                div()
                    .text_sm()
                    .text_color(palette.danger)
                    .child(t!("tasks.list.load_failed")),
            )
            .when_some(status, |d, status| {
                d.child(
                    Button::new(SharedString::from(format!("retry-{status}")))
                        .outline()
                        .compact()
                        .label(t!("tasks.list.retry"))
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.load_more_group(status, cx)),
                        ),
                )
            })
            .into_any_element()
    }

    // GPUI の大きな一時値をセルごとのフレームに分け、Windows のスタックに収める。
    #[inline(never)]
    fn render_row(
        &self,
        row: TaskRow,
        depth: u8,
        group_status: Option<Uuid>,
        project_mode: bool,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = row.id;
        let selected = self.selected == Some(id);
        let name_cell = self.render_name_cell(&row, depth, group_status, project_mode, palette, cx);
        let assignee_cell =
            project_mode.then(|| self.render_assignee_cell(&row, palette.muted, cx));
        let due_cell = self.render_due_cell(&row, palette, cx);
        let priority_cell = self.render_priority_cell(&row, cx);
        let comment_cell = self.render_comment_cell(id, cx);

        div()
            .id(SharedString::from(format!("task-row-el-{id}")))
            .group(SharedString::from(format!("task-row-{id}")))
            .flex()
            .items_center()
            .h(px(ROW_H))
            .px_2()
            .border_b_1()
            .border_color(palette.border.opacity(0.6))
            .cursor_pointer()
            .when(selected, |d| d.bg(palette.selected))
            .when(!selected, |d| d.hover(|d| d.bg(palette.hover)))
            .on_click(cx.listener(move |this, _, window, cx| {
                window.focus(&this.focus_handle, cx);
                this.select_row(id, cx);
            }))
            .child(name_cell)
            .children(assignee_cell)
            .child(due_cell)
            .child(priority_cell)
            .child(comment_cell)
            .into_any_element()
    }

    #[inline(never)]
    fn render_subtask_toggle(
        &self,
        id: Uuid,
        depth: u8,
        group_status: Option<Uuid>,
        project_mode: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected = self.selected == Some(id);
        let expanded = self.expanded.contains(&id);
        // サブタスクの開閉。選択中か展開中の行だけに出す（Web と同じ）。
        if project_mode && depth == 0 && (selected || expanded) {
            Button::new(SharedString::from(format!("subtasks-{id}")))
                .ghost()
                .xsmall()
                .icon(if expanded {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                })
                .tooltip(if expanded {
                    t!("tasks.list.subtasks_collapse")
                } else {
                    t!("tasks.list.subtasks_expand")
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.toggle_subtasks(id, group_status, cx);
                    cx.stop_propagation();
                }))
                .into_any_element()
        } else {
            div().size(px(20.)).flex_shrink_0().into_any_element()
        }
    }

    #[inline(never)]
    fn render_status_button(
        &self,
        row: &TaskRow,
        muted: Hsla,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = row.id;
        let busy = self.pending.contains(&id);
        // 名前の左の丸からステータスを変える（Web と同じ）。
        let status_color = parse_hex_color(&row.status_color).unwrap_or(muted);
        let statuses = self
            .statuses
            .get(&row.project_id)
            .cloned()
            .unwrap_or_default();
        let current_status = row.status_id;
        let status_owner = cx.entity().downgrade();
        Button::new(SharedString::from(format!("row-status-{id}")))
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
            })
            .into_any_element()
    }

    #[inline(never)]
    fn render_label_button(
        &self,
        row: &TaskRow,
        muted: Hsla,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = row.id;
        let busy = self.pending.contains(&id);
        let selected = self.selected == Some(id);
        let group_name = SharedString::from(format!("task-row-{id}"));
        // ラベルの付け外し。行にカーソルを合わせたときだけ出す（Web と同じ）。
        let project_labels = self
            .labels
            .get(&row.project_id)
            .cloned()
            .unwrap_or_default();
        let has_labels: HashSet<Uuid> = row.labels.iter().map(|l| l.id).collect();
        let label_owner = cx.entity().downgrade();
        div()
            .invisible()
            .group_hover(group_name.clone(), |s| s.visible())
            .when(selected, |d| d.visible())
            .child(
                Button::new(SharedString::from(format!("row-labels-{id}")))
                    .ghost()
                    .xsmall()
                    .icon(IconName::Tag)
                    .tooltip(t!("tasks.list.labels"))
                    .disabled(busy)
                    .dropdown_menu(move |mut menu, _, _| {
                        if project_labels.is_empty() {
                            return menu.item(
                                PopupMenuItem::new(t!("tasks.list.labels_empty")).disabled(true),
                            );
                        }
                        for label in &project_labels {
                            let (label, owner) = (label.clone(), label_owner.clone());
                            let color = parse_hex_color(&label.color).unwrap_or(muted);
                            let name = label.name.clone();
                            menu = menu.item(
                                PopupMenuItem::element(move |_, _| {
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .child(div().size(px(10.)).rounded_full().bg(color))
                                        .child(name.clone())
                                })
                                .checked(has_labels.contains(&label.id))
                                .on_click(move |_, _, cx| {
                                    let label = label.clone();
                                    let _ = owner.update(cx, |this, cx| {
                                        this.toggle_row_label(id, label, cx)
                                    });
                                }),
                            );
                        }
                        menu
                    }),
            )
            .into_any_element()
    }

    #[inline(never)]
    fn render_name_cell(
        &self,
        row: &TaskRow,
        depth: u8,
        group_status: Option<Uuid>,
        project_mode: bool,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let muted = palette.muted;
        let subtask_toggle =
            self.render_subtask_toggle(row.id, depth, group_status, project_mode, cx);
        let status_button = self.render_status_button(row, muted, cx);
        let label_button = self.render_label_button(row, muted, cx);
        let label_chips: Vec<AnyElement> = row
            .labels
            .iter()
            .map(|label| {
                let color = parse_hex_color(&label.color).unwrap_or(muted);
                div()
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .h(px(20.))
                    .rounded_full()
                    .border_1()
                    .border_color(palette.border)
                    .text_size(px(11.))
                    .text_color(muted)
                    .child(div().size(px(8.)).rounded_full().bg(color))
                    .child(label.name.clone())
                    .into_any_element()
            })
            .collect();

        div()
            .flex()
            .flex_1()
            .min_w_0()
            .items_center()
            .gap_1p5()
            .when(depth == 1, |d| d.pl(px(24.)))
            .child(subtask_toggle)
            .child(status_button)
            .child(
                div()
                    .min_w(px(96.))
                    .text_sm()
                    .text_ellipsis()
                    .when(row.is_done, |d| d.text_color(muted))
                    .child(row.title.clone()),
            )
            .when(!project_mode, |d| {
                d.child(
                    div()
                        .flex_shrink_0()
                        .text_xs()
                        .text_color(muted)
                        .child(row.seq_key.clone()),
                )
            })
            // ラベルもタイトルと一緒に詰める（幅に比例して縮むので長いタイトルが先に縮む）。
            .child(
                div()
                    .flex()
                    .flex_shrink(1.)
                    .min_w_0()
                    .overflow_hidden()
                    .gap_1()
                    .children(label_chips),
            )
            .when(project_mode, |d| d.child(label_button))
            .into_any_element()
    }

    #[inline(never)]
    fn render_assignee_cell(
        &self,
        row: &TaskRow,
        muted: Hsla,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let busy = self.pending.contains(&row.id);
        div()
            .flex()
            .w(px(ASSIGNEE_W))
            .px_1()
            .child(self.assignee_picker(row, busy, muted, cx))
            .into_any_element()
    }

    #[inline(never)]
    fn render_due_cell(
        &self,
        row: &TaskRow,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = row.id;
        let busy = self.pending.contains(&id);
        let muted = palette.muted;
        // 期限。押すとカレンダーを出す（Web は日付入力）。
        let due_owner = cx.entity().downgrade();
        let editing_due = self.due_editing == Some(id);
        let calendar = self.calendar.clone();
        let clear_owner = due_owner.clone();
        let has_due = row.due.is_some();
        let due_trigger = match row.due {
            Some(due) => Button::new(SharedString::from(format!("row-due-{id}")))
                .ghost()
                .compact()
                .disabled(busy)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .text_sm()
                        .text_color(if is_overdue(&due) && !row.is_done {
                            palette.danger
                        } else {
                            muted
                        })
                        .child(Icon::new(IconName::Calendar).size_4())
                        .child(due_label(&due)),
                ),
            None => Button::new(SharedString::from(format!("row-due-{id}")))
                .ghost()
                .xsmall()
                .disabled(busy)
                .icon(IconName::CalendarPlus)
                .tooltip(t!("tasks.list.set_due")),
        };
        div()
            .flex()
            .w(px(DUE_W))
            .px_1()
            .child(
                Popover::new(SharedString::from(format!("due-popover-{id}")))
                    .trigger(due_trigger)
                    .open(editing_due)
                    .on_open_change(move |open, window, cx| {
                        let _ = due_owner.update(cx, |this, cx| {
                            this.set_due_editing(open.then_some(id), window, cx)
                        });
                    })
                    .content(move |_, _, _| {
                        let clear_owner = clear_owner.clone();
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(Calendar::new(&calendar))
                            .when(has_due, |d| {
                                d.child(
                                    Button::new(SharedString::from(format!("row-due-clear-{id}")))
                                        .ghost()
                                        .compact()
                                        .icon(IconName::X)
                                        .label(t!("tasks.list.clear_due"))
                                        .on_click(move |_, _, cx| {
                                            let _ = clear_owner.update(cx, |this, cx| {
                                                this.due_editing = None;
                                                this.set_row_due(id, None, cx);
                                            });
                                        }),
                                )
                            })
                    }),
            )
            .into_any_element()
    }

    #[inline(never)]
    fn render_priority_cell(&self, row: &TaskRow, cx: &mut Context<Self>) -> AnyElement {
        let id = row.id;
        let busy = self.pending.contains(&id);
        let current_priority = row.priority;
        let priority_owner = cx.entity().downgrade();
        let priority_button = Button::new(SharedString::from(format!("row-priority-{id}")))
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

        div()
            .flex()
            .w(px(PRIORITY_W))
            .px_1()
            .child(priority_button)
            .into_any_element()
    }

    #[inline(never)]
    fn render_comment_cell(&self, id: Uuid, cx: &mut Context<Self>) -> AnyElement {
        let comment_button = Button::new(SharedString::from(format!("row-comment-{id}")))
            .ghost()
            .xsmall()
            .icon(IconName::MessageCircle)
            .tooltip(t!("tasks.list.add_comment"))
            .selected(self.comment_open == Some(id))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.toggle_comment(id, window, cx);
                cx.stop_propagation();
            }));

        div()
            .w(px(COMMENT_W))
            .flex()
            .justify_center()
            .child(comment_button)
            .into_any_element()
    }

    /// 担当者の付け外し（Web の TaskAssigneePicker）。
    #[inline(never)]
    fn assignee_picker(
        &self,
        row: &TaskRow,
        busy: bool,
        muted: Hsla,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = row.id;
        let extra = row.assignees.len().saturating_sub(MAX_AVATARS);
        let avatars: Vec<AnyElement> = row
            .assignees
            .iter()
            .take(MAX_AVATARS)
            .map(|a| {
                user_avatar(&a.name, a.avatar_url.as_deref(), Size::Small, cx).into_any_element()
            })
            .collect();
        let members = self
            .members
            .get(&row.project_id)
            .cloned()
            .unwrap_or_default();
        let assigned: HashSet<Uuid> = row.assignees.iter().map(|a| a.id).collect();
        let owner = cx.entity().downgrade();
        Button::new(SharedString::from(format!("row-assignees-{id}")))
            .ghost()
            .compact()
            .disabled(busy)
            .tooltip(t!("tasks.list.assignees"))
            .when(avatars.is_empty(), |b| b.icon(IconName::UserPlus))
            .when(!avatars.is_empty(), |b| {
                b.child(
                    div()
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
            .dropdown_menu(move |mut menu, _, _| {
                if members.is_empty() {
                    return menu
                        .item(PopupMenuItem::new(t!("tasks.list.assignees_empty")).disabled(true));
                }
                for user in &members {
                    let (user, owner) = (user.clone(), owner.clone());
                    let (name, url) = (user.username.clone(), user.avatar_url.clone());
                    menu = menu.item(
                        PopupMenuItem::element(move |_, cx| {
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(user_avatar(&name, url.as_deref(), Size::Small, cx))
                                .child(name.clone())
                        })
                        .checked(assigned.contains(&user.id))
                        .on_click(move |_, _, cx| {
                            let user = user.clone();
                            let _ =
                                owner.update(cx, |this, cx| this.toggle_row_assignee(id, user, cx));
                        }),
                    );
                }
                menu
            })
            .into_any_element()
    }

    /// その場でのコメント追加（Web の行の下に開く欄）。
    fn render_comment_box(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let has_text = !self.comment_input.read(cx).value().trim().is_empty();
        div()
            .flex()
            .items_start()
            .gap_2()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(palette.border)
            .bg(palette.hover)
            .child(
                div().flex_1().min_w_0().child(
                    Textarea::new(&self.comment_input)
                        .h(px(64.))
                        .disabled(self.comment_posting),
                ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        Button::new("row-comment-send")
                            .primary()
                            .compact()
                            .label(t!("tasks.list.comment_send"))
                            .disabled(self.comment_posting || !has_text)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.post_comment(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("row-comment-close")
                            .ghost()
                            .compact()
                            .label(t!("tasks.list.comment_close"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.comment_open = None;
                                cx.notify();
                            })),
                    ),
            )
            .into_any_element()
    }
}

/// 旗アイコン + 優先度名。アイコンと文字の両方に優先度の色を載せる（Web と同じ）。
/// 前後の一致を除いた、変わった範囲 (開始, 旧の終わり, 新の終わり)。
fn changed_range<T: PartialEq>(old: &[T], new: &[T]) -> (usize, usize, usize) {
    let prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let max_suffix = old.len().min(new.len()) - prefix;
    let suffix = old
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take(max_suffix)
        .take_while(|(a, b)| a == b)
        .count();
    (prefix, old.len() - suffix, new.len() - suffix)
}

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

#[cfg(test)]
mod tests {
    // `super::*` だと gpui の `#[test]` マクロが std のものを隠すので個別に import する。
    use super::{Sort, SortColumn, changed_range, due_bucket};
    use crate::model::TaskRow;
    use api::types::TaskPriority;
    use chrono::{Local, TimeZone, Utc};

    fn row(title: &str, priority: TaskPriority, due_day: Option<u32>) -> TaskRow {
        TaskRow {
            id: uuid::Uuid::new_v4(),
            project_id: uuid::Uuid::nil(),
            project_key: "P".into(),
            seq_key: "P-1".into(),
            title: title.into(),
            priority,
            status_id: uuid::Uuid::nil(),
            status_name: String::new(),
            status_color: String::new(),
            due: due_day.map(|d| Utc.with_ymd_and_hms(2026, 9, d, 0, 0, 0).unwrap()),
            is_done: false,
            assignees: vec![],
            labels: vec![],
            parent_task_id: None,
        }
    }

    fn sorted(sort: Sort, rows: &[TaskRow]) -> Vec<String> {
        let mut rows = rows.to_vec();
        rows.sort_by(|a, b| sort.compare(a, b));
        rows.into_iter().map(|r| r.title).collect()
    }

    #[test]
    fn changed_range_keeps_common_ends() {
        assert_eq!(changed_range(&[1, 2, 3], &[1, 2, 3]), (3, 3, 3));
        assert_eq!(changed_range(&[1, 2, 3], &[1, 9, 9, 3]), (1, 2, 3));
        assert_eq!(changed_range(&[1, 2, 3], &[1, 3]), (1, 2, 1));
        assert_eq!(changed_range(&[1, 1], &[1, 1, 1]), (2, 2, 3));
        assert_eq!(changed_range::<i32>(&[], &[1]), (0, 0, 1));
    }

    #[test]
    fn my_tasks_groups_by_due_date() {
        let today = Local::now().date_naive();
        let at = |days: i64| {
            let date = today + chrono::Duration::days(days);
            Local
                .from_local_datetime(&date.and_hms_opt(12, 0, 0).unwrap())
                .unwrap()
                .with_timezone(&Utc)
        };
        let mut r = row("x", TaskPriority::Medium, None);
        assert_eq!(due_bucket(&r, today), 3);
        r.due = Some(at(-1));
        assert_eq!(due_bucket(&r, today), 0);
        r.due = Some(at(0));
        assert_eq!(due_bucket(&r, today), 1);
        r.due = Some(at(1));
        assert_eq!(due_bucket(&r, today), 2);
        r.is_done = true;
        assert_eq!(due_bucket(&r, today), 4);
    }

    #[test]
    fn my_tasks_sort_matches_web_directions() {
        let rows = [
            row("b", TaskPriority::Low, Some(20)),
            row("a", TaskPriority::CriticalFire, None),
            row("c", TaskPriority::High, Some(10)),
        ];
        let by = |column, desc| Sort { column, desc };
        assert_eq!(sorted(by(SortColumn::Title, false), &rows), ["a", "b", "c"]);
        assert_eq!(sorted(by(SortColumn::Title, true), &rows), ["c", "b", "a"]);
        // 優先度の昇順 = 高い順。
        assert_eq!(
            sorted(by(SortColumn::Priority, false), &rows),
            ["a", "c", "b"]
        );
        // 期限は近い順、期限なしは向きに関係なく末尾。
        assert_eq!(sorted(by(SortColumn::Due, false), &rows), ["c", "b", "a"]);
        assert_eq!(sorted(by(SortColumn::Due, true), &rows), ["b", "c", "a"]);
    }
}
