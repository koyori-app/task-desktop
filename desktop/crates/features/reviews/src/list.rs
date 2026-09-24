//! Project PR list and local review drafts, submitted atomically as one round.
use api::Client;
use api::types::{
    CreateFindingInput, CreateReviewRequest, CreateReviewRequestHeadSha, FindingSeverity,
    ReviewedPullRequest,
};
use gpui_kit::assets::IconName;
use gpui_kit::component::IndexPath;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::list::{List, ListDelegate, ListEvent, ListItem, ListState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{Disableable, Selectable, Theme, WindowExt};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use i18n::t;
use uuid::Uuid;

use crate::detail::severity_label;

#[derive(Debug, Clone)]
pub enum ReviewListEvent {
    Select { pr: i64, title: Option<String> },
}

fn review_request(
    pr: &str,
    sha: &str,
    summary: &str,
    findings: Vec<CreateFindingInput>,
) -> Result<CreateReviewRequest, String> {
    let pr_number = pr
        .trim()
        .parse::<i32>()
        .ok()
        .filter(|n| *n > 0)
        .ok_or_else(|| t!("reviews.form.error.pr_number").to_string())?;
    let sha = sha.trim();
    let head_sha = CreateReviewRequestHeadSha::try_from(sha)
        .map_err(|_| t!("reviews.form.error.sha", count = sha.chars().count()))?;
    let summary = summary.trim();
    Ok(CreateReviewRequest {
        findings,
        head_sha,
        pr_number,
        summary: if summary.is_empty() {
            None
        } else {
            Some(summary.to_string())
        },
    })
}

struct PullRequestRows {
    rows: Vec<ReviewedPullRequest>,
    loading: bool,
}
impl ListDelegate for PullRequestRows {
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
            .p_4()
            .text_sm()
            .text_color(Theme::global(cx).semantic_tokens().colors.muted_foreground)
            .child(t!("reviews.list.empty"))
    }
    fn render_item(
        &mut self,
        ix: IndexPath,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<ListItem> {
        let pr = self.rows.get(ix.row)?;
        let t = Theme::global(cx);
        let c = t.semantic_tokens().colors;
        Some(
            ListItem::new(("pr-row", ix.row)).h(px(90.)).w_full().child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_sm()
                            .text_ellipsis()
                            .font_weight(FontWeight::MEDIUM)
                            .child(format!(
                                "#{} · {}",
                                pr.pr_number,
                                pr.pr_title
                                    .as_deref()
                                    .unwrap_or(t!("reviews.list.untitled"))
                            )),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(c.muted_foreground)
                            .text_ellipsis()
                            .child(t!(
                                "reviews.list.meta",
                                author = pr
                                    .pr_author
                                    .as_deref()
                                    .unwrap_or(t!("reviews.list.unknown_author")),
                                rounds = pr.rounds,
                                time = pr.last_reviewed_at.format("%Y-%m-%d %H:%M")
                            )),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(if pr.blocking > 0 {
                                t.danger
                            } else {
                                c.muted_foreground
                            })
                            .child(t!(
                                "reviews.list.counts",
                                unresolved = pr.unresolved,
                                blocking = pr.blocking
                            )),
                    ),
            ),
        )
    }
}

pub struct ReviewListView {
    client: Option<Client>,
    tenant: Option<Uuid>,
    project: Option<Uuid>,
    prs: Vec<ReviewedPullRequest>,
    loading: bool,
    error: Option<String>,
    shown_error: Option<String>,
    pr_input: Entity<InputState>,
    sha_input: Entity<InputState>,
    repo_input: Entity<InputState>,
    host_input: Entity<InputState>,
    summary_input: Entity<TextareaState>,
    finding_title: Entity<InputState>,
    finding_body: Entity<TextareaState>,
    finding_file: Entity<InputState>,
    finding_line: Entity<InputState>,
    severity: FindingSeverity,
    drafts: Vec<CreateFindingInput>,
    show_create: bool,
    submitting: bool,
    clear_draft: bool,
    clear_form: bool,
    list_state: Entity<ListState<PullRequestRows>>,
    _subs: Vec<Subscription>,
    selected: Option<usize>,
    generation: u64,
    context_generation: u64,
}
impl ReviewListView {
    pub fn new(
        client: Option<Client>,
        tenant: Option<Uuid>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let list_state = cx.new(|cx| {
            ListState::new(
                PullRequestRows {
                    rows: vec![],
                    loading: false,
                },
                window,
                cx,
            )
        });
        let _ = list_state.read(cx).focus_handle(cx).tab_stop(true);
        let sub = cx.subscribe(&list_state, |this, _, event: &ListEvent, cx| {
            if let ListEvent::Select(ix) | ListEvent::Confirm(ix) = event {
                this.selected = Some(ix.row);
                if let Some(pr) = this.prs.get(ix.row) {
                    cx.emit(ReviewListEvent::Select {
                        pr: pr.pr_number as i64,
                        title: pr.pr_title.clone(),
                    });
                }
                cx.notify();
            }
        });
        let mut this = Self {
            client,
            tenant,
            project: None,
            prs: vec![],
            loading: false,
            error: None,
            shown_error: None,
            pr_input: cx.new(|cx| {
                InputState::new(window, cx).placeholder(t!("reviews.form.pr_placeholder"))
            }),
            sha_input: cx.new(|cx| {
                InputState::new(window, cx).placeholder(t!("reviews.form.sha_placeholder"))
            }),
            repo_input: cx.new(|cx| {
                InputState::new(window, cx).placeholder(t!("reviews.form.repo_placeholder"))
            }),
            host_input: cx.new(|cx| {
                InputState::new(window, cx).placeholder(t!("reviews.form.host_placeholder"))
            }),
            summary_input: cx.new(|cx| {
                TextareaState::new(window, cx).placeholder(t!("reviews.form.summary_placeholder"))
            }),
            finding_title: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(t!("reviews.form.finding_title_placeholder"))
            }),
            finding_body: cx.new(|cx| {
                TextareaState::new(window, cx)
                    .placeholder(t!("reviews.form.finding_body_placeholder"))
            }),
            finding_file: cx.new(|cx| {
                InputState::new(window, cx).placeholder(t!("reviews.form.file_placeholder"))
            }),
            finding_line: cx.new(|cx| {
                InputState::new(window, cx).placeholder(t!("reviews.form.line_placeholder"))
            }),
            severity: FindingSeverity::Medium,
            drafts: vec![],
            show_create: false,
            submitting: false,
            clear_draft: false,
            clear_form: false,
            list_state,
            _subs: vec![sub],
            selected: None,
            generation: 0,
            context_generation: 0,
        };
        for input in [
            &this.pr_input,
            &this.sha_input,
            &this.finding_title,
            &this.finding_file,
            &this.finding_line,
            &this.repo_input,
            &this.host_input,
        ] {
            this._subs
                .push(cx.subscribe(input, |this, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        this.error = None;
                        this.shown_error = None;
                        cx.notify();
                    }
                }));
        }
        for input in [&this.summary_input, &this.finding_body] {
            this._subs
                .push(cx.subscribe(input, |this, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        this.error = None;
                        this.shown_error = None;
                        cx.notify();
                    }
                }));
        }
        this
    }
    pub fn set_client(&mut self, client: Client, tenant: Option<Uuid>) {
        if self.tenant != tenant {
            self.clear_client();
            self.project = None;
            self.clear_form = true;
            self.loading = false;
            self.submitting = false;
        }
        self.client = Some(client);
        self.tenant = tenant;
    }
    pub fn clear_client(&mut self) {
        self.context_generation += 1;
        self.submitting = false;
        self.client = None;
        self.generation += 1;
        self.prs.clear();
        self.drafts.clear();
        self.show_create = false;
    }
    pub fn set_project(&mut self, project: Uuid, cx: &mut Context<Self>) {
        if self.project != Some(project) {
            self.context_generation += 1;
            self.submitting = false;
            self.prs.clear();
            self.drafts.clear();
            self.show_create = false;
            self.clear_form = true;
            self.selected = None;
        }
        self.project = Some(project);
        self.reload(cx);
    }
    pub fn start_review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_create = true;
        self.pr_input.update(cx, |s, cx| s.focus(window, cx));
        cx.notify();
    }
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let (Some(client), Some(tenant), Some(project)) =
            (self.client.clone(), self.tenant, self.project)
        else {
            return;
        };
        self.loading = true;
        self.error = None;
        self.generation += 1;
        let generation = self.generation;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = client.list_reviewed_pull_requests(tenant, project).await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.loading = false;
                match result {
                    Ok(prs) => this.prs = prs,
                    Err(e) => this.error = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn add_finding(&mut self, cx: &mut Context<Self>) {
        self.shown_error = None;
        let title = self.finding_title.read(cx).value().trim().to_string();
        let body = self.finding_body.read(cx).value().trim().to_string();
        if title.is_empty() || body.is_empty() {
            self.error = Some(t!("reviews.form.error.finding_required").into());
            cx.notify();
            return;
        }
        let line_text = self.finding_line.read(cx).value().trim().to_string();
        let line = if line_text.is_empty() {
            None
        } else {
            match line_text.parse::<i32>() {
                Ok(n) if n > 0 => Some(n),
                _ => {
                    self.error = Some(t!("reviews.form.error.line").into());
                    cx.notify();
                    return;
                }
            }
        };
        let file = self.finding_file.read(cx).value().trim().to_string();
        self.drafts.push(CreateFindingInput {
            title,
            body,
            severity: self.severity,
            file: if file.is_empty() { None } else { Some(file) },
            line,
        });
        self.clear_draft = true;
        self.error = None;
        cx.notify();
    }
    fn create_review(&mut self, cx: &mut Context<Self>) {
        if self.submitting {
            return;
        }
        self.shown_error = None;
        let (Some(client), Some(tenant), Some(project)) =
            (self.client.clone(), self.tenant, self.project)
        else {
            self.error = Some(t!("reviews.form.error.no_project").into());
            cx.notify();
            return;
        };
        let body = match review_request(
            self.pr_input.read(cx).value().as_str(),
            self.sha_input.read(cx).value().as_str(),
            self.summary_input.read(cx).value().as_str(),
            self.drafts.clone(),
        ) {
            Ok(body) => body,
            Err(error) => {
                self.error = Some(error);
                cx.notify();
                return;
            }
        };
        if !self.finding_title.read(cx).value().trim().is_empty()
            || !self.finding_body.read(cx).value().trim().is_empty()
        {
            self.error = Some(t!("reviews.form.error.pending_finding").into());
            cx.notify();
            return;
        }
        let repo = self.repo_input.read(cx).value().trim().to_string();
        let host = self.host_input.read(cx).value().trim().to_string();
        self.submitting = true;
        let context_generation = self.context_generation;
        self.error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = client
                .create_review_with_repository(
                    tenant,
                    project,
                    &body,
                    if repo.is_empty() {
                        None
                    } else {
                        Some(repo.as_str())
                    },
                    if host.is_empty() {
                        None
                    } else {
                        Some(host.as_str())
                    },
                )
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.context_generation != context_generation
                    || this.project != Some(project)
                    || this.tenant != Some(tenant)
                    || this.client.is_none()
                {
                    return;
                }
                this.submitting = false;
                match result {
                    Ok(review) => {
                        this.show_create = false;
                        this.drafts.clear();
                        this.clear_form = true;
                        this.reload(cx);
                        cx.emit(ReviewListEvent::Select {
                            pr: review.pr_number as i64,
                            title: review.pr_title,
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
impl EventEmitter<ReviewListEvent> for ReviewListView {}
impl Render for ReviewListView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.clear_draft || self.clear_form {
            for input in [&self.finding_title, &self.finding_file, &self.finding_line] {
                input.update(cx, |s, cx| s.set_value("", window, cx));
            }
            self.finding_body
                .update(cx, |s, cx| s.set_value("", window, cx));
            self.clear_draft = false;
        }
        if self.clear_form {
            for input in [
                &self.pr_input,
                &self.sha_input,
                &self.repo_input,
                &self.host_input,
            ] {
                input.update(cx, |s, cx| s.set_value("", window, cx));
            }
            self.summary_input
                .update(cx, |s, cx| s.set_value("", window, cx));
            self.clear_form = false;
        }
        if self.error != self.shown_error {
            self.shown_error = self.error.clone();
            if let Some(e) = &self.error {
                window.push_notification(Notification::new().message(e.clone()), cx);
            }
        }
        let t = Theme::global(cx).clone();
        let c = t.semantic_tokens().colors;
        let mut severity_row = div().flex().flex_wrap().gap_1();
        for severity in [
            FindingSeverity::High,
            FindingSeverity::Medium,
            FindingSeverity::Low,
            FindingSeverity::Nit,
        ] {
            severity_row = severity_row.child(
                Button::new(SharedString::from(format!("severity-{severity}")))
                    .compact()
                    .selected(self.severity == severity)
                    .disabled(self.submitting)
                    .label(severity_label(severity))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.severity = severity;
                        cx.notify();
                    })),
            );
        }
        let form = div()
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .border_b_1()
            .border_color(c.border)
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(t!("reviews.form.title")),
            )
            .child(
                Input::new(&self.pr_input)
                    .disabled(self.submitting)
                    .id("review-pr"),
            )
            .child(
                Input::new(&self.sha_input)
                    .disabled(self.submitting)
                    .id("review-sha"),
            )
            .child(div().text_xs().text_color(c.muted_foreground).child(t!(
                "reviews.form.sha_count",
                count = self.sha_input.read(cx).value().trim().chars().count()
            )))
            .child(
                Input::new(&self.repo_input)
                    .disabled(self.submitting)
                    .id("review-repository"),
            )
            .child(
                Input::new(&self.host_input)
                    .disabled(self.submitting)
                    .id("review-host"),
            )
            .child(
                Textarea::new(&self.summary_input)
                    .disabled(self.submitting)
                    .h(px(90.)),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(c.muted_foreground)
                    .child(t!("reviews.form.drafts_heading")),
            )
            .children(self.drafts.iter().enumerate().map(|(ix, f)| {
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().min_w_0().text_sm().child(format!(
                        "{} · {}",
                        severity_label(f.severity),
                        f.title
                    )))
                    .child(
                        Button::new(("remove-draft", ix))
                            .disabled(self.submitting)
                            .compact()
                            .ghost()
                            .label(t!("reviews.form.remove"))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if ix < this.drafts.len() {
                                    this.drafts.remove(ix);
                                }
                                cx.notify();
                            })),
                    )
            }))
            .child(
                Input::new(&self.finding_title)
                    .disabled(self.submitting)
                    .id("draft-finding-title"),
            )
            .child(severity_row)
            .child(
                Textarea::new(&self.finding_body)
                    .disabled(self.submitting)
                    .h(px(100.)),
            )
            .child(
                Input::new(&self.finding_file)
                    .disabled(self.submitting)
                    .id("draft-finding-file"),
            )
            .child(
                Input::new(&self.finding_line)
                    .disabled(self.submitting)
                    .id("draft-finding-line"),
            )
            .child(
                Button::new("add-finding")
                    .label(t!("reviews.form.add_finding"))
                    .disabled(self.submitting)
                    .on_click(cx.listener(|this, _, _, cx| this.add_finding(cx))),
            )
            .child(
                Button::new("submit-review")
                    .primary()
                    .label(if self.submitting {
                        t!("reviews.form.submitting").into()
                    } else {
                        t!("reviews.form.submit", count = self.drafts.len())
                    })
                    .disabled(self.submitting)
                    .on_click(cx.listener(|this, _, _, cx| this.create_review(cx))),
            );
        self.list_state.update(cx, |state, cx| {
            state.delegate_mut().rows = self.prs.clone();
            state.delegate_mut().loading = self.loading;
            state.set_selected_index(self.selected.map(IndexPath::new), window, cx);
            cx.notify();
        });
        let content = if self.show_create {
            div()
                .id("review-draft-scroll")
                .size_full()
                .overflow_y_scroll()
                .child(form)
                .into_any_element()
        } else {
            List::new(&self.list_state).into_any_element()
        };
        div()
            .id("review-list-view")
            .flex()
            .flex_col()
            .size_full()
            .min_w_0()
            .min_h_0()
            .child(
                div()
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .flex_wrap()
                    .gap_2()
                    .px_4()
                    .py_2()
                    .border_b_1()
                    .border_color(c.border)
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(t!("reviews.list.title")),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("rv-refresh")
                            .ghost()
                            .icon(IconName::RefreshCcwDot)
                            .tooltip(t!("reviews.list.refresh"))
                            .on_click(cx.listener(|this, _, _, cx| this.reload(cx))),
                    )
                    .child(
                        Button::new("rv-new")
                            .compact()
                            .label(if self.show_create {
                                t!("reviews.list.close_draft")
                            } else {
                                t!("reviews.list.new")
                            })
                            .on_click(cx.listener(|this, _, window, cx| {
                                if this.show_create {
                                    this.show_create = false;
                                    cx.notify();
                                } else {
                                    this.start_review(window, cx);
                                }
                            })),
                    ),
            )
            .when_some(self.error.clone(), |view, error| {
                view.child(
                    div()
                        .px_4()
                        .py_2()
                        .flex_shrink_0()
                        .child(Alert::error("review-form-error", error)),
                )
            })
            .child(
                div()
                    .id("review-list-scroll")
                    .flex_1()
                    .min_h_0()
                    .child(content),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::review_request;

    #[test]
    fn corrected_sha_builds_a_zero_finding_review_request() {
        assert!(review_request("42", "ABC123", "", vec![]).is_err());
        let request =
            review_request("42", "0123456789abcdef0123456789abcdef01234567", "", vec![]).unwrap();
        assert_eq!(request.pr_number, 42);
        assert!(request.findings.is_empty());
        assert_eq!(
            String::from(request.head_sha),
            "0123456789abcdef0123456789abcdef01234567"
        );
    }

    #[test]
    fn rejects_invalid_pr_and_reports_incomplete_sha_length() {
        let sha = "0123456789abcdef0123456789abcdef01234567";
        assert!(review_request("0", sha, "", vec![]).is_err());
        // 言語はプロセス共有なので切り替えず、現在の言語の文言と比べる。
        let error = review_request("42", "ABC123", "", vec![]).unwrap_err();
        assert_eq!(error, i18n::t!("reviews.form.error.sha", count = 6));
        assert!(review_request("42", &sha.to_uppercase(), "", vec![]).is_err());
    }
}
