//! Review rounds, findings and server-provided actions and gate.
use api::spec::{Finding, Gate, ReviewSummary};
use api::types::{FindingState, ReviewResponse};
use api::{Client, FindingsQuery};
use gpui_kit::component::IndexPath;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::list::{List, ListDelegate, ListEvent, ListItem, ListState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::text::TextView;
use gpui_kit::component::{Disableable, Theme, WindowExt};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
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
        div()
            .p_3()
            .text_sm()
            .text_color(Theme::global(cx).semantic_tokens().colors.muted_foreground)
            .child("No findings in this round")
    }
    fn render_item(
        &mut self,
        ix: IndexPath,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<ListItem> {
        let finding = self.rows.get(ix.row)?;
        Some(
            ListItem::new(("finding", ix.row))
                .h(px(64.))
                .w_full()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .min_w_0()
                        .flex_1()
                        .child(div().text_sm().text_ellipsis().child(finding.title.clone()))
                        .child(
                            div()
                                .text_xs()
                                .text_color(
                                    Theme::global(cx).semantic_tokens().colors.muted_foreground,
                                )
                                .child(format!(
                                    "R{} · {} · {}",
                                    finding.round, finding.severity, finding.state
                                )),
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
                        this.error = Some("This review no longer exists.".into());
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
                if let Some(id) = this.selected_finding
                    && !this.findings.iter().any(|f| f.id == id)
                {
                    this.selected_finding = None;
                    this.error = Some("This finding no longer exists.".into());
                }
                if this.selected_finding.is_none() {
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
}

fn action_label(state: FindingState) -> &'static str {
    match state {
        FindingState::Fixed => "Mark as Fixed",
        FindingState::Verified => "Verify",
        FindingState::Open => "Reopen",
        FindingState::Deferred => "Defer",
        FindingState::Rejected => "Reject",
    }
}
fn gate_label(summary: &ReviewSummary) -> String {
    let short = |s: &Option<String>| {
        s.as_deref()
            .unwrap_or("unknown")
            .chars()
            .take(7)
            .collect::<String>()
    };
    match summary.gate {
        Some(Gate::Unlinked) => "Repository not linked · not used as a merge gate".into(),
        Some(Gate::Unreviewed) => "Not reviewed".into(),
        Some(Gate::Blocked) => format!("Blocked · {} findings", summary.blocking),
        Some(Gate::StaleUnknown) => "Freshness unknown".into(),
        Some(Gate::Outdated) => format!(
            "Review is outdated · reviewed {} / current {}",
            short(&summary.latest_head_sha),
            short(&summary.cached_pr_head_sha)
        ),
        Some(Gate::Ready) => format!(
            "Ready · checked at {}",
            summary
                .pr_head_checked_at
                .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_else(|| "unknown".into())
        ),
        None => "Review gate unavailable".into(),
    }
}

impl EventEmitter<ReviewDetailEvent> for ReviewDetailView {}
impl Render for ReviewDetailView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::global(cx).clone();
        let c = t.semantic_tokens().colors;
        if self.note_input.is_none() {
            self.note_input =
                Some(cx.new(|cx| {
                    InputState::new(window, cx).placeholder("Transition note (optional)")
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
            return div()
                .id("review-detail-empty")
                .size_full()
                .p_4()
                .text_sm()
                .text_color(c.muted_foreground)
                .child(if self.loading {
                    "Loading review…".into()
                } else {
                    self.error
                        .clone()
                        .unwrap_or_else(|| "Select a pull request".into())
                });
        };
        let selected_round = self
            .selected_round
            .and_then(|id| self.reviews.iter().position(|r| r.id == id))
            .map(|ix| ix + 1)
            .unwrap_or(0);
        let rounds = TabBar::new("review-rounds")
            .selected_index(selected_round)
            .menu(true)
            .child(Tab::new().label("All rounds"))
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
        let list_height = px((visible.len().clamp(1, 4) * 64) as f32);
        let state = self.finding_list.as_ref().unwrap();
        state.update(cx, |state, cx| {
            state.delegate_mut().rows = visible;
            state.delegate_mut().loading = self.loading;
            state.set_selected_index(selected_ix, window, cx);
            cx.notify();
        });
        let finding_list = div().h(list_height).child(List::new(state));
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
                            .compact()
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
                        .text_lg()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(f.title.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(c.muted_foreground)
                        .child(format!("R{} · {} · {}", f.round, f.severity, f.state)),
                )
                .when_some(f.file.clone(), |d, file| {
                    d.child(
                        div()
                            .text_xs()
                            .text_color(c.muted_foreground)
                            .child(match f.line {
                                Some(line) => format!("{file}:{line}"),
                                None => file,
                            }),
                    )
                })
                .child(TextView::markdown("finding-body", f.body).w_full())
                .when_some(f.deferred_task_id, |d, task| {
                    d.child(
                        Button::new("deferred-task")
                            .ghost()
                            .label("Open deferred task")
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
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("History"),
                );
            for (ix, change) in f.transitions.iter().enumerate() {
                body = body.child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_shrink_0()
                        .gap_1()
                        .py_2()
                        .border_t_1()
                        .border_color(c.border)
                        .child(
                            div()
                                .text_xs()
                                .text_color(c.muted_foreground)
                                .child(format!(
                                    "{} · {} · {} → {}",
                                    change.actor.username,
                                    change.created_at.format("%Y-%m-%d %H:%M"),
                                    change
                                        .from_state
                                        .map(|s| s.to_string())
                                        .unwrap_or_else(|| "created".into()),
                                    change.to_state
                                )),
                        )
                        .when_some(change.note.clone(), |d, note| {
                            d.child(TextView::markdown(("transition-note", ix), note).w_full())
                        }),
                );
            }
        }
        div()
            .id("review-detail")
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
                    .min_w_0()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("PR #{pr}")),
                    )
                    .when_some(self.pr_title.clone(), |d, title| {
                        d.child(div().text_sm().child(title))
                    })
                    .when_some(self.summary.clone(), |d, s| {
                        d.child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .p_3()
                                .bg(c.secondary)
                                .rounded_md()
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(gate_label(&s)),
                                )
                                .child(div().text_xs().text_color(c.muted_foreground).child(
                                    format!(
                                        "Repository: {} · {} rounds · {} owner override rejections",
                                        s.repository.as_deref().unwrap_or("not linked"),
                                        s.rounds,
                                        s.owner_override_rejections
                                    ),
                                ))
                                .child(div().text_xs().text_color(c.muted_foreground).child(
                                    format!(
                                        "Reviewed: {}",
                                        s.latest_head_sha.as_deref().unwrap_or("unknown")
                                    ),
                                ))
                                .child(div().text_xs().text_color(c.muted_foreground).child(
                                    format!(
                                            "Checked: {}",
                                            s.pr_head_checked_at
                                                .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
                                                .unwrap_or_else(|| "unknown".into())
                                        ),
                                )),
                        )
                    })
                    .child(rounds)
                    .children(
                        self.reviews
                            .iter()
                            .filter(|r| self.selected_round.is_none_or(|id| r.id == id))
                            .enumerate()
                            .map(|(ix, r)| {
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .text_xs()
                                    .text_color(c.muted_foreground)
                                    .child(format!(
                                        "R{} · {}{} · {} findings · {}",
                                        r.round,
                                        r.reviewer.username,
                                        if r.reviewer_left_tenant {
                                            " (left tenant)"
                                        } else {
                                            ""
                                        },
                                        r.finding_count,
                                        r.created_at.format("%Y-%m-%d %H:%M")
                                    ))
                                    .child(format!("Head: {}", r.head_sha))
                                    .when(!r.summary.is_empty(), |d| {
                                        d.child(
                                            TextView::markdown(
                                                ("round-summary", ix),
                                                r.summary.clone(),
                                            )
                                            .w_full(),
                                        )
                                    })
                            }),
                    )
                    .child(
                        div()
                            .border_t_1()
                            .border_color(c.border)
                            .pt_3()
                            .child(finding_list),
                    )
                    .when_some(self.error.clone(), |d, error| {
                        d.child(div().text_sm().text_color(t.danger).child(error))
                    })
                    .child(div().border_t_1().border_color(c.border).pt_3().child(body)),
            )
    }
}
