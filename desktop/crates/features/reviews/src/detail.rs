//! Review rounds, findings and server-provided actions and gate.
use api::spec::{Finding, Gate, ReviewSummary};
use api::types::{FindingSeverity, FindingState, ReviewResponse};
use api::{Client, FindingsQuery};
use gpui_kit::assets::IconName;
use gpui_kit::component::IndexPath;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::list::{List, ListDelegate, ListEvent, ListItem, ListState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::text::TextView;
use gpui_kit::component::{Disableable, Icon, Selectable, Sizable, WindowExt};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use i18n::t;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub enum ReviewDetailEvent {
    OpenTask { project: Uuid, task: Uuid },
    Updated { project: Uuid },
}

struct FindingRows {
    rows: Vec<Finding>,
    loading: bool,
}
impl ListDelegate for FindingRows {
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
    fn render_empty(
        &mut self,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> impl IntoElement {
        ::ui::empty_state(
            &::ui::colors(cx),
            IconName::CircleCheck,
            t!("reviews.detail.no_findings"),
            None,
        )
        .py_4()
    }
    fn render_item(
        &mut self,
        ix: IndexPath,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<ListItem> {
        let finding = self.rows.get(ix.row)?;
        let theme = ::ui::colors(cx);
        let muted = theme.text_muted;
        let severity_color = match finding.severity {
            FindingSeverity::High => theme.danger,
            FindingSeverity::Medium => theme.warning,
            FindingSeverity::Low | FindingSeverity::Nit => muted,
        };
        let resolved = matches!(
            finding.state,
            FindingState::Fixed | FindingState::Verified | FindingState::Rejected
        );
        Some(
            ListItem::new(("finding", ix.row))
                .h(px(52.))
                .w_full()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .min_w_0()
                        .flex_1()
                        .child(
                            div()
                                .text_sm()
                                .truncate()
                                .when(resolved, |d| d.text_color(muted))
                                .child(finding.title.clone()),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_3()
                                .text_xs()
                                .text_color(muted)
                                .child(format!("R{}", finding.round))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_1()
                                        .text_color(severity_color)
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(div().size(px(7.)).rounded_full().bg(severity_color))
                                        .child(severity_label(finding.severity)),
                                )
                                .child(state_label(finding.state)),
                        ),
                ),
        )
    }
}

pub struct ReviewDetailView {
    client: Option<Client>,
    tenant: Option<Uuid>,
    project: Option<Uuid>,
    pr: Option<i64>,
    pr_title: Option<String>,
    summary: Option<ReviewSummary>,
    reviews: Vec<ReviewResponse>,
    findings: Vec<Finding>,
    selected_round: Option<Uuid>,
    selected_finding: Option<Uuid>,
    note_input: Option<Entity<InputState>>,
    finding_list: Option<Entity<ListState<FindingRows>>>,
    list_subscription: Option<Subscription>,
    clear_note: bool,
    loading: bool,
    applying: bool,
    error: Option<String>,
    shown_error: Option<String>,
    generation: u64,
    client_generation: u64,
    /// 指摘の絞り込み（API の `state` / `severity` に渡す）。
    state_filter: Option<FindingState>,
    severity_filter: Option<FindingSeverity>,
}

/// 一覧の並び: 重大度の高い順、同じなら未解決を先に。
const SEVERITIES: [FindingSeverity; 4] = [
    FindingSeverity::High,
    FindingSeverity::Medium,
    FindingSeverity::Low,
    FindingSeverity::Nit,
];
const STATES: [FindingState; 6] = [
    FindingState::Open,
    FindingState::Fixing,
    FindingState::Fixed,
    FindingState::Deferred,
    FindingState::Rejected,
    FindingState::Verified,
];

fn finding_order(f: &Finding) -> (usize, usize) {
    (
        SEVERITIES
            .iter()
            .position(|s| *s == f.severity)
            .unwrap_or(9),
        STATES.iter().position(|s| *s == f.state).unwrap_or(9),
    )
}

impl ReviewDetailView {
    pub fn new(client: Option<Client>, tenant: Option<Uuid>) -> Self {
        Self {
            client,
            tenant,
            project: None,
            pr: None,
            pr_title: None,
            summary: None,
            reviews: vec![],
            findings: vec![],
            selected_round: None,
            selected_finding: None,
            note_input: None,
            finding_list: None,
            list_subscription: None,
            clear_note: false,
            loading: false,
            applying: false,
            error: None,
            shown_error: None,
            generation: 0,
            client_generation: 0,
            state_filter: None,
            severity_filter: None,
        }
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
            self.project = None;
            self.selected_finding = None;
            self.selected_round = None;
            self.error = None;
            self.loading = false;
            self.applying = false;
        }
        self.client = Some(client);
        self.tenant = tenant;
    }
    pub fn clear_client(&mut self) {
        self.client_generation += 1;
        self.client = None;
        self.generation += 1;
        self.pr = None;
        self.summary = None;
        self.findings.clear();
        self.reviews.clear();
    }
    pub fn set_project(&mut self, project: Uuid) {
        self.clear_note = true;
        self.project = Some(project);
        self.generation += 1;
        self.pr = None;
        self.summary = None;
        self.findings.clear();
        self.reviews.clear();
        self.selected_finding = None;
        self.selected_round = None;
        self.error = None;
        self.applying = false;
    }
    pub fn open(&mut self, pr: i64, pr_title: Option<String>, cx: &mut Context<Self>) {
        self.clear_note = true;
        self.state_filter = None;
        self.severity_filter = None;
        self.pr = Some(pr);
        self.pr_title = pr_title;
        self.summary = None;
        self.findings.clear();
        self.reviews.clear();
        self.selected_finding = None;
        self.selected_round = None;
        self.error = None;
        self.applying = false;
        self.load(cx);
    }

    /// Resolve a notification's immutable review ID before fetching its PR.
    pub fn open_target(&mut self, review: Uuid, finding: Option<Uuid>, cx: &mut Context<Self>) {
        let (Some(client), Some(tenant), Some(project)) =
            (self.client.clone(), self.tenant, self.project)
        else {
            return;
        };
        self.generation += 1;
        let generation = self.generation;
        self.loading = true;
        self.applying = false;
        self.clear_note = true;
        self.state_filter = None;
        self.severity_filter = None;
        self.error = None;
        self.pr = None;
        self.summary = None;
        self.findings.clear();
        self.reviews.clear();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = client.get_review(tenant, project, review).await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                match result {
                    Ok(r) => {
                        this.pr = Some(r.pr_number as i64);
                        this.pr_title = r.pr_title;
                        this.selected_round = Some(review);
                        this.selected_finding = finding;
                        this.load(cx);
                    }
                    Err(api::ApiError::NotFound) => {
                        this.loading = false;
                        this.error = Some(t!("reviews.detail.review_missing").into());
                    }
                    Err(e) => {
                        this.loading = false;
                        this.error = Some(e.to_string());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        let (Some(client), Some(tenant), Some(project), Some(pr)) =
            (self.client.clone(), self.tenant, self.project, self.pr)
        else {
            return;
        };
        self.loading = true;
        self.generation += 1;
        let generation = self.generation;
        let (state_filter, severity_filter) = (self.state_filter, self.severity_filter);
        cx.notify();
        cx.spawn(async move |this, cx| {
            let summary = client.get_review_summary(tenant, project, pr, None).await;
            let reviews = client.list_reviews(tenant, project, Some(pr), None).await;
            let findings = client
                .list_review_findings(
                    tenant,
                    project,
                    &FindingsQuery {
                        pr: Some(pr),
                        state: state_filter,
                        severity: severity_filter,
                        ..Default::default()
                    },
                )
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.loading = false;
                match summary {
                    Ok(s) => this.summary = Some(s),
                    Err(e) => this.error = Some(e.to_string()),
                }
                match reviews {
                    Ok(r) => this.reviews = r,
                    Err(e) => this.error = Some(e.to_string()),
                }
                match findings {
                    Ok(f) => this.findings = f,
                    Err(e) => this.error = Some(e.to_string()),
                }
                this.findings.sort_by_key(finding_order);
                let filtered = this.state_filter.is_some() || this.severity_filter.is_some();
                if let Some(id) = this.selected_finding
                    && !this.findings.iter().any(|f| f.id == id)
                    && !filtered
                {
                    this.selected_finding = None;
                    this.error = Some(t!("reviews.detail.finding_missing").into());
                }
                if this
                    .selected_finding
                    .is_none_or(|id| !this.findings.iter().any(|f| f.id == id))
                {
                    this.selected_finding = this.visible_findings().first().map(|f| f.id);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn visible_findings(&self) -> Vec<&Finding> {
        self.findings
            .iter()
            .filter(|f| self.selected_round.is_none_or(|r| f.review_id == r))
            .collect()
    }
    pub fn available_actions(&self) -> Vec<FindingState> {
        if self.loading || self.applying {
            return vec![];
        }
        self.findings
            .iter()
            .find(|f| Some(f.id) == self.selected_finding)
            .map(|f| f.available_actions.clone())
            .unwrap_or_default()
    }
    pub fn apply_selected_action(&mut self, state: FindingState, cx: &mut Context<Self>) {
        if self.applying || !self.available_actions().contains(&state) {
            return;
        }
        let (Some(client), Some(tenant), Some(project), Some(finding)) = (
            self.client.clone(),
            self.tenant,
            self.project,
            self.selected_finding,
        ) else {
            return;
        };
        let note = self
            .note_input
            .as_ref()
            .map(|i| i.read(cx).value().trim().to_string())
            .filter(|n| !n.is_empty());
        self.applying = true;
        self.error = None;
        let generation = self.generation;
        let client_generation = self.client_generation;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = client
                .update_finding_state(tenant, project, finding, state, note)
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.client_generation != client_generation
                    || this.tenant != Some(tenant)
                    || this.client.is_none()
                {
                    return;
                }
                // PR aggregates changed independently of which detail is currently open.
                if result.is_ok() {
                    cx.emit(ReviewDetailEvent::Updated { project });
                }
                if this.generation != generation {
                    return;
                }
                this.applying = false;
                match result {
                    Ok(updated) => {
                        if let Some(f) = this.findings.iter_mut().find(|f| f.id == finding) {
                            *f = updated;
                        }
                        this.clear_note = true;
                    }
                    Err(
                        api::ApiError::Forbidden { message } | api::ApiError::Conflict { message },
                    ) => this.error = Some(message),
                    Err(e) => this.error = Some(e.to_string()),
                }
                // Always refresh authority, history and gate, including 403/409.
                this.load(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub fn current_review(&self) -> Option<(Uuid, i64, Option<String>)> {
        Some((self.project?, self.pr?, self.pr_title.clone()))
    }

    pub fn focus_current(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(list) = &self.finding_list {
            list.update(cx, |list, cx| list.focus(window, cx));
        }
    }

    fn set_filters(
        &mut self,
        state: Option<FindingState>,
        severity: Option<FindingSeverity>,
        cx: &mut Context<Self>,
    ) {
        if (self.state_filter, self.severity_filter) == (state, severity) {
            return;
        }
        self.state_filter = state;
        self.severity_filter = severity;
        self.clear_note = true;
        self.load(cx);
    }

    /// 状態・重大度の絞り込み。件数は summary の集計（絞り込み前の全体）を使う。
    fn filter_bar(&self, cx: &mut Context<Self>) -> Div {
        let counts = self
            .summary
            .as_ref()
            .map(|s| s.counts.clone())
            .unwrap_or_default();
        let count = |state: Option<FindingState>, severity: Option<FindingSeverity>| -> i64 {
            counts
                .iter()
                .filter(|c| state.is_none_or(|s| c.state == s))
                .filter(|c| severity.is_none_or(|s| c.severity == s))
                .map(|c| c.count)
                .sum()
        };
        let sev = self.severity_filter;
        let state = self.state_filter;
        let chip = |id: SharedString, label: String, selected: bool| {
            Button::new(id)
                .xsmall()
                .ghost()
                .selected(selected)
                .label(label)
                .disabled(self.loading)
        };
        let mut states = div().flex().flex_wrap().flex_1().min_w_0().gap_1().child(
            chip(
                "finding-state-all".into(),
                t!("reviews.filter.all", count = count(None, sev)),
                state.is_none(),
            )
            .on_click(cx.listener(move |this, _, _, cx| this.set_filters(None, sev, cx))),
        );
        for s in STATES {
            states = states.child(
                chip(
                    SharedString::from(format!("finding-state-{s}")),
                    format!("{} {}", state_label(s), count(Some(s), sev)),
                    state == Some(s),
                )
                .on_click(cx.listener(move |this, _, _, cx| this.set_filters(Some(s), sev, cx))),
            );
        }
        // 重大度は 1 つのメニューにまとめる（状態の絞り込みと 2 段に並べない）。
        let owner = cx.entity().downgrade();
        let severity_items: Vec<(Option<FindingSeverity>, String)> =
            std::iter::once((None, t!("reviews.filter.any_severity").to_string()))
                .chain(SEVERITIES.into_iter().map(|s| {
                    (
                        Some(s),
                        format!("{} {}", severity_label(s), count(state, Some(s))),
                    )
                }))
                .collect();
        let severity = Button::new("finding-severity")
            .xsmall()
            .ghost()
            .selected(sev.is_some())
            .disabled(self.loading)
            .label(match sev {
                Some(s) => severity_label(s).to_string(),
                None => t!("reviews.filter.any_severity").to_string(),
            })
            .dropdown_caret(true)
            .dropdown_menu(move |mut menu, _, _| {
                for (value, label) in &severity_items {
                    let (value, owner) = (*value, owner.clone());
                    menu = menu.item(
                        PopupMenuItem::new(label.clone())
                            .checked(sev == value)
                            .on_click(move |_, _, cx| {
                                let _ =
                                    owner.update(cx, |this, cx| this.set_filters(state, value, cx));
                            }),
                    );
                }
                menu
            });
        div()
            .flex()
            .items_start()
            .gap_2()
            .child(states)
            .child(severity)
    }
}

/// コピー用の小さなボタン。
fn copy_button(id: impl Into<ElementId>, text: String) -> Button {
    Button::new(id)
        .ghost()
        .xsmall()
        .icon(IconName::Copy)
        .tooltip(t!("reviews.detail.copy"))
        .on_click(move |_, window, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
            window.push_notification(Notification::new().message(t!("reviews.detail.copied")), cx);
        })
}

pub(crate) fn severity_label(severity: FindingSeverity) -> &'static str {
    match severity {
        FindingSeverity::High => t!("reviews.severity.high"),
        FindingSeverity::Medium => t!("reviews.severity.medium"),
        FindingSeverity::Low => t!("reviews.severity.low"),
        FindingSeverity::Nit => t!("reviews.severity.nit"),
    }
}

fn state_label(state: FindingState) -> &'static str {
    match state {
        FindingState::Open => t!("reviews.state.open"),
        FindingState::Fixing => t!("reviews.state.fixing"),
        FindingState::Fixed => t!("reviews.state.fixed"),
        FindingState::Verified => t!("reviews.state.verified"),
        FindingState::Deferred => t!("reviews.state.deferred"),
        FindingState::Rejected => t!("reviews.state.rejected"),
    }
}

fn action_label(state: FindingState) -> &'static str {
    match state {
        FindingState::Fixing => t!("reviews.action.fixing"),
        FindingState::Fixed => t!("reviews.action.fixed"),
        FindingState::Verified => t!("reviews.action.verify"),
        FindingState::Open => t!("reviews.action.reopen"),
        FindingState::Deferred => t!("reviews.action.defer"),
        FindingState::Rejected => t!("reviews.action.reject"),
    }
}
fn gate_label(summary: &ReviewSummary) -> String {
    let short = |s: &Option<String>| {
        s.as_deref()
            .unwrap_or(t!("reviews.detail.unknown"))
            .chars()
            .take(7)
            .collect::<String>()
    };
    match summary.gate {
        Some(Gate::Unlinked) => t!("reviews.gate.unlinked").into(),
        Some(Gate::Unreviewed) => t!("reviews.gate.unreviewed").into(),
        Some(Gate::Blocked) => t!("reviews.gate.blocked", count = summary.blocking),
        Some(Gate::StaleUnknown) => t!("reviews.gate.stale_unknown").into(),
        Some(Gate::Outdated) => t!(
            "reviews.gate.outdated",
            reviewed = short(&summary.latest_head_sha),
            current = short(&summary.cached_pr_head_sha)
        ),
        Some(Gate::Ready) => t!(
            "reviews.gate.ready",
            time = summary
                .pr_head_checked_at
                .map(|d| core::timezone::local(&d)
                    .format("%Y-%m-%d %H:%M")
                    .to_string())
                .unwrap_or_else(|| t!("reviews.detail.unknown").into())
        ),
        None => t!("reviews.gate.unavailable").into(),
    }
}

impl EventEmitter<ReviewDetailEvent> for ReviewDetailView {}
impl Render for ReviewDetailView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = ::ui::colors(cx);
        let c = t;
        if self.note_input.is_none() {
            self.note_input = Some(cx.new(|cx| {
                InputState::new(window, cx).placeholder(t!("reviews.detail.note_placeholder"))
            }));
        }
        if self.finding_list.is_none() {
            let list = cx.new(|cx| {
                ListState::new(
                    FindingRows {
                        rows: vec![],
                        loading: false,
                    },
                    window,
                    cx,
                )
            });
            let _ = list.read(cx).focus_handle(cx).tab_stop(true);
            self.list_subscription = Some(cx.subscribe(&list, |this, _, event: &ListEvent, cx| {
                if let ListEvent::Select(ix) | ListEvent::Confirm(ix) = event {
                    this.selected_finding = this.visible_findings().get(ix.row).map(|f| f.id);
                    this.clear_note = true;
                    cx.notify();
                }
            }));
            self.finding_list = Some(list);
        }
        if self.clear_note {
            self.clear_note = false;
            if let Some(note) = &self.note_input {
                note.update(cx, |s, cx| s.set_value("", window, cx));
            }
        }
        if self.error != self.shown_error {
            self.shown_error = self.error.clone();
            if let Some(error) = &self.error {
                window.push_notification(Notification::new().message(error.clone()), cx);
            }
        }

        let Some(pr) = self.pr else {
            let text: String = if self.loading {
                t!("reviews.detail.loading").into()
            } else {
                self.error
                    .clone()
                    .unwrap_or_else(|| t!("reviews.detail.empty").into())
            };
            return div()
                .id("review-detail-empty")
                .size_full()
                .flex()
                .items_center()
                .child(::ui::empty_state(
                    &::ui::colors(cx),
                    IconName::GitPullRequest,
                    text,
                    None,
                ));
        };
        let selected_round = self
            .selected_round
            .and_then(|id| self.reviews.iter().position(|r| r.id == id))
            .map(|ix| ix + 1)
            .unwrap_or(0);
        let rounds = TabBar::new("review-rounds")
            .underline()
            .small()
            .selected_index(selected_round)
            .menu(true)
            .child(Tab::new().label(t!("reviews.detail.all_rounds")))
            .children(
                self.reviews
                    .iter()
                    .map(|r| Tab::new().label(format!("R{}", r.round))),
            )
            .on_click(cx.listener(|this, ix: &usize, _, cx| {
                this.selected_round = ix
                    .checked_sub(1)
                    .and_then(|ix| this.reviews.get(ix))
                    .map(|r| r.id);
                this.selected_finding = this.visible_findings().first().map(|f| f.id);
                this.clear_note = true;
                cx.notify();
            }));
        let visible: Vec<Finding> = self.visible_findings().into_iter().cloned().collect();
        let selected_ix = visible
            .iter()
            .position(|f| Some(f.id) == self.selected_finding)
            .map(IndexPath::new);
        let list_height = px((visible.len().clamp(1, 4) * 52) as f32);
        let state = self.finding_list.as_ref().unwrap();
        state.update(cx, |state, cx| {
            state.delegate_mut().rows = visible;
            state.delegate_mut().loading = self.loading;
            state.set_selected_index(selected_ix, window, cx);
            cx.notify();
        });
        let finding_list = div().h(list_height).child(List::new(state));
        let filter_bar = self.filter_bar(cx);
        let selected = self
            .findings
            .iter()
            .find(|f| Some(f.id) == self.selected_finding)
            .cloned();
        let mut body = div().flex().flex_col().flex_shrink_0().min_w_0().gap_3();
        if let Some(f) = selected {
            let mut actions = div().flex().flex_wrap().gap_2();
            for action in &f.available_actions {
                let state = *action;
                actions =
                    actions.child(
                        Button::new(SharedString::from(format!("action-{state}")))
                            .small()
                            .label(action_label(state))
                            .disabled(self.applying || self.loading)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.apply_selected_action(state, cx)
                            })),
                    );
            }
            body = body
                .child(
                    div()
                        .text_base()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(f.title.clone()),
                )
                .child(div().text_xs().text_color(c.text_muted).child(format!(
                    "R{} · {} · {}",
                    f.round,
                    severity_label(f.severity),
                    state_label(f.state)
                )))
                .when_some(f.file.clone(), |d, file| {
                    let location = match f.line {
                        Some(line) => format!("{file}:{line}"),
                        None => file,
                    };
                    d.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .text_xs()
                            .text_color(c.text_muted)
                            .child(location.clone())
                            .child(copy_button("copy-location", location)),
                    )
                })
                .child(
                    TextView::markdown("finding-body", f.body)
                        .w_full()
                        .text_sm(),
                )
                .when_some(f.deferred_task_id, |d, task| {
                    d.child(
                        Button::new("deferred-task")
                            .ghost()
                            .small()
                            .icon(IconName::SquareArrowOutUpRight)
                            .label(t!("reviews.action.open_task"))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(project) = this.project {
                                    cx.emit(ReviewDetailEvent::OpenTask { project, task });
                                }
                            })),
                    )
                })
                .when(!f.available_actions.is_empty(), |d| {
                    d.child(Input::new(self.note_input.as_ref().unwrap()).id("finding-note"))
                        .child(actions)
                })
                .child(::ui::section_heading(
                    &::ui::colors(cx),
                    t!("reviews.detail.history"),
                ));
            // 状態の変化は時系列の縦線に並べ、「誰が・何から何へ・いつ」を 1 行目で読めるようにする。
            let state_color = |state: FindingState| match state {
                FindingState::Open | FindingState::Fixing => c.warning,
                FindingState::Fixed | FindingState::Verified => c.success,
                FindingState::Rejected => c.danger,
                FindingState::Deferred => c.text_muted,
            };
            let last = f.transitions.len().saturating_sub(1);
            for (ix, change) in f.transitions.iter().enumerate() {
                let color = state_color(change.to_state);
                body = body.child(
                    div()
                        .flex()
                        .gap_3()
                        .flex_shrink_0()
                        .min_w_0()
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .items_center()
                                .w(px(12.))
                                .flex_shrink_0()
                                .child(div().mt(px(6.)).size(px(8.)).rounded_full().bg(color))
                                .when(ix < last, |d| {
                                    d.child(div().mt_1().w(px(1.)).flex_1().bg(c.border))
                                }),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_1p5()
                                .flex_1()
                                .min_w_0()
                                .pb_4()
                                .child(
                                    div()
                                        .flex()
                                        .flex_wrap()
                                        .items_center()
                                        .gap_1p5()
                                        .text_sm()
                                        .child(
                                            div()
                                                .font_weight(FontWeight::MEDIUM)
                                                .child(change.actor.username.clone()),
                                        )
                                        .child(match change.from_state {
                                            Some(from) => ::ui::badge(
                                                &c,
                                                state_label(from),
                                                Some(state_color(from)),
                                            ),
                                            None => {
                                                ::ui::badge(&c, t!("reviews.detail.created"), None)
                                            }
                                        })
                                        .child(
                                            div()
                                                .text_color(c.text_muted)
                                                .child(Icon::new(IconName::ArrowRight).xsmall()),
                                        )
                                        .child(::ui::badge(
                                            &c,
                                            state_label(change.to_state),
                                            Some(color),
                                        ))
                                        .child(div().flex_1())
                                        .child(
                                            div().text_xs().text_color(c.text_muted).child(
                                                core::timezone::local(&change.created_at)
                                                    .format("%Y-%m-%d %H:%M")
                                                    .to_string(),
                                            ),
                                        ),
                                )
                                .when_some(change.note.clone(), |d, note| {
                                    d.child(
                                        div().px_3().py_2().rounded_md().bg(c.hover).child(
                                            TextView::markdown(("transition-note", ix), note)
                                                .w_full()
                                                .text_sm(),
                                        ),
                                    )
                                }),
                        ),
                );
            }
        }
        // 上（PR・判定・絞り込み・指摘一覧）は固定し、選んだ指摘の本文だけを下でスクロールする。
        // 1 本のスクロールだと本文を読むうちに一覧が流れて行き来しづらい。
        div()
            .id("review-detail")
            .size_full()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_shrink_0()
                    .gap_3()
                    .min_w_0()
                    .px_4()
                    .pt_4()
                    .pb_3()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_0p5()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(c.text_muted)
                                    .child(t!("reviews.detail.pr", number = pr)),
                            )
                            .when_some(self.pr_title.clone(), |d, title| {
                                d.child(
                                    div()
                                        .text_base()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(title),
                                )
                            }),
                    )
                    .when_some(self.summary.clone(), |d, s| {
                        let (gate_color, gate_icon) = match s.gate {
                            Some(Gate::Ready) => (t.success, IconName::CircleCheck),
                            Some(Gate::Blocked) => (t.danger, IconName::CircleX),
                            Some(Gate::Outdated | Gate::StaleUnknown) => {
                                (t.warning, IconName::TriangleAlert)
                            }
                            _ => (c.text_muted, IconName::Info),
                        };
                        d.child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_0p5()
                                .px_3()
                                .py_2()
                                .bg(gate_color.opacity(0.08))
                                .rounded_lg()
                                .border_1()
                                .border_color(gate_color.opacity(0.35))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .text_sm()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(Icon::new(gate_icon).size_4().text_color(gate_color))
                                        .child(gate_label(&s)),
                                )
                                .child(div().text_xs().text_color(c.text_muted).child(t!(
                                        "reviews.detail.repository",
                                        repository = s
                                            .repository
                                            .as_deref()
                                            .unwrap_or(t!("reviews.detail.not_linked")),
                                        rounds = s.rounds,
                                        rejections = s.owner_override_rejections
                                    )))
                                .child(div().text_xs().text_color(c.text_muted).child(t!(
                                        "reviews.detail.reviewed",
                                        sha = s
                                            .latest_head_sha
                                            .as_deref()
                                            .map(|sha| sha.chars().take(12).collect::<String>())
                                            .unwrap_or_else(|| t!("reviews.detail.unknown").into())
                                    )))
                                // マージ可能の見出しには確認日時が入っているので重ねない。
                                .when(!matches!(s.gate, Some(Gate::Ready)), |d| {
                                    d.child(div().text_xs().text_color(c.text_muted).child(t!(
                                            "reviews.detail.checked",
                                            time = s
                                                .pr_head_checked_at
                                                .map(|d| core::timezone::local(&d)
                                                    .format("%Y-%m-%d %H:%M")
                                                    .to_string())
                                                .unwrap_or_else(
                                                    || t!("reviews.detail.unknown").into()
                                                )
                                        )))
                                }),
                        )
                    })
                    .child(rounds)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .when(self.selected_round.is_none(), |d| d.hidden())
                            .children(
                                self.reviews
                                    .iter()
                                    // 「すべて」ではラウンドの行を出さない（ラウンドが多いと一覧が押し下がる）。
                                    .filter(|r| self.selected_round == Some(r.id))
                                    .enumerate()
                                    .map(|(ix, r)| {
                                        // 「すべて」では 1 行に縮め、ラウンドを選んだときだけ SHA と総評を出す。
                                        let one = self.selected_round.is_some();
                                        div()
                                            .flex()
                                            .flex_col()
                                            .gap_1()
                                            .text_xs()
                                            .text_color(c.text_muted)
                                            .child(t!(
                                                "reviews.detail.round_meta",
                                                round = r.round,
                                                reviewer = r.reviewer.username,
                                                left = if r.reviewer_left_tenant {
                                                    t!("reviews.detail.left_tenant")
                                                } else {
                                                    ""
                                                },
                                                count = r.finding_count,
                                                time = core::timezone::local(&r.created_at)
                                                    .format("%Y-%m-%d %H:%M")
                                            ))
                                            .when(one, |d| {
                                                d.child(
                                                    div()
                                                        .flex()
                                                        .items_center()
                                                        .gap_1()
                                                        .child(t!(
                                                            "reviews.detail.head",
                                                            sha = r.head_sha
                                                        ))
                                                        .child(copy_button(
                                                            SharedString::from(format!(
                                                                "copy-sha-{}",
                                                                r.id
                                                            )),
                                                            r.head_sha.clone(),
                                                        )),
                                                )
                                            })
                                            .when(one && !r.summary.is_empty(), |d| {
                                                d.child(
                                                    div().text_color(c.text).child(
                                                        TextView::markdown(
                                                            ("round-summary", ix),
                                                            r.summary.clone(),
                                                        )
                                                        .w_full()
                                                        .text_sm(),
                                                    ),
                                                )
                                            })
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .border_t_1()
                            .border_color(c.border)
                            .pt_3()
                            .child(filter_bar)
                            .child(finding_list),
                    )
                    .when_some(self.error.clone(), |d, error| {
                        d.child(div().text_sm().text_color(t.danger).child(error))
                    }),
            )
            .child(
                div()
                    .id("review-finding-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .border_t_1()
                    .border_color(c.border)
                    .px_4()
                    .py_3()
                    .child(body),
            )
    }
}
