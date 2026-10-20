//! マイルストーン（期日と進捗）とスプリント（期間・開始・完了・バーンダウン）。

use api::types::{
    CompleteSprintRequest, CreateMilestoneRequest, CreateSprintRequest, SprintDetail, SprintStatus,
    UpdateMilestoneRequest, UpdateSprintRequest,
};
use chrono::NaiveDate;
use gpui_kit::assets::IconName;
use gpui_kit::component::Disableable;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::calendar::Date;
use gpui_kit::component::date_picker::DatePicker;
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::progress::Progress;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use i18n::t;
use uuid::Uuid;

use super::{ProjectSettingsView, Section};
use crate::ui::{Colors, badge, card, confirm, field, heading, picked, placeholder, row};

fn parse_day(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s.get(..10)?, "%Y-%m-%d").ok()
}

fn sprint_status_label(status: SprintStatus) -> &'static str {
    match status {
        SprintStatus::Planning => t!("projects.sprints.planning"),
        SprintStatus::Active => t!("projects.sprints.active"),
        SprintStatus::Completed => t!("projects.sprints.completed"),
    }
}

impl ProjectSettingsView {
    // ---- milestones ----

    pub(crate) fn save_milestone(&mut self, cx: &mut Context<Self>) {
        let Some((client, tenant, project)) = self.ctx() else {
            return;
        };
        let f = &self.forms;
        let name = f.milestone_name.read(cx).value().trim().to_string();
        let Some(due) = picked(&f.milestone_due, cx) else {
            self.notice(t!("projects.milestones.due_required"));
            cx.notify();
            return;
        };
        if name.is_empty() {
            self.notice(t!("projects.common.name_required"));
            cx.notify();
            return;
        }
        let description = f.milestone_description.read(cx).value().trim().to_string();
        let editing = f.editing_milestone;
        self.call(
            cx,
            async move {
                match editing {
                    Some(id) => {
                        let req = UpdateMilestoneRequest {
                            name: Some(name),
                            due_date: Some(due),
                            clear_description: description.is_empty().then_some(true),
                            description: (!description.is_empty()).then_some(description),
                        };
                        client.update_milestone(tenant, project, id, &req).await
                    }
                    None => {
                        let req = CreateMilestoneRequest {
                            name,
                            due_date: due,
                            description: (!description.is_empty()).then_some(description),
                        };
                        client.create_milestone(tenant, project, &req).await
                    }
                }
            },
            |this, _, cx| this.after_planning_change(Section::Milestones, cx),
        );
    }

    fn after_planning_change(&mut self, section: Section, cx: &mut Context<Self>) {
        self.reset_form(section);
        self.load(section, cx);
        self.catalog_changed(cx);
    }

    fn edit_milestone(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        let Some(m) = self.milestones.iter().find(|m| m.id == id).cloned() else {
            return;
        };
        let f = &mut self.forms;
        f.editing_milestone = Some(id);
        f.milestone_name
            .update(cx, |i, cx| i.set_value(m.name.clone(), window, cx));
        f.milestone_description.update(cx, |i, cx| {
            i.set_value(m.description.clone().unwrap_or_default(), window, cx)
        });
        let due = parse_day(&m.due_date);
        f.milestone_due
            .update(cx, |s, cx| s.set_date(Date::Single(due), window, cx));
        cx.notify();
    }

    fn delete_milestone(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let Some((client, tenant, project)) = self.ctx() else {
            return;
        };
        self.call(
            cx,
            async move { client.delete_milestone(tenant, project, id).await },
            |this, (), cx| this.after_planning_change(Section::Milestones, cx),
        );
    }

    fn form_header(&self, editing: bool, new: &'static str, edit: &'static str) -> Div {
        div()
            .text_sm()
            .font_weight(FontWeight::MEDIUM)
            .child(if editing { edit } else { new })
    }

    pub(crate) fn milestones_page(&self, c: &Colors, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let f = &self.forms;
        let busy = self.busy > 0;
        let editing = f.editing_milestone.is_some();
        let form = div()
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .child(self.form_header(
                editing,
                t!("projects.milestones.new"),
                t!("projects.milestones.edit"),
            ))
            .child(
                div()
                    .flex()
                    .gap_3()
                    .child(div().flex_1().child(field(
                        c,
                        t!("projects.common.name"),
                        Input::new(&f.milestone_name),
                    )))
                    .child(
                        div().w(px(180.)).child(field(
                            c,
                            t!("projects.milestones.due"),
                            DatePicker::new(&f.milestone_due)
                                .placeholder(t!("projects.common.pick_date")),
                        )),
                    ),
            )
            .child(field(
                c,
                t!("projects.common.description"),
                Input::new(&f.milestone_description),
            ))
            .child(self.form_buttons(
                "milestone",
                editing,
                cx,
                |this, cx| this.save_milestone(cx),
                Section::Milestones,
            ))
            .into_any_element();

        let mut rows = vec![];
        if self.is_loading(Section::Milestones) {
            rows.push(placeholder(c, t!("projects.common.loading")));
        } else if self.milestones.is_empty() {
            rows.push(placeholder(c, t!("projects.milestones.empty")));
        }
        let this = cx.entity().downgrade();
        let today = core::timezone::today();
        for m in &self.milestones {
            let id = m.id;
            let name = m.name.clone();
            let overdue = parse_day(&m.due_date).is_some_and(|d| d < today)
                && m.task_counts.done < m.task_counts.total;
            let (edit_owner, delete_owner) = (this.clone(), this.clone());
            rows.push(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(row(
                        c,
                        None,
                        m.name.clone(),
                        m.description.clone(),
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(badge(
                                c,
                                m.due_date.get(..10).unwrap_or(&m.due_date).to_string(),
                                overdue.then_some(c.danger),
                            ))
                            .child(
                                Button::new(SharedString::from(format!("ms-edit-{id}")))
                                    .ghost()
                                    .small()
                                    .label(t!("projects.common.edit"))
                                    .on_click(move |_, window, cx| {
                                        let _ = edit_owner.update(cx, |this, cx| {
                                            this.edit_milestone(id, window, cx)
                                        });
                                    }),
                            )
                            .child(
                                Button::new(SharedString::from(format!("ms-delete-{id}")))
                                    .ghost()
                                    .small()
                                    .label(t!("projects.common.delete"))
                                    .disabled(busy)
                                    .on_click(move |_, window, cx| {
                                        let owner = delete_owner.clone();
                                        confirm(
                                            window,
                                            cx,
                                            t!("projects.milestones.delete_title", name = name),
                                            t!("projects.milestones.delete_desc").into(),
                                            move |_, cx| {
                                                let _ = owner.update(cx, |this, cx| {
                                                    this.delete_milestone(id, cx)
                                                });
                                            },
                                        );
                                    }),
                            ),
                    ))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .px_4()
                            .pb_3()
                            .child(
                                div().flex_1().child(
                                    Progress::new(SharedString::from(format!("ms-progress-{id}")))
                                        .value(m.progress_pct as f32),
                                ),
                            )
                            .child(div().text_xs().text_color(c.text_muted).child(t!(
                                "projects.milestones.progress",
                                pct = m.progress_pct,
                                done = m.task_counts.done,
                                total = m.task_counts.total
                            ))),
                    )
                    .into_any_element(),
            );
        }
        vec![card(c, vec![form]), card(c, rows)]
    }

    // ---- sprints ----

    pub(crate) fn save_sprint(&mut self, cx: &mut Context<Self>) {
        let Some((client, tenant, project)) = self.ctx() else {
            return;
        };
        let f = &self.forms;
        let name = f.sprint_name.read(cx).value().trim().to_string();
        let (Some(start), Some(end)) = (picked(&f.sprint_start, cx), picked(&f.sprint_end, cx))
        else {
            self.notice(t!("projects.sprints.dates_required"));
            cx.notify();
            return;
        };
        if name.is_empty() {
            self.notice(t!("projects.common.name_required"));
            cx.notify();
            return;
        }
        if end < start {
            self.notice(t!("projects.sprints.dates_order"));
            cx.notify();
            return;
        }
        let goal = f.sprint_goal.read(cx).value().trim().to_string();
        let editing = f.editing_sprint;
        let (start, end) = (start.to_string(), end.to_string());
        self.call(
            cx,
            async move {
                match editing {
                    Some(id) => {
                        let req = UpdateSprintRequest {
                            name: Some(name),
                            start_date: Some(start),
                            end_date: Some(end),
                            clear_goal: goal.is_empty().then_some(true),
                            goal: (!goal.is_empty()).then_some(goal),
                        };
                        client.update_sprint(tenant, project, id, &req).await
                    }
                    None => {
                        let req = CreateSprintRequest {
                            name,
                            start_date: start,
                            end_date: end,
                            goal: (!goal.is_empty()).then_some(goal),
                        };
                        client.create_sprint(tenant, project, &req).await
                    }
                }
            },
            |this, _, cx| this.after_planning_change(Section::Sprints, cx),
        );
    }

    fn edit_sprint(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        let Some(s) = self.sprints.iter().find(|s| s.id == id).cloned() else {
            return;
        };
        let f = &mut self.forms;
        f.editing_sprint = Some(id);
        f.sprint_name
            .update(cx, |i, cx| i.set_value(s.name.clone(), window, cx));
        f.sprint_goal.update(cx, |i, cx| {
            i.set_value(s.goal.clone().unwrap_or_default(), window, cx)
        });
        let (start, end) = (parse_day(&s.start_date), parse_day(&s.end_date));
        f.sprint_start
            .update(cx, |p, cx| p.set_date(Date::Single(start), window, cx));
        f.sprint_end
            .update(cx, |p, cx| p.set_date(Date::Single(end), window, cx));
        cx.notify();
    }

    fn sprint_action(&mut self, id: Uuid, action: SprintAction, cx: &mut Context<Self>) {
        let Some((client, tenant, project)) = self.ctx() else {
            return;
        };
        self.call(
            cx,
            async move {
                match action {
                    SprintAction::Start => {
                        client.start_sprint(tenant, project, id).await.map(|_| ())
                    }
                    SprintAction::Delete => client.delete_sprint(tenant, project, id).await,
                    SprintAction::Complete(to) => {
                        let req = CompleteSprintRequest {
                            move_incomplete_to_backlog: Some(to.is_none()),
                            move_incomplete_to_sprint_id: to,
                        };
                        client
                            .complete_sprint(tenant, project, id, &req)
                            .await
                            .map(|_| ())
                    }
                }
            },
            |this, (), cx| this.after_planning_change(Section::Sprints, cx),
        );
    }

    pub(crate) fn sprints_page(&self, c: &Colors, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let f = &self.forms;
        let editing = f.editing_sprint.is_some();
        let form = div()
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .child(self.form_header(
                editing,
                t!("projects.sprints.new"),
                t!("projects.sprints.edit"),
            ))
            .child(field(
                c,
                t!("projects.common.name"),
                Input::new(&f.sprint_name),
            ))
            .child(
                div()
                    .flex()
                    .gap_3()
                    .child(
                        div().w(px(180.)).child(field(
                            c,
                            t!("projects.sprints.start"),
                            DatePicker::new(&f.sprint_start)
                                .placeholder(t!("projects.common.pick_date")),
                        )),
                    )
                    .child(div().w(px(180.)).child(field(
                        c,
                        t!("projects.sprints.end"),
                        DatePicker::new(&f.sprint_end).placeholder(t!("projects.common.pick_date")),
                    ))),
            )
            .child(field(
                c,
                t!("projects.sprints.goal"),
                Input::new(&f.sprint_goal),
            ))
            .child(self.form_buttons(
                "sprint",
                editing,
                cx,
                |this, cx| this.save_sprint(cx),
                Section::Sprints,
            ))
            .into_any_element();

        let mut out = vec![card(c, vec![form])];
        if self.is_loading(Section::Sprints) {
            out.push(card(c, vec![placeholder(c, t!("projects.common.loading"))]));
            return out;
        }
        if self.sprints.is_empty() {
            out.push(card(c, vec![placeholder(c, t!("projects.sprints.empty"))]));
            return out;
        }
        let planning: Vec<(Uuid, String)> = self
            .sprints
            .iter()
            .filter(|s| s.status == SprintStatus::Planning)
            .map(|s| (s.id, s.name.clone()))
            .collect();
        for status in [
            SprintStatus::Active,
            SprintStatus::Planning,
            SprintStatus::Completed,
        ] {
            let rows: Vec<AnyElement> = self
                .sprints
                .iter()
                .filter(|s| s.status == status)
                .map(|s| self.sprint_row(c, s, &planning, cx))
                .collect();
            if !rows.is_empty() {
                out.push(heading(c, sprint_status_label(status)));
                out.push(card(c, rows));
            }
        }
        out
    }

    fn sprint_row(
        &self,
        c: &Colors,
        s: &SprintDetail,
        planning: &[(Uuid, String)],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let busy = self.busy > 0;
        let id = s.id;
        let this = cx.entity().downgrade();
        let name = s.name.clone();
        let mut controls = div().flex().items_center().gap_1();
        match s.status {
            SprintStatus::Planning => {
                let (start_owner, edit_owner, delete_owner) =
                    (this.clone(), this.clone(), this.clone());
                let delete_name = name.clone();
                controls = controls
                    .child(
                        Button::new(SharedString::from(format!("sprint-start-{id}")))
                            .outline()
                            .small()
                            .icon(IconName::Play)
                            .label(t!("projects.sprints.start_action"))
                            .disabled(busy)
                            .on_click(move |_, _, cx| {
                                let _ = start_owner.update(cx, |this, cx| {
                                    this.sprint_action(id, SprintAction::Start, cx)
                                });
                            }),
                    )
                    .child(
                        Button::new(SharedString::from(format!("sprint-edit-{id}")))
                            .ghost()
                            .small()
                            .label(t!("projects.common.edit"))
                            .on_click(move |_, window, cx| {
                                let _ = edit_owner
                                    .update(cx, |this, cx| this.edit_sprint(id, window, cx));
                            }),
                    )
                    .child(
                        Button::new(SharedString::from(format!("sprint-delete-{id}")))
                            .ghost()
                            .small()
                            .label(t!("projects.common.delete"))
                            .disabled(busy)
                            .on_click(move |_, window, cx| {
                                let owner = delete_owner.clone();
                                confirm(
                                    window,
                                    cx,
                                    t!("projects.sprints.delete_title", name = delete_name),
                                    t!("projects.sprints.delete_desc").into(),
                                    move |_, cx| {
                                        let _ = owner.update(cx, |this, cx| {
                                            this.sprint_action(id, SprintAction::Delete, cx)
                                        });
                                    },
                                );
                            }),
                    );
            }
            SprintStatus::Active => {
                let targets: Vec<(Option<Uuid>, String)> =
                    std::iter::once((None, t!("projects.sprints.complete_backlog").to_string()))
                        .chain(planning.iter().map(|(pid, pname)| {
                            (
                                Some(*pid),
                                t!("projects.sprints.complete_move", name = pname),
                            )
                        }))
                        .collect();
                controls = controls.child(
                    Button::new(SharedString::from(format!("sprint-complete-{id}")))
                        .primary()
                        .small()
                        .icon(IconName::CircleCheck)
                        .label(t!("projects.sprints.complete_action"))
                        .dropdown_caret(true)
                        .disabled(busy)
                        .dropdown_menu(move |mut menu, _, _| {
                            for (target, label) in &targets {
                                let (target, owner) = (*target, this.clone());
                                let (label, name) = (label.clone(), name.clone());
                                menu = menu.item(PopupMenuItem::new(label.clone()).on_click(
                                    move |_, window, cx| {
                                        let owner = owner.clone();
                                        confirm(
                                            window,
                                            cx,
                                            t!("projects.sprints.complete_title", name = name),
                                            label.clone(),
                                            move |_, cx| {
                                                let _ = owner.update(cx, |this, cx| {
                                                    this.sprint_action(
                                                        id,
                                                        SprintAction::Complete(target),
                                                        cx,
                                                    )
                                                });
                                            },
                                        );
                                    },
                                ));
                            }
                            menu
                        }),
                );
            }
            SprintStatus::Completed => {}
        }
        let counts = &s.task_counts;
        let pct = if counts.total > 0 {
            counts.done as f32 * 100. / counts.total as f32
        } else {
            0.
        };
        let description = format!(
            "{} – {}{}",
            s.start_date.get(..10).unwrap_or(&s.start_date),
            s.end_date.get(..10).unwrap_or(&s.end_date),
            s.goal
                .as_ref()
                .filter(|g| !g.is_empty())
                .map(|g| format!(" · {g}"))
                .unwrap_or_default()
        );
        div()
            .flex()
            .flex_col()
            .child(row(c, None, s.name.clone(), Some(description), controls))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_4()
                    .pb_3()
                    .child(
                        div().flex_1().child(
                            Progress::new(SharedString::from(format!("sprint-progress-{id}")))
                                .value(pct),
                        ),
                    )
                    .child(div().text_xs().text_color(c.text_muted).child(t!(
                        "projects.sprints.counts",
                        done = counts.done,
                        progress = counts.in_progress,
                        total = counts.total
                    )))
                    .when(!s.burndown.is_empty(), |d| d.child(burndown(c, s))),
            )
            .into_any_element()
    }
}

#[derive(Clone, Copy)]
enum SprintAction {
    Start,
    Delete,
    /// 未完了のタスクの移し先（None はバックログ）。
    Complete(Option<Uuid>),
}

/// 残りタスク数の小さな棒グラフ（実績の棒 + 理想線の高さを薄く重ねる）。
fn burndown(c: &Colors, s: &SprintDetail) -> Div {
    let max = s
        .burndown
        .iter()
        .map(|p| (p.actual_remaining as i64).max(p.ideal_remaining as i64))
        .max()
        .unwrap_or(0)
        .max(1) as f32;
    let height = 28.;
    div()
        .flex()
        .items_end()
        .gap(px(1.))
        .h(px(height))
        .children(s.burndown.iter().map(|p| {
            let actual = p.actual_remaining as f32 / max * height;
            let ideal = p.ideal_remaining.max(0) as f32 / max * height;
            div()
                .w(px(4.))
                .h(px(height))
                .relative()
                .child(
                    div()
                        .absolute()
                        .bottom_0()
                        .w_full()
                        .h(px(ideal))
                        .bg(c.text_muted.opacity(0.25)),
                )
                .child(
                    div()
                        .absolute()
                        .bottom_0()
                        .w_full()
                        .h(px(actual))
                        .bg(c.accent),
                )
        }))
}

#[cfg(test)]
mod tests {
    use super::parse_day;

    #[test]
    fn parses_date_and_datetime_prefix() {
        assert_eq!(
            parse_day("2026-09-24").map(|d| d.to_string()),
            Some("2026-09-24".into())
        );
        assert_eq!(
            parse_day("2026-09-24T00:00:00Z").map(|d| d.to_string()),
            Some("2026-09-24".into())
        );
        assert!(parse_day("nope").is_none());
    }
}
