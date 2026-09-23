//! §17-18 Review Detail。gate/Freshness は backend の値を表示するだけ
//! （Desktop は再計算しない §18.2）。Finding actions は `available_actions`
//! 駆動（§18.1）— 空ならボタンを出さない。

use api::spec::{Finding, Gate, ReviewSummary};
use api::types::{FindingSeverity, FindingState, ReviewResponse};
use api::{Client, FindingsQuery};
use gpui_kit::component::Theme;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use uuid::Uuid;

pub struct ReviewDetailView {
    client: Option<Client>,
    tenant: Option<Uuid>,
    project: Option<Uuid>,
    pr: Option<i64>,
    pr_title: Option<String>,
    summary: Option<ReviewSummary>,
    reviews: Vec<ReviewResponse>,
    findings: Vec<Finding>,
    loading: bool,
    error: Option<String>,
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
            loading: false,
            error: None,
        }
    }

    pub fn set_client(&mut self, client: Client, tenant: Option<Uuid>) {
        self.client = Some(client);
        self.tenant = tenant;
    }

    /// ログアウト時に呼ぶ。
    pub fn clear_client(&mut self) {
        self.client = None;
    }

    pub fn set_project(&mut self, project: Uuid) {
        self.project = Some(project);
        self.pr = None;
        self.summary = None;
        self.findings = vec![];
        self.reviews = vec![];
    }

    pub fn open(&mut self, pr: i64, pr_title: Option<String>, cx: &mut Context<Self>) {
        self.pr = Some(pr);
        self.pr_title = pr_title;
        self.load(cx);
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        let (Some(client), Some(tenant), Some(project), Some(pr)) =
            (self.client.clone(), self.tenant, self.project, self.pr)
        else {
            return;
        };
        self.loading = true;
        self.error = None;
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
                        repo: None,
                        state: None,
                        severity: None,
                    },
                )
                .await;
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                this.summary = summary.ok();
                this.reviews = reviews.unwrap_or_default();
                this.findings = findings.unwrap_or_default();
                cx.notify();
            });
        })
        .detach();
    }

    /// §18.1: action は backend が返す available_actions だけを出す。
    fn apply_action(&mut self, finding: Uuid, state: FindingState, cx: &mut Context<Self>) {
        let (Some(client), Some(tenant), Some(project)) =
            (self.client.clone(), self.tenant, self.project)
        else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let res = client
                .update_finding_state(tenant, project, finding, state, None)
                .await;
            let _ = this.update(cx, |this, cx| {
                match res {
                    Ok(updated) => {
                        if let Some(f) = this.findings.iter_mut().find(|f| f.id == finding) {
                            *f = updated;
                        }
                    }
                    Err(e) => this.error = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }
}

fn severity_color(sev: &FindingSeverity, t: &gpui_kit::component::Theme) -> Hsla {
    match sev {
        FindingSeverity::High => t.danger,
        FindingSeverity::Medium => t.warning,
        FindingSeverity::Low | FindingSeverity::Nit => t.semantic_tokens().colors.muted_foreground,
    }
}

fn state_label(state: &FindingState) -> &'static str {
    match state {
        FindingState::Open => "open",
        FindingState::Fixed => "fixed",
        FindingState::Verified => "verified",
        FindingState::Deferred => "deferred",
        FindingState::Rejected => "rejected",
    }
}

fn gate_label(gate: Option<Gate>, t: &gpui_kit::component::Theme) -> Option<(SharedString, Hsla)> {
    let (label, color) = match gate? {
        Gate::Ready => ("Ready", t.success),
        Gate::Blocked => ("Blocked", t.danger),
        Gate::Outdated => ("Outdated", t.warning),
        Gate::StaleUnknown => ("Freshness unknown", t.warning),
        Gate::Unreviewed => ("Unreviewed", t.semantic_tokens().colors.muted_foreground),
        Gate::Unlinked => ("Unlinked", t.semantic_tokens().colors.muted_foreground),
    };
    Some((label.into(), color))
}

impl Render for ReviewDetailView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::global(cx).clone();
        let c = t.semantic_tokens().colors.clone();

        let Some(pr) = self.pr else {
            return div().id("review-detail-empty").size_full().p_4().child(
                div()
                    .text_sm()
                    .text_color(c.muted_foreground)
                    .child("Select a pull request"),
            );
        };
        let summary = self.summary.clone();
        let gate = gate_label(summary.as_ref().and_then(|s| s.gate), &t);
        // §18.2: head が動いたかは server の latest/cached を表示するだけ。
        let head_moved = summary.as_ref().is_some_and(|s| {
            matches!(
                (&s.latest_head_sha, &s.cached_pr_head_sha),
                (Some(l), Some(c)) if l != c
            )
        });

        let mut findings_col = div().flex().flex_col();
        for f in self.findings.clone() {
            let fid = f.id;
            let sev_color = severity_color(&f.severity, &t);
            let mut actions_row = div().flex().flex_row().gap_1().flex_wrap();
            for action in f.available_actions.clone() {
                let label = match action {
                    FindingState::Fixed => "Mark fixed",
                    FindingState::Verified => "Verify",
                    FindingState::Deferred => "Defer",
                    FindingState::Rejected => "Reject",
                    FindingState::Open => "Reopen",
                };
                actions_row = actions_row.child(
                    div()
                        .id(ElementId::Name(
                            format!("act-{}-{}", fid.simple(), label).into(),
                        ))
                        .px_2()
                        .py_1()
                        .rounded_md()
                        .text_xs()
                        .cursor_pointer()
                        .bg(c.secondary)
                        .text_color(c.secondary_foreground)
                        .hover(|s| s.bg(c.muted))
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.apply_action(fid, action, cx)),
                        ),
                );
            }
            findings_col = findings_col.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(c.border)
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(sev_color)
                                    .child(format!("{}", f.severity).to_uppercase()),
                            )
                            .child(div().flex_1().text_sm().child(f.title.clone()))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(c.muted_foreground)
                                    .child(state_label(&f.state)),
                            ),
                    )
                    .when_some(f.file.clone(), |d, file| {
                        d.child(div().text_xs().text_color(c.muted_foreground).child(
                            match f.line {
                                Some(l) => format!("{file}:{l}"),
                                None => file,
                            },
                        ))
                    })
                    .child(
                        div()
                            .text_sm()
                            .text_color(c.muted_foreground)
                            .child(f.body.clone()),
                    )
                    .child(actions_row),
            );
        }
        if self.findings.is_empty() && !self.loading {
            findings_col = findings_col.child(
                div().p_4().child(
                    div()
                        .text_sm()
                        .text_color(c.muted_foreground)
                        .child("No findings"),
                ),
            );
        }

        div()
            .id("review-detail")
            .flex()
            .flex_col()
            .size_full()
            .overflow_y_scroll()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .px_4()
                    .py_3()
                    .border_b_1()
                    .border_color(c.border)
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_lg()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(format!("PR #{pr}")),
                            )
                            .when_some(gate, |d, (label, color)| {
                                d.child(
                                    div()
                                        .px_2()
                                        .py_1()
                                        .rounded_md()
                                        .text_xs()
                                        .text_color(color)
                                        .border_1()
                                        .border_color(color)
                                        .child(label),
                                )
                            })
                            .when(head_moved, |d| {
                                d.child(div().text_xs().text_color(t.warning).child("head moved"))
                            }),
                    )
                    .when_some(self.pr_title.clone(), |d, title| {
                        d.child(div().text_sm().child(title))
                    })
                    .when_some(summary.clone(), |d, s| {
                        d.child(
                            div()
                                .text_xs()
                                .text_color(c.muted_foreground)
                                .child(format!(
                                    "{} rounds · {} unresolved · {} blocking",
                                    s.rounds,
                                    s.counts
                                        .iter()
                                        .filter(|x| matches!(x.state, FindingState::Open))
                                        .map(|x| x.count)
                                        .sum::<i64>(),
                                    s.blocking,
                                )),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .px_4()
                    .py_2()
                    .border_b_1()
                    .border_color(c.border)
                    .child(
                        div()
                            .text_xs()
                            .text_color(c.muted_foreground)
                            .child("Rounds"),
                    )
                    .children(self.reviews.iter().map(|r| {
                        div()
                            .text_xs()
                            .text_color(c.muted_foreground)
                            .child(format!(
                                "round {} · {} findings · {} · {}",
                                r.round,
                                r.finding_count,
                                r.reviewer.username,
                                r.created_at.format("%m-%d %H:%M"),
                            ))
                    })),
            )
            .when_some(self.error.clone(), |d, e| {
                d.child(
                    div()
                        .px_4()
                        .py_2()
                        .child(div().text_sm().text_color(t.danger).child(e)),
                )
            })
            .child(findings_col)
    }
}
