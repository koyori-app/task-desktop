//! §15 一覧。My Tasks / Project の 2 モードを 1 View で持つ。
//!
//! 見た目と操作は Web（koyori-app/task の TaskGroupedList / TaskGroupedRow）に合わせる:
//! ステータス別グループ、列見出しからの並べ替え、行からの担当者・期限・優先度・
//! ラベル・ステータスの変更、その場でのコメント追加、サブタスクの展開。

use std::collections::{HashMap, HashSet};

use api::types::{
    AssigneeInput, BulkUpdateFields, BulkUpdateRequest, CreateCommentRequest, LabelResponse,
    MilestoneResponse, ProjectStatusResponse, SearchTaskHit, SprintResponse, TaskPriority,
    UpdateTaskRequest, UserSummary,
};
use api::{Client, MyTasksQuery, TasksQuery};
use chrono::{Local, NaiveDate};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::calendar::{Calendar, CalendarEvent, CalendarState, Date};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::popover::Popover;
use gpui_kit::component::{Disableable, Icon, Selectable, Sizable, Size, WindowExt};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use i18n::t;
use uuid::Uuid;

use crate::avatar::user_avatar;
use crate::model::{
    PRIORITIES, RowAssignee, TaskRow, due_label, due_timestamp, is_overdue, parse_hex_color,
    priority_color, priority_label, strip_tags,
};

const PAGE_SIZE: u32 = 50;

/// 列幅。名前に幅を回すため、担当はアイコン、期限・優先度は短い表記が収まる分だけ取る。
const ASSIGNEE_W: f32 = 80.;
const DUE_W: f32 = 100.;
const PRIORITY_W: f32 = 84.;
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
    /// 全文検索の結果 1 件（`search_hits` の index）。
    Hit(usize),
    Bottom,
}

/// Project 一覧の絞り込み（`TasksQuery` に載せる）。
#[derive(Debug, Clone, Default, PartialEq)]
struct Filters {
    priority: Option<TaskPriority>,
    assignee: Option<Uuid>,
    label: Option<Uuid>,
    milestone: Option<Uuid>,
    sprint: Option<Uuid>,
    archived: bool,
}

impl Filters {
    fn is_active(&self) -> bool {
        *self != Filters::default()
    }

    fn apply(&self, q: &mut TasksQuery) {
        q.priority = self.priority;
        q.assignee_id = self.assignee;
        q.label_id = self.label;
        q.milestone_id = self.milestone;
        q.sprint_id = self.sprint;
        q.is_archived = Some(self.archived);
    }
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

type Palette = ::ui::Colors;

/// 絞り込み 1 種（メニューの 1 項目）。選んだ後の `Filters` を先に作っておく。
#[derive(Clone)]
struct FilterCategory {
    title: &'static str,
    icon: IconName,
    /// 選択中の値の名前（未選択は None）。
    current: Option<String>,
    /// 「すべて」を選んだ後。
    cleared: Filters,
    /// (名前, 選択中か, 選んだ後)
    options: Vec<(String, bool, Filters)>,
}

fn filter_category<T: Copy + PartialEq>(
    filters: &Filters,
    title: &'static str,
    icon: IconName,
    current: Option<T>,
    options: Vec<(T, String)>,
    set: fn(&mut Filters, Option<T>),
) -> FilterCategory {
    let mut cleared = filters.clone();
    set(&mut cleared, None);
    FilterCategory {
        title,
        icon,
        current: current
            .and_then(|c| options.iter().find(|(v, _)| *v == c))
            .map(|(_, name)| name.clone()),
        cleared,
        options: options
            .into_iter()
            .map(|(value, name)| {
                let mut next = filters.clone();
                set(&mut next, Some(value));
                (name, current == Some(value), next)
            })
            .collect(),
    }
}

/// 行にホバーしたとき（か `show` のとき）だけ見える入れ物。場所は常に取る。
fn hover_only(group: SharedString, show: bool, child: impl IntoElement) -> Div {
    div()
        .flex()
        .items_center()
        .when(!show, |d| d.invisible().group_hover(group, |s| s.visible()))
        .child(child)
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
    /// 作成欄に文字があるか（追加ボタンの表示に使う）。
    create_has_text: bool,
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
    /// 絞り込み（Project のみ）と、その選択肢。
    filters: Filters,
    milestones: HashMap<Uuid, Vec<MilestoneResponse>>,
    sprints: HashMap<Uuid, Vec<SprintResponse>>,
    /// 検索欄。Project は全文検索、My Tasks は読み込み済みの行を絞る。
    search_input: Entity<InputState>,
    search_query: String,
    search_hits: Option<Vec<SearchTaskHit>>,
    search_generation: u64,
    search_loading: bool,
    clear_search_input: bool,
    /// 一括操作の対象（Project のみ）。
    bulk: HashSet<Uuid>,
    bulk_running: bool,
    /// 一覧全体の読み込みに失敗した（空の表示を再試行にする）。
    load_failed: bool,
    notice: Option<String>,
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
        let sub = cx.subscribe(&create_input, |this, input, ev: &InputEvent, cx| match ev {
            InputEvent::PressEnter { .. } => this.create_task(cx),
            // 追加ボタンは入力があるときだけ出す。空かどうかが変わったときだけ描き直す。
            InputEvent::Change => {
                let has_text = !input.read(cx).value().trim().is_empty();
                if has_text != this.create_has_text {
                    this.create_has_text = has_text;
                    cx.notify();
                }
            }
            _ => {}
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
        let search_input = cx
            .new(|cx| InputState::new(window, cx).placeholder(t!("tasks.list.search_placeholder")));
        let search_sub = cx.subscribe(&search_input, |this, input, ev: &InputEvent, cx| {
            match ev {
                InputEvent::Change => {
                    let query = input.read(cx).value().trim().to_string();
                    this.set_search(query, cx);
                }
                // Enter で最初の結果（または表示中の先頭行）を開く。
                InputEvent::PressEnter { .. } => this.move_selection(1, cx),
                _ => {}
            }
        });
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
            create_has_text: false,
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
            filters: Filters::default(),
            milestones: HashMap::new(),
            sprints: HashMap::new(),
            search_input,
            search_query: String::new(),
            search_hits: None,
            search_generation: 0,
            search_loading: false,
            clear_search_input: false,
            bulk: HashSet::new(),
            bulk_running: false,
            load_failed: false,
            notice: None,
            _subs: vec![sub, calendar_sub, search_sub],
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
            self.milestones.clear();
            self.sprints.clear();
            self.filters = Filters::default();
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
        self.milestones.clear();
        self.sprints.clear();
        self.filters = Filters::default();
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
        self.bulk.clear();
        self.load_failed = false;
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
            self.filters = Filters::default();
            self.reset_search();
        }
        self.mode = mode;
        self.clear_loaded();
        self.reload(cx);
    }

    /// プロジェクト設定でステータス・ラベル・計画などが変わった。手元の写しを捨て、
    /// 表示中ならそのプロジェクトを取り直す。
    pub fn invalidate_project(&mut self, project: Uuid, cx: &mut Context<Self>) {
        self.statuses.remove(&project);
        self.members.remove(&project);
        self.labels.remove(&project);
        self.milestones.remove(&project);
        self.sprints.remove(&project);
        if self.project().is_some_and(|(id, _)| id == project) || self.project().is_none() {
            self.reload(cx);
        }
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
        self.load_failed = false;
        self.generation += 1;
        let generation = self.generation;
        cx.notify();
        if self.project().is_some() && !self.search_query.is_empty() {
            self.run_search(cx);
        }
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
                    Err(e) => {
                        this.load_failed = true;
                        this.error = Some(e.to_string());
                    }
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
                    Err(e) => {
                        this.load_failed = true;
                        this.error = Some(e.to_string());
                    }
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
        if !self.milestones.contains_key(&project) {
            let client = client.clone();
            cx.spawn(async move |this, cx| {
                let milestones = client.list_milestones(tenant, project).await;
                let sprints = client.list_sprints(tenant, project, None).await;
                let _ = this.update(cx, |this, cx| {
                    if this.tenant == Some(tenant) {
                        this.milestones
                            .insert(project, milestones.unwrap_or_default());
                        this.sprints.insert(project, sprints.unwrap_or_default());
                        cx.notify();
                    }
                });
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
        let mut q = TasksQuery {
            status_id: Some(status),
            root_only: Some(true),
            sort,
            limit: Some(PAGE_SIZE),
            cursor,
            ..Default::default()
        };
        self.filters.apply(&mut q);
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
            is_archived: Some(self.filters.archived),
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
        let due = date.and_then(|d| due_timestamp(d, &Local));
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
        // 検索結果は行データを持たないので、表示中のプロジェクトで開く。
        let Some(project) = self
            .rows
            .get(&id)
            .map(|r| r.project_id)
            .or_else(|| self.project().map(|(p, _)| p))
        else {
            return;
        };
        self.selected = Some(id);
        cx.emit(TaskListEvent::Select { project, task: id });
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
        if self.project().is_some() && !self.search_query.is_empty() {
            let count = self.search_hits.as_ref().map_or(0, Vec::len);
            return vec![GroupView {
                key: "search".into(),
                name: t!("tasks.list.search_results").into(),
                color: None,
                ids: vec![],
                count: count as i64,
                status_id: None,
                more: None,
                loading: self.search_loading,
                failed: false,
                more_on_top: false,
            }];
        }
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
        let needle = self.search_query.to_lowercase();
        for id in &self.order {
            if let Some(row) = self.rows.get(id)
                && (needle.is_empty()
                    || row.title.to_lowercase().contains(&needle)
                    || row.seq_key.to_lowercase().contains(&needle))
            {
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

    // ---- 絞り込み・検索 ----

    fn set_filters(&mut self, filters: Filters, cx: &mut Context<Self>) {
        if self.filters == filters {
            return;
        }
        self.filters = filters;
        self.bulk.clear();
        self.reload(cx);
    }

    fn reset_search(&mut self) {
        self.search_generation += 1;
        self.search_query.clear();
        self.search_hits = None;
        self.search_loading = false;
        self.clear_search_input = true;
    }

    fn set_search(&mut self, query: String, cx: &mut Context<Self>) {
        if self.search_query == query {
            return;
        }
        self.search_query = query;
        self.selected = None;
        if self.search_query.is_empty() {
            self.search_generation += 1;
            self.search_hits = None;
            self.search_loading = false;
        } else if self.project().is_some() {
            self.run_search(cx);
        }
        cx.notify();
    }

    /// 全文検索（入力が止まってから送る）。
    fn run_search(&mut self, cx: &mut Context<Self>) {
        let (Some(client), Some(tenant), Some((project, _))) =
            (self.client.clone(), self.tenant, self.project())
        else {
            return;
        };
        self.search_generation += 1;
        let generation = self.search_generation;
        let query = self.search_query.clone();
        self.search_loading = true;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(250))
                .await;
            if !this
                .update(cx, |this, _| this.search_generation == generation)
                .unwrap_or(false)
            {
                return;
            }
            let res = client.search_tasks(tenant, project, &query).await;
            let _ = this.update(cx, |this, cx| {
                if this.search_generation != generation {
                    return;
                }
                this.search_loading = false;
                match res {
                    Ok(found) => this.search_hits = Some(found.tasks),
                    Err(e) => {
                        this.search_hits = Some(vec![]);
                        this.error = Some(e.to_string());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    // ---- 一括操作 ----

    fn toggle_bulk(&mut self, id: Uuid, cx: &mut Context<Self>) {
        if self.project().is_none() {
            return;
        }
        if !self.bulk.remove(&id) {
            self.bulk.insert(id);
        }
        cx.notify();
    }

    /// 選択中のタスクへ同じ変更を一度に送る。失敗した分は理由を知らせる。
    fn bulk_update(&mut self, update: BulkUpdateFields, cx: &mut Context<Self>) {
        let (Some(client), Some(tenant), Some((project, _))) =
            (self.client.clone(), self.tenant, self.project())
        else {
            return;
        };
        if self.bulk.is_empty() || self.bulk_running {
            return;
        }
        let req = BulkUpdateRequest {
            task_ids: self.bulk.iter().copied().collect(),
            update,
        };
        self.bulk_running = true;
        let context_generation = self.context_generation;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let res = client.bulk_update_tasks(tenant, project, &req).await;
            let _ = this.update(cx, |this, cx| {
                this.bulk_running = false;
                if this.context_generation != context_generation {
                    return;
                }
                match res {
                    Ok(result) => {
                        if result.failed.is_empty() {
                            this.notice = Some(t!("tasks.list.bulk_done", count = result.updated));
                            this.bulk.clear();
                        } else {
                            let reasons = result
                                .failed
                                .iter()
                                .map(|f| f.reason.clone())
                                .collect::<HashSet<_>>()
                                .into_iter()
                                .collect::<Vec<_>>()
                                .join(", ");
                            // 失敗したものだけ選択に残して再実行できるようにする。
                            this.bulk = result.failed.iter().map(|f| f.task_id).collect();
                            this.error = Some(t!(
                                "tasks.list.bulk_partial",
                                updated = result.updated,
                                failed = result.failed.len(),
                                reasons = reasons
                            ));
                        }
                        this.reload(cx);
                        if let Some(task) = this.selected {
                            cx.emit(TaskListEvent::Select { project, task });
                        }
                    }
                    Err(e) => this.error = Some(e.to_string()),
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
            let req = crate::model::create_request(title.clone(), status_id, None);
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
        if std::mem::take(&mut self.clear_search_input) {
            self.search_input
                .update(cx, |s, cx| s.set_value("", window, cx));
        }
        if let Some(notice) = self.notice.take() {
            window.push_notification(Notification::new().message(notice), cx);
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
        let palette = ::ui::colors(cx);
        let c = palette;
        let has_title = self.create_has_text;
        let muted = palette.text_muted;
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
        } else if self.view_groups.is_empty() && self.load_failed {
            Some(t!("tasks.list.load_failed"))
        } else if self.view_groups.is_empty() && !self.search_query.is_empty() {
            Some(t!("tasks.list.search_empty"))
        } else if self.view_groups.is_empty() && self.filters.is_active() {
            Some(t!("tasks.list.filter_empty"))
        } else if self.view_groups.is_empty() {
            Some(empty_text)
        } else {
            None
        };
        let failed = self.load_failed && self.view_groups.is_empty();
        let filtered = self.filters.is_active() && self.view_groups.is_empty();
        let filter_bar = self.render_filter_bar(palette, cx);
        let bulk_bar =
            (project_mode && !self.bulk.is_empty()).then(|| self.render_bulk_bar(palette, cx));

        let mut header = div()
            .flex()
            .flex_shrink_0()
            .items_center()
            .h(px(32.))
            .px_4()
            .border_b_1()
            .border_color(c.border)
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    // 一括選択のチェックボックスの分だけ Project は右へずらす。
                    .pl(px(if project_mode { 68. } else { 46. }))
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
                    .h(px(44.))
                    .px_4()
                    .border_b_1()
                    .border_color(c.border)
                    .child(
                        div().flex_1().min_w_0().child(
                            Input::new(&self.create_input)
                                .id("new-task-title")
                                .appearance(false)
                                .prefix(Icon::new(IconName::Plus).small().text_color(muted))
                                .disabled(self.creating),
                        ),
                    )
                    .when(has_title || self.creating, |d| {
                        d.child(
                            Button::new("create-task")
                                .primary()
                                .small()
                                .label(if self.creating {
                                    t!("tasks.list.creating")
                                } else {
                                    t!("tasks.list.add")
                                })
                                .disabled(self.creating)
                                .on_click(cx.listener(|this, _, _, cx| this.create_task(cx))),
                        )
                    }),
            )
            .child(filter_bar)
            .children(bulk_bar)
            .map(|d| match empty {
                Some(text) => d.child(
                    ::ui::empty_state(
                        &palette,
                        if failed {
                            IconName::CircleAlert
                        } else if filtered || !self.search_query.is_empty() {
                            IconName::SearchX
                        } else {
                            IconName::Inbox
                        },
                        text,
                        None,
                    )
                    .flex_1()
                    .when(failed, |d| {
                        d.child(
                            Button::new("list-retry")
                                .outline()
                                .small()
                                .icon(IconName::RefreshCw)
                                .label(t!("tasks.list.retry"))
                                .on_click(cx.listener(|this, _, _, cx| this.reload(cx))),
                        )
                    })
                    .when(filtered, |d| {
                        d.child(
                            Button::new("list-clear-filters")
                                .outline()
                                .small()
                                .icon(IconName::FunnelX)
                                .label(t!("tasks.list.filter_clear"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.set_filters(Filters::default(), cx)
                                })),
                        )
                    }),
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
                                // Enter で選択中のタスクを開き直す（Detail を最新にする）。
                                "enter" => {
                                    let Some(id) = this.selected else { return };
                                    this.select_row(id, cx);
                                }
                                // x で一括操作の対象に入れる / 外す（Project のみ）。
                                "x" if this.project().is_some() => {
                                    let Some(id) = this.selected else { return };
                                    this.toggle_bulk(id, cx);
                                }
                                "escape" if !this.bulk.is_empty() => {
                                    this.bulk.clear();
                                    cx.notify();
                                }
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
            .px_4()
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
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
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
        let muted = ::ui::colors(cx).text_muted;
        div()
            .flex()
            .pl(px(56.))
            .py_0p5()
            .child(
                Button::new(SharedString::from(format!("more-{}", group.key)))
                    .ghost()
                    .xsmall()
                    .text_color(muted)
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
            .text_color(::ui::colors(cx).text_muted)
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
        if let (Some(_), false) = (self.project(), self.search_query.is_empty()) {
            items.push(Item::Header {
                group: 0,
                collapsed: false,
            });
            match &self.search_hits {
                Some(hits) if !hits.is_empty() => {
                    items.extend((0..hits.len()).map(Item::Hit));
                }
                _ => items.push(Item::GroupEmpty(0)),
            }
            items.push(Item::Bottom);
            return items;
        }
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
                Item::Hit(hit) => self
                    .search_hits
                    .as_ref()
                    .and_then(|h| h.get(*hit))
                    .map(|h| (h.id, ix)),
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
        let palette = ::ui::colors(cx);
        let Some(item) = self.items.get(ix).cloned() else {
            return div().into_any_element();
        };
        let group = |g: usize| self.view_groups.get(g);
        match item {
            Item::Header {
                group: g,
                collapsed,
            } => match group(g) {
                Some(group) => self.render_group_header(group, collapsed, palette.text_muted, cx),
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
                .text_color(palette.text_muted)
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
                    .text_color(palette.text_muted)
                    .child(text)
                    .into_any_element()
            }
            Item::Hit(hit) => match self.search_hits.as_ref().and_then(|h| h.get(hit)).cloned() {
                Some(hit) => self.render_hit(hit, palette, cx),
                None => div().into_any_element(),
            },
            Item::Bottom => div().h_4().into_any_element(),
        }
    }

    /// 全文検索の 1 件。行データは持たないので、キー・タイトル・一致箇所だけ出す。
    #[inline(never)]
    fn render_hit(
        &self,
        hit: SearchTaskHit,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = hit.id;
        let selected = self.selected == Some(id);
        let key = match self.project() {
            Some((_, key)) => format!("{key}-{}", hit.seq_id),
            None => format!("#{}", hit.seq_id),
        };
        let snippet = strip_tags(&hit.highlight);
        let snippet = snippet.trim();
        let show_snippet = !snippet.is_empty() && snippet != hit.title;
        div()
            .id(SharedString::from(format!("task-hit-{id}")))
            .flex()
            .items_center()
            .gap_2()
            .h(px(ROW_H))
            .px_3()
            .border_b_1()
            .border_color(palette.border.opacity(0.6))
            .cursor_pointer()
            .when(selected, |d| d.bg(palette.selected))
            .when(!selected, |d| d.hover(|d| d.bg(palette.hover)))
            .on_click(cx.listener(move |this, _, window, cx| {
                window.focus(&this.focus_handle, cx);
                this.select_row(id, cx);
            }))
            .child(
                Icon::new(IconName::Search)
                    .size_3()
                    .text_color(palette.text_muted),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .text_xs()
                    .text_color(palette.text_muted)
                    .child(key),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .max_w(px(320.))
                    .text_sm()
                    .text_ellipsis()
                    .child(hit.title.clone()),
            )
            .when(show_snippet, |d| {
                d.child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_xs()
                        .text_color(palette.text_muted)
                        .text_ellipsis()
                        .child(snippet.to_string()),
                )
            })
            .into_any_element()
    }

    /// 検索欄と、Project の絞り込み（優先度・担当者・ラベル・マイルストーン・スプリント・アーカイブ）。
    #[inline(never)]
    fn render_filter_bar(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let search = div().w(px(220.)).child(
            Input::new(&self.search_input)
                .id("task-search")
                .small()
                .prefix(
                    Icon::new(IconName::Search)
                        .small()
                        .text_color(palette.text_muted),
                )
                .cleanable(true),
        );
        let bar = ::ui::toolbar(&palette).flex_wrap().child(search);
        let Some((project, _)) = self.project() else {
            return bar.into_any_element();
        };
        let f = &self.filters;
        let names = |items: Option<Vec<(Uuid, String)>>| items.unwrap_or_default();
        let categories = vec![
            filter_category(
                f,
                t!("tasks.list.column.priority"),
                IconName::Flag,
                f.priority,
                PRIORITIES
                    .iter()
                    .map(|p| (*p, priority_label(*p).to_string()))
                    .collect(),
                |f, v| f.priority = v,
            ),
            filter_category(
                f,
                t!("tasks.list.column.assignee"),
                IconName::User,
                f.assignee,
                names(
                    self.members
                        .get(&project)
                        .map(|m| m.iter().map(|u| (u.id, u.username.clone())).collect()),
                ),
                |f, v| f.assignee = v,
            ),
            filter_category(
                f,
                t!("tasks.list.labels"),
                IconName::Tag,
                f.label,
                names(
                    self.labels
                        .get(&project)
                        .map(|l| l.iter().map(|x| (x.id, x.name.clone())).collect()),
                ),
                |f, v| f.label = v,
            ),
            filter_category(
                f,
                t!("tasks.detail.milestone"),
                IconName::Milestone,
                f.milestone,
                names(
                    self.milestones
                        .get(&project)
                        .map(|l| l.iter().map(|x| (x.id, x.name.clone())).collect()),
                ),
                |f, v| f.milestone = v,
            ),
            filter_category(
                f,
                t!("tasks.detail.sprint"),
                IconName::Zap,
                f.sprint,
                names(
                    self.sprints
                        .get(&project)
                        .map(|l| l.iter().map(|x| (x.id, x.name.clone())).collect()),
                ),
                |f, v| f.sprint = v,
            ),
        ];
        let active =
            categories.iter().filter(|c| c.current.is_some()).count() + usize::from(f.archived);
        let archived_toggle = {
            let mut next = f.clone();
            next.archived = !next.archived;
            next
        };
        // 選んでいる条件だけを、外せるチップで並べる。
        let chips: Vec<AnyElement> = categories
            .iter()
            .filter_map(|c| Some((c, c.current.clone()?)))
            .enumerate()
            .map(|(ix, (c, value))| {
                let cleared = c.cleared.clone();
                Button::new(("filter-chip", ix))
                    .outline()
                    .xsmall()
                    .icon(c.icon)
                    .label(format!("{} : {value}", c.title))
                    .tooltip(t!("tasks.list.filter_remove"))
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.set_filters(cleared.clone(), cx)),
                    )
                    .into_any_element()
            })
            .chain(f.archived.then(|| {
                let next = archived_toggle.clone();
                Button::new("filter-chip-archived")
                    .outline()
                    .xsmall()
                    .icon(IconName::Archive)
                    .label(t!("tasks.list.show_archived"))
                    .tooltip(t!("tasks.list.filter_remove"))
                    .on_click(cx.listener(move |this, _, _, cx| this.set_filters(next.clone(), cx)))
                    .into_any_element()
            }))
            .collect();
        let owner = cx.entity().downgrade();
        let archived = f.archived;
        let menu = Button::new("filter-menu")
            .ghost()
            .small()
            .icon(IconName::ListFilter)
            .label(if active > 0 {
                t!("tasks.list.filter_count", count = active)
            } else {
                t!("tasks.list.filter").to_string()
            })
            .selected(active > 0)
            .dropdown_menu(move |mut menu, window, cx| {
                for category in &categories {
                    let category = category.clone();
                    let owner = owner.clone();
                    let label = match &category.current {
                        Some(value) => format!("{} : {value}", category.title),
                        None => category.title.to_string(),
                    };
                    menu = menu.submenu_with_icon(
                        Some(Icon::new(category.icon)),
                        label,
                        window,
                        cx,
                        move |mut sub, _, _| {
                            let all_owner = owner.clone();
                            let cleared = category.cleared.clone();
                            sub = sub.item(
                                PopupMenuItem::new(t!("tasks.list.filter_all"))
                                    .checked(category.current.is_none())
                                    .on_click(move |_, _, cx| {
                                        let cleared = cleared.clone();
                                        let _ = all_owner
                                            .update(cx, |this, cx| this.set_filters(cleared, cx));
                                    }),
                            );
                            if category.options.is_empty() {
                                return sub.item(
                                    PopupMenuItem::new(t!("tasks.detail.none")).disabled(true),
                                );
                            }
                            for (name, checked, next) in &category.options {
                                let (next, owner) = (next.clone(), owner.clone());
                                sub = sub.item(
                                    PopupMenuItem::new(name.clone()).checked(*checked).on_click(
                                        move |_, _, cx| {
                                            let next = next.clone();
                                            let _ = owner
                                                .update(cx, |this, cx| this.set_filters(next, cx));
                                        },
                                    ),
                                );
                            }
                            sub.max_h(px(360.)).scrollable(true)
                        },
                    );
                }
                let (archived_owner, clear_owner) = (owner.clone(), owner.clone());
                let next = archived_toggle.clone();
                menu.separator()
                    .item(
                        PopupMenuItem::new(t!("tasks.list.show_archived"))
                            .icon(IconName::Archive)
                            .checked(archived)
                            .on_click(move |_, _, cx| {
                                let next = next.clone();
                                let _ = archived_owner
                                    .update(cx, |this, cx| this.set_filters(next, cx));
                            }),
                    )
                    .separator()
                    .item(
                        PopupMenuItem::new(t!("tasks.list.filter_clear"))
                            .icon(IconName::FunnelX)
                            .disabled(active == 0)
                            .on_click(move |_, _, cx| {
                                let _ = clear_owner.update(cx, |this, cx| {
                                    this.set_filters(Filters::default(), cx)
                                });
                            }),
                    )
            });
        bar.child(menu).children(chips).into_any_element()
    }

    /// 一括操作のバー（選択中の件数と、まとめて変える項目）。
    #[inline(never)]
    fn render_bulk_bar(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let Some((project, _)) = self.project() else {
            return div().into_any_element();
        };
        let owner = cx.entity().downgrade();
        let busy = self.bulk_running;
        type Pick = (String, BulkUpdateFields);
        let menu_button =
            |id: &'static str, icon: IconName, label: &'static str, items: Vec<Pick>| {
                let owner = owner.clone();
                Button::new(id)
                    .outline()
                    .xsmall()
                    .icon(icon)
                    .label(label)
                    .dropdown_caret(true)
                    .disabled(busy)
                    .dropdown_menu(move |mut menu, _, _| {
                        if items.is_empty() {
                            return menu
                                .item(PopupMenuItem::new(t!("tasks.detail.none")).disabled(true));
                        }
                        for (name, update) in &items {
                            let (update, owner) = (update.clone(), owner.clone());
                            menu = menu.item(PopupMenuItem::new(name.clone()).on_click(
                                move |_, _, cx| {
                                    let update = update.clone();
                                    let _ =
                                        owner.update(cx, |this, cx| this.bulk_update(update, cx));
                                },
                            ));
                        }
                        menu
                    })
            };
        let statuses = self
            .statuses
            .get(&project)
            .map(|ss| {
                ss.iter()
                    .map(|s| {
                        (
                            s.name.clone(),
                            BulkUpdateFields {
                                status_id: Some(s.id),
                                ..Default::default()
                            },
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        let members = self
            .members
            .get(&project)
            .map(|ms| {
                ms.iter()
                    .map(|u| {
                        (
                            u.username.clone(),
                            BulkUpdateFields {
                                assignee_id: Some(u.id),
                                ..Default::default()
                            },
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        let labels = self.labels.get(&project).cloned().unwrap_or_default();
        let add_labels = labels
            .iter()
            .map(|l| {
                (
                    l.name.clone(),
                    BulkUpdateFields {
                        add_label_ids: Some(vec![l.id]),
                        ..Default::default()
                    },
                )
            })
            .collect();
        let remove_labels = labels
            .iter()
            .map(|l| {
                (
                    l.name.clone(),
                    BulkUpdateFields {
                        remove_label_ids: Some(vec![l.id]),
                        ..Default::default()
                    },
                )
            })
            .collect();
        let mut sprints: Vec<Pick> = self
            .sprints
            .get(&project)
            .map(|ss| {
                ss.iter()
                    .filter(|s| s.status != api::types::SprintStatus::Completed)
                    .map(|s| {
                        (
                            s.name.clone(),
                            BulkUpdateFields {
                                sprint_id: Some(s.id),
                                ..Default::default()
                            },
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        sprints.push((
            t!("tasks.list.bulk_clear_sprint").into(),
            BulkUpdateFields {
                clear_sprint_id: Some(true),
                ..Default::default()
            },
        ));
        div()
            .flex()
            .flex_wrap()
            .flex_shrink_0()
            .items_center()
            .gap_1()
            .px_4()
            .py_1p5()
            .border_b_1()
            .border_color(palette.border)
            .bg(palette.hover)
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .pr_2()
                    .child(t!("tasks.list.bulk_selected", count = self.bulk.len())),
            )
            .child(menu_button(
                "bulk-status",
                IconName::CircleDot,
                t!("tasks.detail.status"),
                statuses,
            ))
            .child(menu_button(
                "bulk-assignee",
                IconName::UserPlus,
                t!("tasks.list.bulk_assign"),
                members,
            ))
            .child(menu_button(
                "bulk-add-label",
                IconName::TagPlus,
                t!("tasks.list.bulk_add_label"),
                add_labels,
            ))
            .child(menu_button(
                "bulk-remove-label",
                IconName::TagX,
                t!("tasks.list.bulk_remove_label"),
                remove_labels,
            ))
            .child(menu_button(
                "bulk-sprint",
                IconName::Zap,
                t!("tasks.detail.sprint"),
                sprints,
            ))
            .child(div().flex_1())
            .child(
                Button::new("bulk-clear")
                    .ghost()
                    .xsmall()
                    .icon(IconName::X)
                    .label(t!("tasks.list.bulk_clear"))
                    .tooltip(t!("tasks.list.bulk_hint"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.bulk.clear();
                        cx.notify();
                    })),
            )
            .into_any_element()
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
                        .small()
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
            project_mode.then(|| self.render_assignee_cell(&row, palette.text_muted, cx));
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
            .mx_2()
            .rounded_md()
            .cursor_pointer()
            .when(selected, |d| d.bg(palette.selected))
            .when(!selected, |d| d.hover(|d| d.bg(palette.hover)))
            .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
                window.focus(&this.focus_handle, cx);
                if ev.modifiers().secondary() && project_mode {
                    this.toggle_bulk(id, cx);
                    return;
                }
                this.select_row(id, cx);
            }))
            .when(self.bulk.contains(&id), |d| {
                d.bg(palette.selected.opacity(0.7))
            })
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
        let muted = palette.text_muted;
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

        let id = row.id;
        // 一括選択の枠は、何か選んでいる間か行にホバーしたときだけ出す。
        let bulk_box = project_mode.then(|| {
            hover_only(
                SharedString::from(format!("task-row-{id}")),
                !self.bulk.is_empty(),
                Checkbox::new(SharedString::from(format!("bulk-{id}")))
                    .checked(self.bulk.contains(&id))
                    .tooltip(t!("tasks.list.bulk_hint"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.toggle_bulk(id, cx);
                    })),
            )
        });
        div()
            .flex()
            .flex_1()
            .min_w_0()
            .items_center()
            .gap_1p5()
            .children(bulk_box)
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
            .child(hover_only(
                SharedString::from(format!("task-row-{}", row.id)),
                !row.assignees.is_empty() || self.selected == Some(row.id),
                self.assignee_picker(row, busy, muted, cx),
            ))
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
        let muted = palette.text_muted;
        // 期限。押すとカレンダーを出す（Web は日付入力）。
        let due_owner = cx.entity().downgrade();
        let editing_due = self.due_editing == Some(id);
        let calendar = self.calendar.clone();
        let clear_owner = due_owner.clone();
        let has_due = row.due.is_some();
        let due_trigger = match row.due {
            Some(due) => Button::new(SharedString::from(format!("row-due-{id}")))
                .ghost()
                .small()
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
                        .child(Icon::new(IconName::Calendar).small())
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
            .child(hover_only(
                SharedString::from(format!("task-row-{id}")),
                has_due || editing_due,
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
                                        .small()
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
            ))
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
            .small()
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
            .child(hover_only(
                SharedString::from(format!("task-row-{id}")),
                self.comment_open == Some(id) || self.selected == Some(id),
                comment_button,
            ))
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
            .small()
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
                            .small()
                            .label(t!("tasks.list.comment_send"))
                            .disabled(self.comment_posting || !has_text)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.post_comment(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("row-comment-close")
                            .ghost()
                            .small()
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
