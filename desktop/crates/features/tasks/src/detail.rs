//! §15 Detail ペイン。プロパティ（Status / Priority / Assignee / 期限 / ラベル /
//! マイルストーン / スプリント / 見積 / 進捗 / カスタムフィールド）の編集と、
//! コメント・サブタスク・依存関係・履歴・作業時間・添付のタブ。
//! タスク本体の更新は Optimistic + rollback（§23）。描画は `sections` に分ける。

mod sections;

use std::collections::HashMap;
use std::future::Future;
use std::rc::Rc;

use crate::markdown::MarkdownBlocks;
use crate::model::{create_request, due_timestamp, parse_minutes};
use api::Client;
use api::types::{
    ActivityItem, AssigneeInput, CommentThread, CreateCommentRequest, CreateTimeLogRequest,
    CustomFieldType, CustomFieldValueInput, LabelResponse, MilestoneResponse,
    ProjectCustomFieldResponse, ProjectStatusResponse, SearchTaskHit, SprintResponse,
    TaskAssigneeSummary, TaskAttachmentResponse, TaskDetailResponse, TaskPriority,
    TaskRelationsResponse, TimeLogResponse, TimeLogSummaryResponse, TimerStatusResponse,
    UpdateTaskRequest, UserSummary, WatcherUser,
};
use chrono::NaiveDate;
use gpui_kit::component::WindowExt;
use gpui_kit::component::button::ButtonVariant;
use gpui_kit::component::calendar::Date;
use gpui_kit::component::date_picker::{DatePickerEvent, DatePickerState};
use gpui_kit::component::dialog::DialogButtonProps;
use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use gpui_kit::*;
use i18n::t;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub enum TaskDetailEvent {
    Updated {
        project: Uuid,
        task: Uuid,
    },
    /// 削除・アーカイブした。一覧から消すこと（Detail は空になる）。
    Removed {
        project: Uuid,
        task: Uuid,
    },
    /// サブタスク・親・関連タスクを開く。
    OpenTask {
        project: Uuid,
        task: Uuid,
    },
}

/// 下段のタブ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Comments,
    Subtasks,
    Relations,
    Activity,
    Time,
    Files,
}

impl Tab {
    const ALL: [Tab; 6] = [
        Tab::Comments,
        Tab::Subtasks,
        Tab::Relations,
        Tab::Activity,
        Tab::Time,
        Tab::Files,
    ];
}

/// 遅延読み込みする区画の状態。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Load {
    Idle,
    Loading,
    Loaded,
    Failed,
}

/// 依存関係の向き（API の `type`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RelationKind {
    Blocks,
    BlockedBy,
}

impl RelationKind {
    fn api(self) -> &'static str {
        match self {
            RelationKind::Blocks => "blocks",
            RelationKind::BlockedBy => "blocked_by",
        }
    }
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
    /// 本体以外の操作（コメント・関係・時間など）の実行中。
    acting: bool,
    generation: u64,
    client_generation: u64,
    error: Option<String>,
    shown_error: Option<String>,
    /// 成功の知らせ（render で Toast に出す）。
    notice: Option<String>,
    title_input: Entity<InputState>,
    due_picker: Entity<DatePickerState>,
    hard_due_picker: Entity<DatePickerState>,
    estimate_input: Entity<InputState>,
    progress_input: Entity<InputState>,
    comment_input: Entity<TextareaState>,
    edit_input: Entity<TextareaState>,
    description_input: Entity<TextareaState>,
    subtask_input: Entity<InputState>,
    relation_input: Entity<InputState>,
    log_minutes_input: Entity<InputState>,
    log_note_input: Entity<InputState>,
    log_date_picker: Entity<DatePickerState>,
    /// カスタムフィールド（テキスト系）ごとの入力欄。描画時に作る。
    field_inputs: HashMap<Uuid, Entity<InputState>>,
    editing_description: bool,
    /// 値の無い副次プロパティ（最終期限・見積など）も並べる。
    show_all_props: bool,
    description_view: MarkdownBlocks,
    posting_comment: bool,
    clear_comment_input: bool,
    clear_subtask_input: bool,
    clear_relation_input: bool,
    clear_log_inputs: bool,
    _subs: Vec<Subscription>,
    /// 入力欄をどの task まで同期したか。
    title_synced: Option<Uuid>,
    /// project_id → key。Detail の見出しを一覧と同じ `KEY-12` 表記にする。
    project_keys: HashMap<Uuid, String>,
    // ---- プロジェクトの候補（project が変わるまで使い回す） ----
    catalog_project: Option<Uuid>,
    labels: Vec<LabelResponse>,
    milestones: Vec<MilestoneResponse>,
    sprints: Vec<SprintResponse>,
    custom_fields: Vec<ProjectCustomFieldResponse>,
    // ---- 区画 ----
    me: Option<Uuid>,
    tab: Tab,
    watchers: Vec<WatcherUser>,
    relations: Option<TaskRelationsResponse>,
    relations_failed: bool,
    relation_kind: RelationKind,
    relation_hits: Vec<SearchTaskHit>,
    relation_search: u64,
    activities: Vec<ActivityItem>,
    activity_cursor: Option<String>,
    activity_state: Load,
    timer: Option<TimerStatusResponse>,
    time_summary: Option<TimeLogSummaryResponse>,
    time_logs: Vec<TimeLogResponse>,
    time_state: Load,
    attachments: Vec<TaskAttachmentResponse>,
    files_state: Load,
    editing_comment: Option<Uuid>,
    reply_to: Option<(Uuid, String)>,
    /// タイマー作動中の経過表示を進める。
    ticker: Option<Task<()>>,
}

fn text_input(
    window: &mut Window,
    cx: &mut Context<TaskDetailView>,
    hint: &str,
) -> Entity<InputState> {
    let hint = hint.to_string();
    cx.new(|cx| InputState::new(window, cx).placeholder(hint))
}

fn date_picker(window: &mut Window, cx: &mut Context<TaskDetailView>) -> Entity<DatePickerState> {
    cx.new(|cx| DatePickerState::new(window, cx).date_format("%Y-%m-%d"))
}

fn on_enter(
    cx: &mut Context<TaskDetailView>,
    input: &Entity<InputState>,
    f: impl Fn(&mut TaskDetailView, &mut Context<TaskDetailView>) + 'static,
) -> Subscription {
    cx.subscribe(input, move |this, _, event: &InputEvent, cx| {
        if matches!(event, InputEvent::PressEnter { .. }) {
            f(this, cx);
        }
    })
}

impl TaskDetailView {
    pub fn new(
        client: Option<Client>,
        tenant: Option<Uuid>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let title_input = text_input(window, cx, t!("tasks.detail.title_placeholder"));
        let due_picker = date_picker(window, cx);
        let hard_due_picker = date_picker(window, cx);
        let log_date_picker = date_picker(window, cx);
        let estimate_input = text_input(window, cx, t!("tasks.detail.estimate_placeholder"));
        let progress_input = text_input(window, cx, "0–100");
        let subtask_input = text_input(window, cx, t!("tasks.detail.subtask_placeholder"));
        let relation_input = text_input(window, cx, t!("tasks.detail.relation_search"));
        let log_minutes_input = text_input(window, cx, t!("tasks.detail.log_minutes_placeholder"));
        let log_note_input = text_input(window, cx, t!("tasks.detail.log_note_placeholder"));
        let comment_input = cx.new(|cx| {
            TextareaState::new(window, cx).placeholder(t!("tasks.detail.comment_placeholder"))
        });
        let edit_input = cx.new(|cx| TextareaState::new(window, cx));
        let subs = vec![
            on_enter(cx, &title_input, |this, cx| this.save_title(cx)),
            // 保存ボタンは変更があるときだけ出すので、入力のたびに描き直す。
            cx.subscribe(&title_input, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
            on_enter(cx, &estimate_input, |this, cx| this.save_estimate(cx)),
            on_enter(cx, &progress_input, |this, cx| this.save_progress(cx)),
            on_enter(cx, &subtask_input, |this, cx| this.create_subtask(cx)),
            on_enter(cx, &log_minutes_input, |this, cx| this.add_time_log(cx)),
            on_enter(cx, &log_note_input, |this, cx| this.add_time_log(cx)),
            cx.subscribe(&relation_input, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.search_relations(cx);
                }
            }),
            cx.subscribe(&due_picker, |this, _, event: &DatePickerEvent, cx| {
                if let DatePickerEvent::Change(Date::Single(date)) = event {
                    this.apply_due(*date, false, cx);
                }
            }),
            cx.subscribe(&hard_due_picker, |this, _, event: &DatePickerEvent, cx| {
                if let DatePickerEvent::Change(Date::Single(date)) = event {
                    this.apply_due(*date, true, cx);
                }
            }),
            // Ctrl/Cmd+Enter で投稿（Enter は改行）。
            cx.subscribe(&comment_input, |this, _, event: &InputEvent, cx| {
                if matches!(
                    event,
                    InputEvent::PressEnter {
                        secondary: true,
                        ..
                    }
                ) {
                    this.post_comment(cx);
                }
            }),
            cx.subscribe(&edit_input, |this, _, event: &InputEvent, cx| {
                if matches!(
                    event,
                    InputEvent::PressEnter {
                        secondary: true,
                        ..
                    }
                ) {
                    this.save_comment_edit(cx);
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
            acting: false,
            generation: 0,
            client_generation: 0,
            error: None,
            shown_error: None,
            notice: None,
            title_input,
            due_picker,
            hard_due_picker,
            estimate_input,
            progress_input,
            comment_input,
            edit_input,
            description_input: cx.new(|cx| {
                TextareaState::new(window, cx)
                    .placeholder(t!("tasks.detail.description_placeholder"))
            }),
            subtask_input,
            relation_input,
            log_minutes_input,
            log_note_input,
            log_date_picker,
            field_inputs: HashMap::new(),
            editing_description: false,
            show_all_props: false,
            description_view: MarkdownBlocks::new(),
            posting_comment: false,
            clear_comment_input: false,
            clear_subtask_input: false,
            clear_relation_input: false,
            clear_log_inputs: true,
            _subs: subs,
            title_synced: None,
            project_keys: HashMap::new(),
            catalog_project: None,
            labels: vec![],
            milestones: vec![],
            sprints: vec![],
            custom_fields: vec![],
            me: None,
            tab: Tab::Comments,
            watchers: vec![],
            relations: None,
            relations_failed: false,
            relation_kind: RelationKind::BlockedBy,
            relation_hits: vec![],
            relation_search: 0,
            activities: vec![],
            activity_cursor: None,
            activity_state: Load::Idle,
            timer: None,
            time_summary: None,
            time_logs: vec![],
            time_state: Load::Idle,
            attachments: vec![],
            files_state: Load::Idle,
            editing_comment: None,
            reply_to: None,
            ticker: None,
        }
    }

    /// プロジェクト設定の変更後、次に開くときにラベル・計画・カスタムフィールドを取り直す。
    pub fn invalidate_catalog(&mut self) {
        self.catalog_project = None;
    }

    pub fn set_project_keys(&mut self, keys: impl IntoIterator<Item = (Uuid, String)>) {
        self.project_keys = keys.into_iter().collect();
    }

    /// 一覧からの選択。各種ロードを投げる。同じタスクの開き直しはタブを保つ。
    pub fn open(&mut self, project: Uuid, task: Uuid, cx: &mut Context<Self>) {
        let same = self.task == Some(task) && self.project == Some(project);
        // 同じプロジェクトのステータス・担当候補は取り直すまで前の値で描く（ちらつかせない）。
        if self.project != Some(project) {
            self.statuses.clear();
            self.assignables.clear();
        }
        self.generation += 1;
        self.updating = false;
        self.acting = false;
        self.project = Some(project);
        self.task = Some(task);
        self.detail = None;
        self.comments = vec![];
        self.error = None;
        self.title_synced = None;
        self.clear_comment_input = true;
        self.clear_subtask_input = true;
        self.clear_relation_input = true;
        self.clear_log_inputs = true;
        self.editing_description = false;
        self.posting_comment = false;
        self.clear_sections();
        if !same {
            self.tab = Tab::Comments;
        }
        if self.catalog_project != Some(project) {
            self.catalog_project = None;
            self.labels.clear();
            self.milestones.clear();
            self.sprints.clear();
            self.custom_fields.clear();
        }
        if self.client.is_none() || self.tenant.is_none() {
            return;
        }
        self.loading = true;
        cx.notify();
        // 本体が届いた時点で描き、他は並行に取って届いたものから埋める（順に待つと遅い 1 本に引きずられる）。
        self.run(
            |c, t, p, k| async move { c.get_task(t, p, k).await },
            |this, res, _| {
                this.loading = false;
                match res {
                    Ok(detail) => this.detail = Some(detail),
                    Err(api::ApiError::NotFound) => {
                        this.error = Some(t!("tasks.detail.not_found").into())
                    }
                    Err(e) => this.error = Some(e.to_string()),
                }
            },
            cx,
        );
        self.run(
            |c, t, p, _| async move { c.list_statuses(t, p).await },
            |this, res, _| match res {
                Ok(s) => this.statuses = s,
                Err(e) if this.error.is_none() => {
                    this.error = Some(t!("tasks.detail.statuses_error", error = e))
                }
                Err(_) => {}
            },
            cx,
        );
        self.run(
            |c, t, p, _| async move { c.list_assignable_users(t, p, None).await },
            |this, res, _| match res {
                Ok(users) => this.assignables = users,
                Err(e) if this.error.is_none() => {
                    this.error = Some(t!("tasks.detail.assignees_error", error = e))
                }
                Err(_) => {}
            },
            cx,
        );
        self.run(
            |c, t, p, k| async move { c.list_comments(t, p, k).await },
            |this, res, _| match res {
                Ok(comments) => this.comments = comments.comments,
                Err(e) if this.error.is_none() => {
                    this.error = Some(t!("tasks.detail.comments_error", error = e))
                }
                Err(_) => {}
            },
            cx,
        );
        self.load_aux(cx);
        self.load_tab(self.tab, cx);
    }

    fn clear_sections(&mut self) {
        self.watchers.clear();
        self.relations = None;
        self.relations_failed = false;
        self.relation_hits.clear();
        self.relation_search += 1;
        self.activities.clear();
        self.activity_cursor = None;
        self.activity_state = Load::Idle;
        self.timer = None;
        self.time_summary = None;
        self.time_logs.clear();
        self.time_state = Load::Idle;
        self.attachments.clear();
        self.files_state = Load::Idle;
        self.editing_comment = None;
        self.reply_to = None;
        self.ticker = None;
    }

    pub fn set_client(&mut self, client: Client, tenant: Option<Uuid>) {
        if !self
            .client
            .as_ref()
            .is_some_and(|current| current.same_session(&client))
        {
            self.client_generation += 1;
            self.me = None;
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
        self.acting = false;
        self.client = None;
        self.me = None;
        self.detail = None;
        self.comments.clear();
        self.statuses.clear();
        self.assignables.clear();
        self.catalog_project = None;
        self.clear_sections();
        self.project = None;
        self.task = None;
    }

    /// 削除・アーカイブ後に空の Detail へ戻す。
    fn clear_task(&mut self) {
        self.generation += 1;
        self.detail = None;
        self.task = None;
        self.comments.clear();
        self.clear_sections();
        self.updating = false;
        self.acting = false;
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

    fn is_watching(&self) -> bool {
        self.me
            .is_some_and(|me| self.watchers.iter().any(|w| w.id == me))
    }

    fn task_key(&self, seq_id: i32) -> String {
        match self.project.and_then(|p| self.project_keys.get(&p)) {
            Some(key) => format!("{key}-{seq_id}"),
            None => format!("#{seq_id}"),
        }
    }

    fn fail(&mut self, error: api::ApiError) {
        self.error = Some(error.to_string());
    }

    // ---- 非同期の共通経路 ----

    /// 表示中のタスクに対する 1 回のリクエスト。開き直した後の結果は捨てる。
    fn run<T, Fut>(
        &mut self,
        request: impl FnOnce(Client, Uuid, Uuid, Uuid) -> Fut,
        done: impl FnOnce(&mut Self, api::Result<T>, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) where
        T: 'static,
        Fut: Future<Output = api::Result<T>> + 'static,
    {
        let (Some(client), Some(tenant), Some(project), Some(task)) =
            (self.client.clone(), self.tenant, self.project, self.task)
        else {
            return;
        };
        let generation = self.generation;
        let fut = request(client, tenant, project, task);
        cx.spawn(async move |this, cx| {
            let res = fut.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation || this.client.is_none() {
                    return;
                }
                done(this, res, cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// 利用者の操作（`acting` の間はボタンを止める）。失敗は Toast。
    fn act<T, Fut>(
        &mut self,
        request: impl FnOnce(Client, Uuid, Uuid, Uuid) -> Fut,
        done: impl FnOnce(&mut Self, T, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) where
        T: 'static,
        Fut: Future<Output = api::Result<T>> + 'static,
    {
        if self.acting {
            return;
        }
        self.acting = true;
        self.error = None;
        cx.notify();
        self.run(
            request,
            move |this, res, cx| {
                this.acting = false;
                match res {
                    Ok(value) => done(this, value, cx),
                    Err(e) => this.fail(e),
                }
            },
            cx,
        );
    }

    /// 取り消せない操作の確認（OK で `action`）。
    fn confirm(
        &self,
        title: &'static str,
        description: String,
        ok: &'static str,
        action: impl Fn(&mut Self, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let owner = cx.entity().downgrade();
        let action = Rc::new(action);
        window.open_alert_dialog(cx, move |alert, _, _| {
            let (owner, action) = (owner.clone(), action.clone());
            alert
                .title(title)
                .description(description.clone())
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(ok)
                        .ok_variant(ButtonVariant::Danger)
                        .cancel_text(t!("tasks.detail.cancel"))
                        .show_cancel(true),
                )
                .on_ok(move |_, _, cx| {
                    let _ = owner.update(cx, |this, cx| action(this, cx));
                    true
                })
        });
    }

    /// 候補（ラベル等）・自分・ウォッチ・依存関係・タイマー。
    fn load_aux(&mut self, cx: &mut Context<Self>) {
        if self.catalog_project.is_none() {
            self.run(
                |c, t, p, _| async move {
                    Ok((
                        c.list_labels(t, p).await,
                        c.list_milestones(t, p).await,
                        c.list_sprints(t, p, None).await,
                        c.list_custom_fields(t, p).await,
                        p,
                    ))
                },
                |this, res: api::Result<_>, _| {
                    let Ok((labels, milestones, sprints, fields, project)) = res else {
                        return;
                    };
                    this.catalog_project = Some(project);
                    // 権限や未提供で取れない候補は空のまま（その項目の選択肢が出ないだけ）。
                    this.labels = labels.unwrap_or_default();
                    this.milestones = milestones.unwrap_or_default();
                    this.sprints = sprints.unwrap_or_default();
                    this.custom_fields = fields.map(|f| f.fields).unwrap_or_default();
                },
                cx,
            );
        }
        if self.me.is_none() {
            self.run(
                |c, _, _, _| async move { c.get_me().await },
                |this, res, _| {
                    if let Ok(me) = res {
                        this.me = Some(me.id);
                    }
                },
                cx,
            );
        }
        self.run(
            |c, t, p, k| async move { c.list_watchers(t, p, k).await },
            |this, res, _| {
                if let Ok(w) = res {
                    this.watchers = w.watchers;
                }
            },
            cx,
        );
        self.reload_relations(cx);
        self.run(
            |c, t, p, k| async move { c.get_timer_status(t, p, k).await },
            |this, res, cx| {
                if let Ok(timer) = res {
                    this.set_timer(timer, cx);
                }
            },
            cx,
        );
    }

    fn reload_relations(&mut self, cx: &mut Context<Self>) {
        self.relations_failed = false;
        self.run(
            |c, t, p, k| async move { c.list_relations(t, p, k).await },
            |this, res, _| match res {
                Ok(r) => this.relations = Some(r),
                Err(e) => {
                    this.relations_failed = true;
                    this.fail(e);
                }
            },
            cx,
        );
    }

    fn select_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        self.tab = tab;
        self.load_tab(tab, cx);
        cx.notify();
    }

    /// 開いたタブの中身を初回だけ取る。
    fn load_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        match tab {
            Tab::Activity if self.activity_state == Load::Idle => self.load_activity(false, cx),
            Tab::Time if self.time_state == Load::Idle => self.reload_time(cx),
            Tab::Files if self.files_state == Load::Idle => self.reload_files(cx),
            _ => {}
        }
    }

    fn load_activity(&mut self, more: bool, cx: &mut Context<Self>) {
        if self.activity_state == Load::Loading {
            return;
        }
        let cursor = if more {
            self.activity_cursor.clone()
        } else {
            None
        };
        if more && cursor.is_none() {
            return;
        }
        self.activity_state = Load::Loading;
        self.run(
            move |c, t, p, k| async move { c.list_activities(t, p, k, cursor).await },
            move |this, res, _| match res {
                Ok(page) => {
                    if !more {
                        this.activities.clear();
                    }
                    this.activities.extend(page.activities);
                    this.activity_cursor = page.next_cursor;
                    this.activity_state = Load::Loaded;
                }
                Err(e) => {
                    this.activity_state = Load::Failed;
                    this.fail(e);
                }
            },
            cx,
        );
    }

    fn reload_time(&mut self, cx: &mut Context<Self>) {
        self.time_state = Load::Loading;
        self.run(
            |c, t, p, k| async move {
                let summary = c.get_time_summary(t, p, k).await?;
                let logs = c.list_time_logs(t, p, k).await?;
                let timer = c.get_timer_status(t, p, k).await?;
                Ok((summary, logs, timer))
            },
            |this, res, cx| match res {
                Ok((summary, logs, timer)) => {
                    this.time_summary = Some(summary);
                    this.time_logs = logs;
                    this.time_state = Load::Loaded;
                    this.set_timer(timer, cx);
                }
                Err(e) => {
                    this.time_state = Load::Failed;
                    this.fail(e);
                }
            },
            cx,
        );
    }

    fn reload_files(&mut self, cx: &mut Context<Self>) {
        self.files_state = Load::Loading;
        self.run(
            |c, t, p, k| async move { c.list_task_attachments(t, p, k).await },
            |this, res, _| match res {
                Ok(list) => {
                    this.attachments = list.attachments;
                    this.files_state = Load::Loaded;
                }
                Err(e) => {
                    this.files_state = Load::Failed;
                    this.fail(e);
                }
            },
            cx,
        );
    }

    /// タイマーの状態を反映し、作動中は 30 秒ごとに経過表示を描き直す。
    fn set_timer(&mut self, timer: TimerStatusResponse, cx: &mut Context<Self>) {
        let running = timer.is_running;
        self.timer = Some(timer);
        if !running {
            self.ticker = None;
            return;
        }
        if self.ticker.is_some() {
            return;
        }
        self.ticker = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(30))
                    .await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        }));
    }

    // ---- タスク本体の更新 ----

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

    /// 値をそのまま差し替える更新（失敗で元へ戻す）。
    fn update_field<V: Clone + Send + 'static>(
        &mut self,
        get: fn(&mut TaskDetailResponse) -> &mut V,
        next: V,
        req: UpdateTaskRequest,
        cx: &mut Context<Self>,
    ) {
        let Some(prev) = self.detail.as_mut().map(|d| get(d).clone()) else {
            return;
        };
        self.update(move |d| *get(d) = next, move |d| *get(d) = prev, req, cx);
    }

    fn set_status(&mut self, status_id: Uuid, cx: &mut Context<Self>) {
        self.update_field(
            |d| &mut d.status_id,
            status_id,
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
        self.update_field(
            |d| &mut d.priority,
            priority,
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
        let mut next = d.assignees.clone();
        if let Some(pos) = next.iter().position(|a| a.user.id == user.id) {
            next.remove(pos);
        } else {
            next.push(TaskAssigneeSummary {
                role: "assignee".into(),
                user: user.clone(),
            });
        }
        let list = next
            .iter()
            .map(|a| AssigneeInput {
                role: a.role.clone(),
                user_id: a.user.id,
            })
            .collect();
        self.update_field(
            |d| &mut d.assignees,
            next,
            UpdateTaskRequest {
                assignees: Some(list),
                ..Default::default()
            },
            cx,
        );
    }

    fn toggle_label(&mut self, label: LabelResponse, cx: &mut Context<Self>) {
        let Some(d) = self.detail.as_ref() else {
            return;
        };
        let has = d.labels.iter().any(|l| l.id == label.id);
        let mut next = d.labels.clone();
        let req = if has {
            next.retain(|l| l.id != label.id);
            UpdateTaskRequest {
                remove_label_ids: vec![label.id],
                ..Default::default()
            }
        } else {
            let id = label.id;
            next.push(label);
            UpdateTaskRequest {
                add_label_ids: vec![id],
                ..Default::default()
            }
        };
        self.update_field(|d| &mut d.labels, next, req, cx);
    }

    fn set_milestone(&mut self, id: Option<Uuid>, cx: &mut Context<Self>) {
        let req = UpdateTaskRequest {
            milestone_id: id,
            clear_milestone_id: id.is_none().then_some(true),
            ..Default::default()
        };
        self.update_field(|d| &mut d.milestone_id, id, req, cx);
    }

    fn set_sprint(&mut self, id: Option<Uuid>, cx: &mut Context<Self>) {
        let req = UpdateTaskRequest {
            sprint_id: id,
            clear_sprint_id: id.is_none().then_some(true),
            ..Default::default()
        };
        self.update_field(|d| &mut d.sprint_id, id, req, cx);
    }

    fn clear_parent(&mut self, cx: &mut Context<Self>) {
        let req = UpdateTaskRequest {
            clear_parent_task_id: Some(true),
            ..Default::default()
        };
        self.update_field(|d| &mut d.parent_task_id, None, req, cx);
    }

    /// 期限（soft）/ ハード期限（hard）。`None` は外す。
    fn apply_due(&mut self, date: Option<NaiveDate>, hard: bool, cx: &mut Context<Self>) {
        let due = date.and_then(|d| due_timestamp(d, &core::timezone::current()));
        let current = self.detail.as_ref().and_then(|d| {
            if hard {
                d.hard_deadline
            } else {
                d.soft_deadline
            }
        });
        if current.map(|c| c.with_timezone(&core::timezone::current()).date_naive()) == date {
            return;
        }
        let req = match (due, hard) {
            (Some(dt), false) => UpdateTaskRequest {
                soft_deadline: Some(dt),
                ..Default::default()
            },
            (Some(dt), true) => UpdateTaskRequest {
                hard_deadline: Some(dt),
                ..Default::default()
            },
            (None, false) => UpdateTaskRequest {
                clear_soft_deadline: Some(true),
                ..Default::default()
            },
            (None, true) => UpdateTaskRequest {
                clear_hard_deadline: Some(true),
                ..Default::default()
            },
        };
        if hard {
            self.update_field(|d| &mut d.hard_deadline, due, req, cx);
        } else {
            self.update_field(|d| &mut d.soft_deadline, due, req, cx);
        }
    }

    fn save_title(&mut self, cx: &mut Context<Self>) {
        let title = self.title_input.read(cx).value().trim().to_string();
        if title.is_empty() || self.detail.as_ref().is_some_and(|d| d.title == title) {
            return;
        }
        self.update_field(
            |d| &mut d.title,
            title.clone(),
            UpdateTaskRequest {
                title: Some(title),
                ..Default::default()
            },
            cx,
        );
    }

    fn save_description(&mut self, cx: &mut Context<Self>) {
        let text = self.description_input.read(cx).value().to_string();
        self.editing_description = false;
        let req = if text.trim().is_empty() {
            UpdateTaskRequest {
                clear_description: Some(true),
                ..Default::default()
            }
        } else {
            UpdateTaskRequest {
                description: Some(text.clone()),
                ..Default::default()
            }
        };
        let next = (!text.trim().is_empty()).then_some(text);
        self.update_field(|d| &mut d.description, next, req, cx);
    }

    /// 見積（`90` / `1h30m`）。空は外す。
    fn save_estimate(&mut self, cx: &mut Context<Self>) {
        let raw = self.estimate_input.read(cx).value().trim().to_string();
        let next = if raw.is_empty() {
            None
        } else {
            match parse_minutes(&raw) {
                Some(m) => Some(m),
                None => {
                    self.error = Some(t!("tasks.detail.invalid_minutes").into());
                    cx.notify();
                    return;
                }
            }
        };
        if self.detail.as_ref().map(|d| d.estimated_minutes) == Some(next) {
            return;
        }
        let req = UpdateTaskRequest {
            estimated_minutes: next,
            clear_estimated_minutes: next.is_none().then_some(true),
            ..Default::default()
        };
        self.update_field(|d| &mut d.estimated_minutes, next, req, cx);
    }

    fn save_progress(&mut self, cx: &mut Context<Self>) {
        let raw = self.progress_input.read(cx).value().trim().to_string();
        let Some(pct) = raw
            .trim_end_matches('%')
            .parse::<i32>()
            .ok()
            .filter(|p| (0..=100).contains(p))
        else {
            self.error = Some(t!("tasks.detail.invalid_progress").into());
            cx.notify();
            return;
        };
        if self.detail.as_ref().map(|d| d.progress_pct) == Some(pct) {
            return;
        }
        self.update_field(
            |d| &mut d.progress_pct,
            pct,
            UpdateTaskRequest {
                progress_pct: Some(pct),
                ..Default::default()
            },
            cx,
        );
    }

    /// カスタムフィールド 1 つの値（`None` は外す）。形式は backend が検証する。
    fn set_custom_field(&mut self, field: Uuid, value: Option<String>, cx: &mut Context<Self>) {
        let Some(d) = self.detail.as_ref() else {
            return;
        };
        let value = value.filter(|v| !v.trim().is_empty());
        if d.custom_field_values
            .iter()
            .any(|v| v.field.id == field && v.value == value)
        {
            return;
        }
        if let Some(def) = self.custom_fields.iter().find(|f| f.id == field)
            && def.field_type == CustomFieldType::Date
            && let Some(v) = &value
            && NaiveDate::parse_from_str(v, "%Y-%m-%d").is_err()
        {
            self.error = Some(t!("tasks.detail.invalid_date").into());
            cx.notify();
            return;
        }
        let display = value.as_ref().map(|v| {
            self.custom_fields
                .iter()
                .find(|f| f.id == field)
                .and_then(|f| select_options(f).into_iter().find(|(val, _)| val == v))
                .map(|(_, label)| label)
                .unwrap_or_else(|| v.clone())
        });
        let mut next = d.custom_field_values.clone();
        if let Some(entry) = next.iter_mut().find(|v| v.field.id == field) {
            entry.value = value.clone();
            entry.display_value = display;
        }
        let req = UpdateTaskRequest {
            custom_field_values: Some(vec![CustomFieldValueInput {
                field_id: field,
                value,
            }]),
            ..Default::default()
        };
        self.update_field(|d| &mut d.custom_field_values, next, req, cx);
    }

    // ---- ヘッダーの操作 ----

    fn toggle_watch(&mut self, cx: &mut Context<Self>) {
        let watching = self.is_watching();
        self.act(
            move |c, t, p, k| async move {
                if watching {
                    c.stop_watch(t, p, k).await?;
                } else {
                    c.start_watch(t, p, k).await?;
                }
                c.list_watchers(t, p, k).await
            },
            move |this, list, _| {
                this.watchers = list.watchers;
                this.notice = Some(if watching {
                    t!("tasks.detail.unwatched").into()
                } else {
                    t!("tasks.detail.watching_now").into()
                });
            },
            cx,
        );
    }

    fn toggle_archive(&mut self, cx: &mut Context<Self>) {
        let Some(archived) = self.detail.as_ref().map(|d| d.is_archived) else {
            return;
        };
        self.act(
            move |c, t, p, k| async move {
                if archived {
                    c.unarchive_task(t, p, k).await?;
                } else {
                    c.archive_task(t, p, k).await?;
                }
                Ok((p, k))
            },
            move |this, (project, task), cx| {
                if archived {
                    if let Some(d) = this.detail.as_mut() {
                        d.is_archived = false;
                    }
                    this.notice = Some(t!("tasks.detail.unarchived").into());
                    cx.emit(TaskDetailEvent::Updated { project, task });
                } else {
                    this.clear_task();
                    this.notice = Some(t!("tasks.detail.archived_notice").into());
                    cx.emit(TaskDetailEvent::Removed { project, task });
                }
            },
            cx,
        );
    }

    fn delete_task(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(d) = self.detail.as_ref() else {
            return;
        };
        let name = format!("{} {}", self.task_key(d.seq_id), d.title);
        self.confirm(
            t!("tasks.detail.delete_title"),
            t!("tasks.detail.delete_confirm", name = name),
            t!("tasks.detail.delete"),
            |this, cx| {
                this.act(
                    |c, t, p, k| async move {
                        c.delete_task(t, p, k).await?;
                        Ok((p, k))
                    },
                    |this, (project, task), cx| {
                        this.clear_task();
                        this.notice = Some(t!("tasks.detail.deleted").into());
                        cx.emit(TaskDetailEvent::Removed { project, task });
                    },
                    cx,
                );
            },
            window,
            cx,
        );
    }

    fn open_task(&mut self, task: Uuid, cx: &mut Context<Self>) {
        if let Some(project) = self.project {
            cx.emit(TaskDetailEvent::OpenTask { project, task });
        }
    }

    // ---- サブタスク・依存関係 ----

    fn create_subtask(&mut self, cx: &mut Context<Self>) {
        let title = self.subtask_input.read(cx).value().trim().to_string();
        if title.is_empty() {
            return;
        }
        let Some(status) = self
            .statuses
            .iter()
            .find(|s| s.is_default)
            .or_else(|| self.statuses.first())
            .map(|s| s.id)
        else {
            self.error = Some(t!("tasks.list.status_error").into());
            cx.notify();
            return;
        };
        self.act(
            move |c, t, p, k| async move {
                c.create_task(t, p, &create_request(title, status, Some(k)))
                    .await?;
                Ok((c.list_relations(t, p, k).await?, p, k))
            },
            |this, (relations, project, task), cx| {
                this.relations = Some(relations);
                this.clear_subtask_input = true;
                cx.emit(TaskDetailEvent::Updated { project, task });
            },
            cx,
        );
    }

    /// 依存関係の候補を全文検索で探す（入力が止まってから）。
    fn search_relations(&mut self, cx: &mut Context<Self>) {
        self.relation_search += 1;
        let generation = self.relation_search;
        let query = self.relation_input.read(cx).value().trim().to_string();
        if query.is_empty() {
            self.relation_hits.clear();
            cx.notify();
            return;
        }
        let (Some(client), Some(tenant), Some(project), Some(task)) =
            (self.client.clone(), self.tenant, self.project, self.task)
        else {
            return;
        };
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(250))
                .await;
            if !this
                .update(cx, |this, _| this.relation_search == generation)
                .unwrap_or(false)
            {
                return;
            }
            let res = client.search_tasks(tenant, project, &query).await;
            let _ = this.update(cx, |this, cx| {
                if this.relation_search != generation || this.task != Some(task) {
                    return;
                }
                match res {
                    Ok(found) => {
                        this.relation_hits =
                            found.tasks.into_iter().filter(|h| h.id != task).collect()
                    }
                    Err(e) => this.fail(e),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn add_relation(&mut self, target: Uuid, cx: &mut Context<Self>) {
        let kind = self.relation_kind.api();
        self.act(
            move |c, t, p, k| async move {
                c.add_relation(t, p, k, target, kind).await?;
                c.list_relations(t, p, k).await
            },
            |this, relations, _| {
                this.relations = Some(relations);
                this.relation_hits.clear();
                this.clear_relation_input = true;
            },
            cx,
        );
    }

    fn remove_relation(&mut self, relation: Uuid, cx: &mut Context<Self>) {
        self.act(
            move |c, t, p, k| async move {
                c.remove_relation(t, p, k, relation).await?;
                c.list_relations(t, p, k).await
            },
            |this, relations, _| this.relations = Some(relations),
            cx,
        );
    }

    // ---- コメント ----

    fn post_comment(&mut self, cx: &mut Context<Self>) {
        if self.posting_comment {
            return;
        }
        let body = self.comment_input.read(cx).value().trim().to_string();
        if body.is_empty() {
            return;
        }
        let parent = self.reply_to.as_ref().map(|(id, _)| *id);
        self.posting_comment = true;
        self.run(
            move |c, t, p, k| async move {
                c.create_comment(
                    t,
                    p,
                    k,
                    &CreateCommentRequest {
                        body,
                        parent_comment_id: parent,
                    },
                )
                .await?;
                c.list_comments(t, p, k).await
            },
            |this, res, _| {
                this.posting_comment = false;
                match res {
                    Ok(list) => {
                        this.comments = list.comments;
                        this.clear_comment_input = true;
                        this.reply_to = None;
                    }
                    // 失敗したら下書きを残す。
                    Err(e) => this.fail(e),
                }
            },
            cx,
        );
        cx.notify();
    }

    fn start_reply(&mut self, id: Uuid, name: String, window: &mut Window, cx: &mut Context<Self>) {
        self.reply_to = Some((id, name));
        self.comment_input
            .update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    fn start_comment_edit(
        &mut self,
        id: Uuid,
        body: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.editing_comment = Some(id);
        self.edit_input.update(cx, |input, cx| {
            input.set_value(body, window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    fn save_comment_edit(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.editing_comment else {
            return;
        };
        let body = self.edit_input.read(cx).value().trim().to_string();
        if body.is_empty() {
            return;
        }
        self.act(
            move |c, t, p, k| async move {
                c.update_comment(t, p, k, id, &body).await?;
                c.list_comments(t, p, k).await
            },
            |this, list, _| {
                this.comments = list.comments;
                this.editing_comment = None;
            },
            cx,
        );
    }

    fn delete_comment(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm(
            t!("tasks.detail.delete_comment_title"),
            t!("tasks.detail.delete_comment_confirm").into(),
            t!("tasks.detail.delete"),
            move |this, cx| {
                this.act(
                    move |c, t, p, k| async move {
                        c.delete_comment(t, p, k, id).await?;
                        c.list_comments(t, p, k).await
                    },
                    |this, list, _| this.comments = list.comments,
                    cx,
                );
            },
            window,
            cx,
        );
    }

    // ---- 作業時間 ----

    fn toggle_timer(&mut self, cx: &mut Context<Self>) {
        let running = self.timer.as_ref().is_some_and(|t| t.is_running);
        self.act(
            move |c, t, p, k| async move {
                if running {
                    c.stop_timer(t, p, k).await?;
                } else {
                    c.start_timer(t, p, k).await?;
                }
                Ok(())
            },
            move |this, (), cx| {
                this.notice = Some(if running {
                    t!("tasks.detail.timer_stopped").into()
                } else {
                    t!("tasks.detail.timer_started").into()
                });
                this.reload_time(cx);
            },
            cx,
        );
    }

    fn add_time_log(&mut self, cx: &mut Context<Self>) {
        let raw = self.log_minutes_input.read(cx).value().trim().to_string();
        if raw.is_empty() {
            return;
        }
        let Some(minutes) = parse_minutes(&raw) else {
            self.error = Some(t!("tasks.detail.invalid_minutes").into());
            cx.notify();
            return;
        };
        let note = self.log_note_input.read(cx).value().trim().to_string();
        let logged_at = match self.log_date_picker.read(cx).date() {
            Date::Single(Some(date)) => date,
            _ => core::timezone::today(),
        };
        let req = CreateTimeLogRequest {
            logged_at,
            logged_minutes: minutes,
            note: (!note.is_empty()).then_some(note),
        };
        self.act(
            move |c, t, p, k| async move { c.create_time_log(t, p, k, &req).await },
            |this, _, cx| {
                this.clear_log_inputs = true;
                this.reload_time(cx);
            },
            cx,
        );
    }

    fn delete_time_log(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm(
            t!("tasks.detail.delete_log_title"),
            t!("tasks.detail.delete_log_confirm").into(),
            t!("tasks.detail.delete"),
            move |this, cx| {
                this.act(
                    move |c, t, p, k| async move { c.delete_time_log(t, p, k, id).await },
                    |this, (), cx| this.reload_time(cx),
                    cx,
                );
            },
            window,
            cx,
        );
    }

    // ---- 添付 ----

    fn detach_file(&mut self, id: Uuid, name: String, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm(
            t!("tasks.detail.detach_title"),
            t!("tasks.detail.detach_confirm", name = name),
            t!("tasks.detail.detach"),
            move |this, cx| {
                this.act(
                    move |c, t, p, k| async move { c.detach_task_file(t, p, k, id).await },
                    |this, (), cx| this.reload_files(cx),
                    cx,
                );
            },
            window,
            cx,
        );
    }
}

impl EventEmitter<TaskDetailEvent> for TaskDetailView {}

/// select 型の選択肢 `[{label, value}]`（backend の形）。
fn select_options(field: &ProjectCustomFieldResponse) -> Vec<(String, String)> {
    field
        .options
        .as_ref()
        .and_then(|o| o.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|o| {
                    let value = o.get("value")?.as_str()?.to_string();
                    let label = o
                        .get("label")
                        .and_then(|l| l.as_str())
                        .unwrap_or(&value)
                        .to_string();
                    Some((value, label))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `current` の次のステータス（position 順）。最後か見つからなければ None。
fn next_status(statuses: &[ProjectStatusResponse], current: Uuid) -> Option<ProjectStatusResponse> {
    let mut ordered: Vec<&ProjectStatusResponse> = statuses.iter().collect();
    ordered.sort_by_key(|s| s.position);
    let ix = ordered.iter().position(|s| s.id == current)?;
    ordered.get(ix + 1).map(|s| (*s).clone())
}

#[cfg(test)]
mod tests {
    // `super::*` だと gpui の `#[test]` マクロが std のものを隠すので個別に import する。
    use super::{next_status, select_options};
    use api::types::{ProjectCustomFieldResponse, ProjectStatusResponse};
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

    #[test]
    fn select_options_reads_label_value_pairs() {
        let field: ProjectCustomFieldResponse = serde_json::from_value(serde_json::json!({
            "id": Uuid::nil(), "project_id": Uuid::nil(), "name": "Env", "field_type": "select",
            "is_required": false, "position": 0, "created_at": "2026-01-01T00:00:00Z",
            "options": [{"label": "本番", "value": "prod"}, {"value": "dev"}, {"label": "x"}],
        }))
        .unwrap();
        assert_eq!(
            select_options(&field),
            vec![
                ("prod".to_string(), "本番".to_string()),
                ("dev".to_string(), "dev".to_string())
            ]
        );
    }
}
