//! Detail ペインの描画。ヘッダー・プロパティ・説明と、下段のタブ。

use chrono::{DateTime, Days, Local, Utc};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::calendar::Date;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::date_picker::DatePickerState;
use gpui_kit::component::date_picker::{DatePicker, DateRangePreset};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::progress::Progress;
use gpui_kit::component::tab::{Tab as KitTab, TabBar};
use gpui_kit::component::text::TextView;
use gpui_kit::component::{Disableable, Icon, Selectable, Sizable, Size, WindowExt};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use i18n::t;
use uuid::Uuid;

use super::{Load, RelationKind, Tab, TaskDetailView, next_status, select_options};
use crate::avatar::user_avatar;
use crate::markdown::hard_line_breaks;
use crate::model::{
    PRIORITIES, activity_text, format_minutes, parse_hex_color, priority_color, priority_label,
};
use crate::ui::{status_dot, status_pill};
use api::types::{CustomFieldType, TaskDetailResponse, TaskResponse};

type Colors = ::ui::Colors;

fn colors(cx: &App) -> Colors {
    ::ui::colors(cx)
}

fn local_time(at: &DateTime<Utc>) -> String {
    at.with_timezone(&Local)
        .format("%Y-%m-%d %H:%M")
        .to_string()
}

fn file_size(bytes: i64) -> String {
    match bytes {
        b if b >= 1 << 20 => format!("{:.1} MB", b as f64 / (1 << 20) as f64),
        b if b >= 1 << 10 => format!("{:.1} KB", b as f64 / (1 << 10) as f64),
        b => format!("{b} B"),
    }
}

fn section_title(text: impl Into<SharedString>) -> Div {
    div()
        .text_sm()
        .font_weight(FontWeight::SEMIBOLD)
        .child(text.into())
}

fn empty_note(c: Colors, text: impl Into<SharedString>) -> Div {
    div()
        .py_2()
        .text_sm()
        .text_color(c.text_muted)
        .child(text.into())
}

/// 期限のよく使う候補（今日 / 明日 / 1 週間後）。
fn due_presets() -> Vec<DateRangePreset> {
    let today = Local::now().date_naive();
    [
        (t!("tasks.detail.due_today"), 0),
        (t!("tasks.detail.due_tomorrow"), 1),
        (t!("tasks.detail.due_next_week"), 7),
    ]
    .into_iter()
    .filter_map(|(label, days)| {
        let date = today.checked_add_days(Days::new(days))?;
        Some(DateRangePreset::single(label, date))
    })
    .collect()
}

impl Render for TaskDetailView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let c = colors(cx);
        if self.error != self.shown_error {
            self.shown_error = self.error.clone();
            if let Some(error) = &self.error {
                window.push_notification(Notification::new().message(error.clone()), cx);
            }
        }
        if let Some(notice) = self.notice.take() {
            window.push_notification(Notification::new().message(notice), cx);
        }

        let Some(detail) = self.detail.clone() else {
            let text = if self.loading {
                t!("tasks.detail.loading").to_string()
            } else {
                self.error
                    .clone()
                    .unwrap_or_else(|| t!("tasks.detail.select_task").into())
            };
            return div()
                .id("task-detail-empty")
                .size_full()
                .flex()
                .items_center()
                .child(::ui::empty_state(
                    &c,
                    if self.error.is_some() {
                        IconName::CircleAlert
                    } else {
                        IconName::ClipboardList
                    },
                    text,
                    None,
                ))
                .into_any_element();
        };

        self.sync_inputs(&detail, window, cx);

        let header = self.render_header(&detail, c, cx);
        let title = self.render_title(cx);
        let properties = self.render_properties(&detail, c, cx);
        let description = self.render_description(&detail, c, cx);
        let tabs = self.render_tab_bar(cx);
        let body = match self.tab {
            Tab::Comments => self.render_comments(c, cx),
            Tab::Subtasks => self.render_subtasks(c, cx),
            Tab::Relations => self.render_relations(c, cx),
            Tab::Activity => self.render_activity(c, cx),
            Tab::Time => self.render_time(&detail, c, cx),
            Tab::Files => self.render_files(c, cx),
        };

        div()
            .id("task-detail")
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
                    .child(header)
                    .child(title)
                    .child(properties)
                    .child(description)
                    .when_some(self.error.clone(), |d, e| {
                        d.child(div().text_sm().text_color(c.danger).child(e))
                    })
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .pt_1()
                            .child(tabs)
                            .child(body),
                    ),
            )
            .into_any_element()
    }
}

impl TaskDetailView {
    /// 非同期の結果を入力欄へ写す（spawn からは Window に触れないため render で行う）。
    fn sync_inputs(
        &mut self,
        detail: &TaskDetailResponse,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if std::mem::take(&mut self.clear_comment_input) {
            self.comment_input
                .update(cx, |s, cx| s.set_value("", window, cx));
        }
        if std::mem::take(&mut self.clear_subtask_input) {
            self.subtask_input
                .update(cx, |s, cx| s.set_value("", window, cx));
        }
        if std::mem::take(&mut self.clear_relation_input) {
            self.relation_input
                .update(cx, |s, cx| s.set_value("", window, cx));
        }
        if std::mem::take(&mut self.clear_log_inputs) {
            let today = Local::now().date_naive();
            self.log_minutes_input
                .update(cx, |s, cx| s.set_value("", window, cx));
            self.log_note_input
                .update(cx, |s, cx| s.set_value("", window, cx));
            self.log_date_picker.update(cx, |s, cx| {
                s.set_date(Date::Single(Some(today)), window, cx)
            });
        }
        let description = detail
            .description
            .clone()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| t!("tasks.detail.no_description").into());
        self.description_view.sync(&description, window, cx);
        // 期限は保存値が変わるたび（失敗時の巻き戻しを含む）に合わせる。
        for (picker, value) in [
            (&self.due_picker, detail.soft_deadline),
            (&self.hard_due_picker, detail.hard_deadline),
        ] {
            let date = value.map(|d| d.with_timezone(&Local).date_naive());
            if picker.read(cx).date() != Date::Single(date) {
                picker.update(cx, |s, cx| s.set_date(Date::Single(date), window, cx));
            }
        }
        // テキスト系カスタムフィールドの入力欄を用意する。
        let mut created = false;
        for value in &detail.custom_field_values {
            if matches!(
                value.field.field_type,
                CustomFieldType::Select | CustomFieldType::Checkbox
            ) || self.field_inputs.contains_key(&value.field.id)
            {
                continue;
            }
            let hint = match value.field.field_type {
                CustomFieldType::Date => "YYYY-MM-DD".to_string(),
                CustomFieldType::Url => "https://…".to_string(),
                _ => t!("tasks.detail.field_placeholder").to_string(),
            };
            let input = cx.new(|cx| InputState::new(window, cx).placeholder(hint));
            let field = value.field.id;
            let sub = cx.subscribe(&input, move |this, input, event: &InputEvent, cx| {
                // Enter だけで保存する（Blur はタスク切り替え後に届き、別のタスクへ書き得る）。
                if matches!(event, InputEvent::PressEnter { .. }) {
                    let value = input.read(cx).value().trim().to_string();
                    this.set_custom_field(field, Some(value), cx);
                }
            });
            self._subs.push(sub);
            self.field_inputs.insert(field, input);
            created = true;
        }
        // 入力欄は task が変わった時（と失敗の巻き戻し時）だけ保存値に合わせる。
        if self.title_synced != Some(detail.id) || created {
            let full = self.title_synced != Some(detail.id);
            self.title_synced = Some(detail.id);
            if full {
                let title = detail.title.clone();
                self.title_input
                    .update(cx, |s, cx| s.set_value(title, window, cx));
                self.description_input.update(cx, |s, cx| {
                    s.set_value(detail.description.clone().unwrap_or_default(), window, cx)
                });
                let estimate = detail
                    .estimated_minutes
                    .map(format_minutes)
                    .unwrap_or_default();
                self.estimate_input
                    .update(cx, |s, cx| s.set_value(estimate, window, cx));
                let progress = detail.progress_pct.to_string();
                self.progress_input
                    .update(cx, |s, cx| s.set_value(progress, window, cx));
            }
            for value in &detail.custom_field_values {
                if let Some(input) = self.field_inputs.get(&value.field.id) {
                    let text = value.value.clone().unwrap_or_default();
                    input.update(cx, |s, cx| s.set_value(text, window, cx));
                }
            }
        }
    }

    #[inline(never)]
    fn render_header(
        &self,
        detail: &TaskDetailResponse,
        c: Colors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let has_done_state = self.statuses.iter().any(|s| s.is_done_state);
        let done = self.is_done();
        let watching = self.is_watching();
        let archived = detail.is_archived;
        let running = self.timer.as_ref().is_some_and(|t| t.is_running);
        let owner = cx.entity().downgrade();
        let watcher_names = self
            .watchers
            .iter()
            .map(|w| w.name.clone())
            .collect::<Vec<_>>()
            .join(", ");
        let busy = self.acting;
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(c.text_muted)
                    .child(self.task_key(detail.seq_id)),
            )
            .when(archived, |d| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .px_2()
                        .h(px(20.))
                        .rounded_full()
                        .border_1()
                        .border_color(c.border)
                        .text_xs()
                        .text_color(c.text_muted)
                        .child(Icon::new(IconName::Archive).size_3())
                        .child(t!("tasks.detail.archived_badge")),
                )
            })
            .when(running, |d| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .text_xs()
                        .text_color(c.success)
                        .child(Icon::new(IconName::Timer).size_3())
                        .child(t!("tasks.detail.timer_running")),
                )
            })
            .child(div().flex_1())
            .child(
                Button::new("watch-toggle")
                    .ghost()
                    .small()
                    .icon(if watching {
                        IconName::BellRing
                    } else {
                        IconName::Bell
                    })
                    .label(if watching {
                        t!("tasks.detail.watching")
                    } else {
                        t!("tasks.detail.watch_short")
                    })
                    .when(!self.watchers.is_empty(), |b| {
                        b.child(
                            div()
                                .text_xs()
                                .text_color(c.text_muted)
                                .child(self.watchers.len().to_string()),
                        )
                    })
                    .selected(watching)
                    .disabled(busy || self.me.is_none())
                    .tooltip(if self.watchers.is_empty() {
                        if watching {
                            t!("tasks.detail.unwatch").to_string()
                        } else {
                            t!("tasks.detail.watch").to_string()
                        }
                    } else {
                        t!(
                            "tasks.detail.watchers_tooltip",
                            action = if watching {
                                t!("tasks.detail.unwatch")
                            } else {
                                t!("tasks.detail.watch")
                            },
                            names = watcher_names
                        )
                    })
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_watch(cx))),
            )
            .when(has_done_state, |d| {
                d.child(
                    Button::new("done-toggle")
                        .small()
                        .disabled(self.updating)
                        .when(done, |b| b.outline().label(t!("tasks.detail.reopen")))
                        .when(!done, |b| b.primary().label(t!("tasks.detail.mark_done")))
                        .icon(IconName::Check)
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_done(cx))),
                )
            })
            .child(
                Button::new("task-actions")
                    .ghost()
                    .small()
                    .icon(IconName::Ellipsis)
                    .disabled(busy)
                    .tooltip(t!("tasks.detail.actions"))
                    .dropdown_menu(move |menu, _, _| {
                        let (o1, o2, o3) = (owner.clone(), owner.clone(), owner.clone());
                        menu.item(
                            PopupMenuItem::new(if watching {
                                t!("tasks.detail.unwatch")
                            } else {
                                t!("tasks.detail.watch")
                            })
                            .icon(if watching {
                                IconName::EyeOff
                            } else {
                                IconName::Eye
                            })
                            .on_click(move |_, _, cx| {
                                let _ = o1.update(cx, |this, cx| this.toggle_watch(cx));
                            }),
                        )
                        .item(
                            PopupMenuItem::new(if archived {
                                t!("tasks.detail.unarchive")
                            } else {
                                t!("tasks.detail.archive")
                            })
                            .icon(if archived {
                                IconName::ArchiveRestore
                            } else {
                                IconName::Archive
                            })
                            .on_click(move |_, _, cx| {
                                let _ = o2.update(cx, |this, cx| this.toggle_archive(cx));
                            }),
                        )
                        .separator()
                        .item(
                            PopupMenuItem::new(t!("tasks.detail.delete"))
                                .icon(IconName::Trash)
                                .on_click(move |_, window, cx| {
                                    let _ = o3.update(cx, |this, cx| this.delete_task(window, cx));
                                }),
                        )
                    }),
            )
            .into_any_element()
    }

    #[inline(never)]
    fn render_title(&self, cx: &mut Context<Self>) -> AnyElement {
        let dirty = self
            .detail
            .as_ref()
            .is_some_and(|d| self.title_input.read(cx).value().trim() != d.title.trim());
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .ml(px(-12.))
                    .text_lg()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(
                        Input::new(&self.title_input)
                            .id("task-title")
                            .appearance(false)
                            .large()
                            .disabled(self.updating),
                    ),
            )
            .when(dirty, |d| {
                d.child(
                    Button::new("save-title")
                        .disabled(self.updating)
                        .primary()
                        .small()
                        .icon(IconName::Check)
                        .label(t!("tasks.detail.save"))
                        .tooltip(t!("tasks.detail.save_title"))
                        .on_click(cx.listener(|this, _, _, cx| this.save_title(cx))),
                )
            })
            .into_any_element()
    }

    #[inline(never)]
    fn render_properties(
        &self,
        detail: &TaskDetailResponse,
        c: Colors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let status = self.render_status_control(detail, c, cx);
        let priority = self.render_priority_control(detail, cx);
        let assignees = self.render_assignee_control(detail, cx);
        let labels = self.render_label_control(detail, c, cx);
        let milestone = self.render_milestone_control(detail, cx);
        let sprint = self.render_sprint_control(detail, cx);
        let parent = self.render_parent(detail, c, cx);
        let fields = self.render_custom_fields(detail, c, cx);
        let date = |picker: &Entity<DatePickerState>| {
            div().w(px(156.)).child(
                DatePicker::new(picker)
                    .small()
                    .appearance(false)
                    .placeholder(t!("tasks.detail.due_placeholder"))
                    .cleanable(true)
                    .presets(due_presets())
                    .disabled(self.updating),
            )
        };
        let estimate = div().w(px(120.)).child(
            Input::new(&self.estimate_input)
                .id("task-estimate")
                .small()
                .disabled(self.updating),
        );
        let progress = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                div().w(px(64.)).child(
                    Input::new(&self.progress_input)
                        .id("task-progress")
                        .small()
                        .suffix(div().text_xs().text_color(c.text_muted).child("%"))
                        .disabled(self.updating),
                ),
            )
            .child(
                div()
                    .w(px(96.))
                    .child(Progress::new("task-progress-bar").value(detail.progress_pct as f32)),
            );
        let meta = {
            let mut parts = vec![];
            if let Some(user) = &detail.created_by {
                parts.push(t!("tasks.detail.created_by", name = user.username));
            }
            parts.push(t!(
                "tasks.detail.created_at",
                at = local_time(&detail.created_at)
            ));
            parts.push(t!(
                "tasks.detail.updated_at",
                at = local_time(&detail.updated_at)
            ));
            if let Some(done) = &detail.completed_at {
                parts.push(t!("tasks.detail.completed_at", at = local_time(done)));
            }
            parts.join(" · ")
        };

        // 値のある副次プロパティは常に出し、空のものは「他のプロパティ」でまとめて出し入れする。
        let all = self.show_all_props;
        let has_fields = !fields.is_empty();
        let secondary: Vec<(bool, AnyElement)> = vec![
            (
                detail.hard_deadline.is_some(),
                ::ui::property(
                    &c,
                    IconName::CalendarClock,
                    t!("tasks.detail.hard_due"),
                    date(&self.hard_due_picker),
                )
                .into_any_element(),
            ),
            (
                detail.milestone_id.is_some(),
                ::ui::property(
                    &c,
                    IconName::Milestone,
                    t!("tasks.detail.milestone"),
                    milestone,
                )
                .into_any_element(),
            ),
            (
                detail.sprint_id.is_some(),
                ::ui::property(&c, IconName::Zap, t!("tasks.detail.sprint"), sprint)
                    .into_any_element(),
            ),
            (
                detail.estimated_minutes.is_some(),
                ::ui::property(&c, IconName::Clock, t!("tasks.detail.estimate"), estimate)
                    .into_any_element(),
            ),
            (
                detail.progress_pct > 0,
                ::ui::property(&c, IconName::Gauge, t!("tasks.detail.progress"), progress)
                    .into_any_element(),
            ),
        ];
        let hidden = secondary.iter().filter(|(set, _)| !set).count();
        div()
            .flex()
            .flex_col()
            .child(::ui::property(
                &c,
                IconName::CircleDot,
                t!("tasks.detail.status"),
                status,
            ))
            .child(::ui::property(
                &c,
                IconName::Flag,
                t!("tasks.detail.priority"),
                priority,
            ))
            .child(::ui::property(
                &c,
                IconName::User,
                t!("tasks.detail.assignees"),
                assignees,
            ))
            .child(::ui::property(
                &c,
                IconName::Calendar,
                t!("tasks.detail.due"),
                date(&self.due_picker),
            ))
            .child(::ui::property(
                &c,
                IconName::Tag,
                t!("tasks.detail.labels"),
                labels,
            ))
            .children(parent)
            .children(
                secondary
                    .into_iter()
                    .filter(|(set, _)| all || *set)
                    .map(|(_, row)| row),
            )
            .when(all || has_fields, |d| d.children(fields))
            .when(all, |d| {
                d.child(div().pt_1().text_xs().text_color(c.text_muted).child(meta))
            })
            .child(
                div().flex().pt_1().child(
                    Button::new("toggle-props")
                        .ghost()
                        .xsmall()
                        .text_color(c.text_muted)
                        .icon(if all {
                            IconName::ChevronUp
                        } else {
                            IconName::ChevronDown
                        })
                        .label(if all {
                            t!("tasks.detail.fewer_props").to_string()
                        } else if hidden > 0 {
                            t!("tasks.detail.more_props", count = hidden)
                        } else {
                            t!("tasks.detail.show_meta").to_string()
                        })
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.show_all_props = !this.show_all_props;
                            cx.notify();
                        })),
                ),
            )
            .into_any_element()
    }

    #[inline(never)]
    fn render_status_control(
        &self,
        detail: &TaskDetailResponse,
        c: Colors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let current = self
            .statuses
            .iter()
            .find(|s| s.id == detail.status_id)
            .cloned();
        let statuses = self.statuses.clone();
        let cur = detail.status_id;
        let owner = cx.entity().downgrade();
        let button = Button::new("status-select")
            .ghost()
            .small()
            .disabled(self.updating || self.statuses.is_empty())
            .child(match &current {
                Some(s) => status_pill(&s.name, parse_hex_color(&s.color).unwrap_or(c.text_muted)),
                None => div().child("—"),
            })
            .dropdown_menu(move |mut menu, _, _| {
                for s in &statuses {
                    let (id, owner) = (s.id, owner.clone());
                    let color = parse_hex_color(&s.color).unwrap_or(c.text_muted);
                    let name = s.name.clone();
                    menu = menu.item(
                        PopupMenuItem::element(move |_, _| status_pill(&name, color))
                            .checked(cur == id)
                            .on_click(move |_, _, cx| {
                                let _ = owner.update(cx, |this, cx| this.set_status(id, cx));
                            }),
                    );
                }
                menu
            });
        // ワークフロー順で 1 つ先のステータスへ進める（Web の「次へ」と同じ）。最後なら出さない。
        let next = next_status(&self.statuses, cur);
        div()
            .flex()
            .items_center()
            .gap_1()
            .child(button)
            .when_some(next, |d, next| {
                d.child(
                    Button::new("status-next")
                        .ghost()
                        .xsmall()
                        .text_color(c.text_muted)
                        .icon(IconName::ArrowRight)
                        .disabled(self.updating)
                        .tooltip(t!("tasks.detail.next_status", name = next.name))
                        .on_click(cx.listener(move |this, _, _, cx| this.set_status(next.id, cx))),
                )
            })
            .into_any_element()
    }

    #[inline(never)]
    fn render_priority_control(
        &self,
        detail: &TaskDetailResponse,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let cur = detail.priority;
        let owner = cx.entity().downgrade();
        Button::new("priority-select")
            .ghost()
            .small()
            .disabled(self.updating)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .text_color(priority_color(cur))
                    .child(Icon::new(IconName::Flag).small())
                    .child(priority_label(cur)),
            )
            .dropdown_menu(move |mut menu, _, _| {
                for p in PRIORITIES {
                    let owner = owner.clone();
                    menu = menu.item(
                        PopupMenuItem::new(priority_label(p))
                            .checked(cur == p)
                            .on_click(move |_, _, cx| {
                                let _ = owner.update(cx, |this, cx| this.set_priority(p, cx));
                            }),
                    );
                }
                menu
            })
            .into_any_element()
    }

    #[inline(never)]
    fn render_assignee_control(
        &self,
        detail: &TaskDetailResponse,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let ids: Vec<Uuid> = detail.assignees.iter().map(|a| a.user.id).collect();
        let label = if detail.assignees.is_empty() {
            t!("tasks.detail.unassigned").to_string()
        } else {
            detail
                .assignees
                .iter()
                .map(|a| a.user.username.clone())
                .collect::<Vec<_>>()
                .join(", ")
        };
        let avatars: Vec<_> = detail
            .assignees
            .iter()
            .map(|a| {
                user_avatar(
                    &a.user.username,
                    a.user.avatar_url.as_deref(),
                    Size::Small,
                    cx,
                )
            })
            .collect();
        let assignables = self.assignables.clone();
        let owner = cx.entity().downgrade();
        let empty = avatars.is_empty();
        let muted = ::ui::colors(cx).text_muted;
        Button::new("assignee-select")
            .ghost()
            .small()
            .disabled(self.updating || self.assignables.is_empty())
            .when(empty, |b| b.text_color(muted))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .when(!avatars.is_empty(), |d| {
                        // 複数人は少し重ねて並べる。
                        d.child(div().flex().items_center().children(
                            avatars.into_iter().enumerate().map(|(ix, avatar)| {
                                div().when(ix > 0, |d| d.ml(px(-6.))).child(avatar)
                            }),
                        ))
                    })
                    .child(label),
            )
            .dropdown_menu(move |mut menu, _, _| {
                for u in &assignables {
                    let (user, owner) = (u.clone(), owner.clone());
                    let (name, url) = (u.username.clone(), u.avatar_url.clone());
                    menu = menu.item(
                        PopupMenuItem::element(move |_, cx| {
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(user_avatar(&name, url.as_deref(), Size::Small, cx))
                                .child(name.clone())
                        })
                        .checked(ids.contains(&u.id))
                        .on_click(move |_, _, cx| {
                            let _ = owner.update(cx, |this, cx| this.toggle_assignee(&user, cx));
                        }),
                    );
                }
                menu
            })
            .into_any_element()
    }

    #[inline(never)]
    fn render_label_control(
        &self,
        detail: &TaskDetailResponse,
        c: Colors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let chips = detail.labels.iter().map(|label| {
            let color = parse_hex_color(&label.color).unwrap_or(c.text_muted);
            div()
                .flex()
                .flex_shrink_0()
                .items_center()
                .gap_1()
                .px_2()
                .h(px(22.))
                .rounded_full()
                .border_1()
                .border_color(c.border)
                .text_xs()
                .child(div().size(px(8.)).rounded_full().bg(color))
                .child(label.name.clone())
        });
        let all = self.labels.clone();
        let has: Vec<Uuid> = detail.labels.iter().map(|l| l.id).collect();
        let owner = cx.entity().downgrade();
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_1()
            .children(chips)
            .child(
                Button::new("label-select")
                    .ghost()
                    .xsmall()
                    .icon(IconName::Tag)
                    .tooltip(t!("tasks.detail.edit_labels"))
                    .disabled(self.updating)
                    .dropdown_menu(move |mut menu, _, _| {
                        if all.is_empty() {
                            return menu.item(
                                PopupMenuItem::new(t!("tasks.list.labels_empty")).disabled(true),
                            );
                        }
                        for label in &all {
                            let (label, owner) = (label.clone(), owner.clone());
                            let color = parse_hex_color(&label.color).unwrap_or(c.text_muted);
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
                                .checked(has.contains(&label.id))
                                .on_click(move |_, _, cx| {
                                    let label = label.clone();
                                    let _ =
                                        owner.update(cx, |this, cx| this.toggle_label(label, cx));
                                }),
                            );
                        }
                        menu
                    }),
            )
            .into_any_element()
    }

    #[inline(never)]
    fn render_milestone_control(
        &self,
        detail: &TaskDetailResponse,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let cur = detail.milestone_id;
        let items: Vec<(Uuid, String)> = self
            .milestones
            .iter()
            .map(|m| (m.id, format!("{} · {}", m.name, m.due_date)))
            .collect();
        let label = cur
            .and_then(|id| items.iter().find(|(m, _)| *m == id))
            .map(|(_, name)| name.clone())
            .unwrap_or_else(|| t!("tasks.detail.none").into());
        let owner = cx.entity().downgrade();
        let muted = ::ui::colors(cx).text_muted;
        Button::new("milestone-select")
            .ghost()
            .small()
            .when(cur.is_none(), |b| b.text_color(muted))
            .label(label)
            .disabled(self.updating || (items.is_empty() && cur.is_none()))
            .dropdown_menu(move |mut menu, _, _| {
                let none_owner = owner.clone();
                menu = menu.item(
                    PopupMenuItem::new(t!("tasks.detail.none"))
                        .checked(cur.is_none())
                        .on_click(move |_, _, cx| {
                            let _ = none_owner.update(cx, |this, cx| this.set_milestone(None, cx));
                        }),
                );
                for (id, name) in &items {
                    let (id, owner) = (*id, owner.clone());
                    menu = menu.item(
                        PopupMenuItem::new(name.clone())
                            .checked(cur == Some(id))
                            .on_click(move |_, _, cx| {
                                let _ =
                                    owner.update(cx, |this, cx| this.set_milestone(Some(id), cx));
                            }),
                    );
                }
                menu
            })
            .into_any_element()
    }

    #[inline(never)]
    fn render_sprint_control(
        &self,
        detail: &TaskDetailResponse,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let cur = detail.sprint_id;
        let items: Vec<(Uuid, String, bool)> = self
            .sprints
            .iter()
            .map(|s| {
                let status = match s.status {
                    api::types::SprintStatus::Planning => t!("tasks.sprint.planning"),
                    api::types::SprintStatus::Active => t!("tasks.sprint.active"),
                    api::types::SprintStatus::Completed => t!("tasks.sprint.completed"),
                };
                (
                    s.id,
                    format!("{} ({status})", s.name),
                    s.status == api::types::SprintStatus::Completed,
                )
            })
            .collect();
        let label = cur
            .and_then(|id| items.iter().find(|(s, _, _)| *s == id))
            .map(|(_, name, _)| name.clone())
            .unwrap_or_else(|| t!("tasks.detail.none").into());
        let owner = cx.entity().downgrade();
        let muted = ::ui::colors(cx).text_muted;
        Button::new("sprint-select")
            .ghost()
            .small()
            .when(cur.is_none(), |b| b.text_color(muted))
            .label(label)
            .disabled(self.updating || (items.is_empty() && cur.is_none()))
            .dropdown_menu(move |mut menu, _, _| {
                let none_owner = owner.clone();
                menu = menu.item(
                    PopupMenuItem::new(t!("tasks.detail.none"))
                        .checked(cur.is_none())
                        .on_click(move |_, _, cx| {
                            let _ = none_owner.update(cx, |this, cx| this.set_sprint(None, cx));
                        }),
                );
                for (id, name, completed) in &items {
                    let (id, owner) = (*id, owner.clone());
                    menu = menu.item(
                        PopupMenuItem::new(name.clone())
                            .checked(cur == Some(id))
                            // 完了したスプリントには入れない（backend も拒否する）。
                            .disabled(*completed && cur != Some(id))
                            .on_click(move |_, _, cx| {
                                let _ = owner.update(cx, |this, cx| this.set_sprint(Some(id), cx));
                            }),
                    );
                }
                menu
            })
            .into_any_element()
    }

    #[inline(never)]
    fn render_parent(
        &self,
        detail: &TaskDetailResponse,
        c: Colors,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let parent = detail.parent_task_id?;
        let label = self
            .relations
            .as_ref()
            .and_then(|r| r.parent.as_ref())
            .map(|p| format!("{} {}", self.task_key(p.seq_id), p.title))
            .unwrap_or_else(|| t!("tasks.detail.parent_task").into());
        Some(
            ::ui::property(
                &c,
                IconName::CornerLeftUp,
                t!("tasks.detail.parent"),
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .min_w_0()
                    .child(
                        Button::new("open-parent")
                            .ghost()
                            .small()
                            .label(label)
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.open_task(parent, cx)),
                            ),
                    )
                    .child(
                        Button::new("clear-parent")
                            .ghost()
                            .xsmall()
                            .icon(IconName::X)
                            .tooltip(t!("tasks.detail.clear_parent"))
                            .disabled(self.updating)
                            .on_click(cx.listener(|this, _, _, cx| this.clear_parent(cx))),
                    ),
            )
            .into_any_element(),
        )
    }

    #[inline(never)]
    fn render_custom_fields(
        &self,
        detail: &TaskDetailResponse,
        c: Colors,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let mut values = detail.custom_field_values.clone();
        values.sort_by_key(|v| v.field.position);
        values
            .into_iter()
            .map(|value| {
                let field = value.field.id;
                let name = if value.field.is_required {
                    format!("{} *", value.field.name)
                } else {
                    value.field.name.clone()
                };
                let control = match value.field.field_type {
                    CustomFieldType::Checkbox => {
                        Checkbox::new(SharedString::from(format!("field-{field}")))
                            .checked(value.value.as_deref() == Some("true"))
                            .disabled(self.updating)
                            .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                                this.set_custom_field(field, Some(checked.to_string()), cx)
                            }))
                            .into_any_element()
                    }
                    CustomFieldType::Select => {
                        let options = self
                            .custom_fields
                            .iter()
                            .find(|f| f.id == field)
                            .map(select_options)
                            .unwrap_or_default();
                        let current = value.value.clone();
                        let owner = cx.entity().downgrade();
                        Button::new(SharedString::from(format!("field-{field}")))
                            .ghost()
                            .small()
                            .disabled(self.updating)
                            .label(
                                value
                                    .display_value
                                    .clone()
                                    .or(value.value.clone())
                                    .unwrap_or_else(|| t!("tasks.detail.none").into()),
                            )
                            .dropdown_menu(move |mut menu, _, _| {
                                let none_owner = owner.clone();
                                menu = menu.item(
                                    PopupMenuItem::new(t!("tasks.detail.none"))
                                        .checked(current.is_none())
                                        .on_click(move |_, _, cx| {
                                            let _ = none_owner.update(cx, |this, cx| {
                                                this.set_custom_field(field, None, cx)
                                            });
                                        }),
                                );
                                for (val, label) in &options {
                                    let (val, owner) = (val.clone(), owner.clone());
                                    menu = menu.item(
                                        PopupMenuItem::new(label.clone())
                                            .checked(current.as_deref() == Some(val.as_str()))
                                            .on_click(move |_, _, cx| {
                                                let val = val.clone();
                                                let _ = owner.update(cx, |this, cx| {
                                                    this.set_custom_field(field, Some(val), cx)
                                                });
                                            }),
                                    );
                                }
                                menu
                            })
                            .into_any_element()
                    }
                    _ => match self.field_inputs.get(&field) {
                        Some(input) => div()
                            .w(px(200.))
                            .child(
                                Input::new(input)
                                    .id(SharedString::from(format!("field-{field}")))
                                    .small()
                                    .disabled(self.updating),
                            )
                            .into_any_element(),
                        None => div()
                            .text_sm()
                            .child(value.display_value.clone().unwrap_or_default())
                            .into_any_element(),
                    },
                };
                ::ui::property(&c, IconName::SlidersHorizontal, name, control).into_any_element()
            })
            .collect()
    }

    #[inline(never)]
    fn render_description(
        &self,
        _detail: &TaskDetailResponse,
        c: Colors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap_2()
            .pt_4()
            .border_t_1()
            .border_color(c.border)
            .child(
                ::ui::section_heading(&c, t!("tasks.detail.description")).child(
                    Button::new("edit-description")
                        .xsmall()
                        .ghost()
                        .icon(if self.editing_description {
                            IconName::X
                        } else {
                            IconName::Pencil
                        })
                        .label(if self.editing_description {
                            t!("tasks.detail.cancel")
                        } else {
                            t!("tasks.detail.edit")
                        })
                        .on_click(cx.listener(|this, _, window, cx| {
                            if this.editing_description {
                                let description = this
                                    .detail
                                    .as_ref()
                                    .and_then(|d| d.description.clone())
                                    .unwrap_or_default();
                                this.description_input.update(cx, |input, cx| {
                                    input.set_value(description, window, cx)
                                });
                            }
                            this.editing_description = !this.editing_description;
                            cx.notify();
                        })),
                ),
            )
            .when(self.editing_description, |d| {
                d.child(Textarea::new(&self.description_input).h(px(160.)))
                    .child(
                        Button::new("save-description")
                            .primary()
                            .small()
                            .disabled(self.updating)
                            .label(t!("tasks.detail.save_description"))
                            .on_click(cx.listener(|this, _, _, cx| this.save_description(cx))),
                    )
            })
            .when(!self.editing_description, |d| {
                d.child(self.description_view.render("task-description", cx))
            })
            .into_any_element()
    }

    #[inline(never)]
    fn render_tab_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let count = |n: usize| {
            if n > 0 {
                format!(" {n}")
            } else {
                String::new()
            }
        };
        let comments: usize = self.comments.iter().map(|c| 1 + c.replies.len()).sum();
        let subtasks = self.relations.as_ref().map_or(0, |r| r.subtasks.len());
        let relations = self
            .relations
            .as_ref()
            .map_or(0, |r| r.blocks.len() + r.blocked_by.len());
        let labels = Tab::ALL.map(|tab| match tab {
            Tab::Comments => format!("{}{}", t!("tasks.detail.tab_comments"), count(comments)),
            Tab::Subtasks => format!("{}{}", t!("tasks.detail.tab_subtasks"), count(subtasks)),
            Tab::Relations => format!("{}{}", t!("tasks.detail.tab_relations"), count(relations)),
            Tab::Activity => t!("tasks.detail.tab_activity").to_string(),
            Tab::Time => match &self.time_summary {
                Some(s) if s.actual_minutes > 0 => format!(
                    "{} {}",
                    t!("tasks.detail.tab_time"),
                    format_minutes(s.actual_minutes)
                ),
                _ => t!("tasks.detail.tab_time").to_string(),
            },
            Tab::Files => format!(
                "{}{}",
                t!("tasks.detail.tab_files"),
                count(self.attachments.len())
            ),
        });
        let selected = Tab::ALL.iter().position(|t| *t == self.tab).unwrap_or(0);
        TabBar::new("task-detail-tabs")
            .underline()
            .small()
            .selected_index(selected)
            .menu(true)
            .children(labels.into_iter().map(|label| KitTab::new().label(label)))
            .on_click(cx.listener(|this, ix: &usize, _, cx| {
                if let Some(tab) = Tab::ALL.get(*ix) {
                    this.select_tab(*tab, cx);
                }
            }))
            .into_any_element()
    }

    // ---- コメント ----

    #[inline(never)]
    fn render_comments(&self, c: Colors, cx: &mut Context<Self>) -> AnyElement {
        let mut list = div().flex().flex_col().gap_3();
        if self.comments.is_empty() {
            list = list.child(empty_note(c, t!("tasks.detail.no_comments")));
        }
        for thread in &self.comments {
            list = list.child(self.render_comment(
                thread.id,
                thread.id,
                &thread.user,
                thread.body.as_deref(),
                thread.is_deleted,
                &thread.created_at,
                &thread.updated_at,
                false,
                c,
                cx,
            ));
            for reply in &thread.replies {
                list = list.child(
                    div()
                        .ml_6()
                        .pl_3()
                        .border_l_1()
                        .border_color(c.border)
                        .child(self.render_comment(
                            reply.id,
                            thread.id,
                            &reply.user,
                            reply.body.as_deref(),
                            reply.is_deleted,
                            &reply.created_at,
                            &reply.updated_at,
                            true,
                            c,
                            cx,
                        )),
                );
            }
        }
        let has_text = !self.comment_input.read(cx).value().trim().is_empty();
        let composer = div()
            .flex()
            .flex_col()
            .gap_2()
            .pt_2()
            .when_some(self.reply_to.clone(), |d, (_, name)| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_xs()
                        .text_color(c.text_muted)
                        .child(Icon::new(IconName::Reply).size_3())
                        .child(t!("tasks.detail.replying_to", name = name))
                        .child(
                            Button::new("cancel-reply")
                                .ghost()
                                .xsmall()
                                .icon(IconName::X)
                                .tooltip(t!("tasks.detail.cancel"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.reply_to = None;
                                    cx.notify();
                                })),
                        ),
                )
            })
            .child(
                Textarea::new(&self.comment_input)
                    .h(px(80.))
                    .disabled(self.posting_comment),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_xs()
                            .text_color(c.text_muted)
                            .child(t!("tasks.detail.comment_hint")),
                    )
                    .child(
                        Button::new("post-comment")
                            .primary()
                            .small()
                            .icon(IconName::Send)
                            .label(if self.posting_comment {
                                t!("tasks.detail.posting")
                            } else {
                                t!("tasks.detail.post")
                            })
                            .disabled(self.posting_comment || !has_text)
                            .on_click(cx.listener(|this, _, _, cx| this.post_comment(cx))),
                    ),
            );
        list.child(composer).into_any_element()
    }

    #[allow(clippy::too_many_arguments)] // スレッドの親と返信で同じ描画を使う
    #[inline(never)]
    fn render_comment(
        &self,
        id: Uuid,
        thread: Uuid,
        user: &api::types::CommentUser,
        body: Option<&str>,
        deleted: bool,
        created_at: &DateTime<Utc>,
        updated_at: &DateTime<Utc>,
        reply: bool,
        c: Colors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let own = self.me == Some(user.id);
        let edited = (*updated_at - *created_at).num_seconds() > 1 && !deleted;
        let editing = self.editing_comment == Some(id);
        let avatar = user_avatar(
            &user.name,
            user.avatar_url.as_deref(),
            if reply { Size::XSmall } else { Size::Small },
            cx,
        );
        let name = user.name.clone();
        let text = body.unwrap_or_default().to_string();
        let actions = (!deleted && !editing).then(|| {
            let reply_name = name.clone();
            let edit_text = text.clone();
            div()
                .flex()
                .items_center()
                .gap_0p5()
                .child(
                    Button::new(SharedString::from(format!("reply-{id}")))
                        .ghost()
                        .xsmall()
                        .icon(IconName::Reply)
                        .tooltip(t!("tasks.detail.reply"))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.start_reply(thread, reply_name.clone(), window, cx)
                        })),
                )
                .when(own, |d| {
                    d.child(
                        Button::new(SharedString::from(format!("edit-comment-{id}")))
                            .ghost()
                            .xsmall()
                            .icon(IconName::Pencil)
                            .tooltip(t!("tasks.detail.edit"))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.start_comment_edit(id, edit_text.clone(), window, cx)
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("delete-comment-{id}")))
                            .ghost()
                            .xsmall()
                            .icon(IconName::Trash)
                            .tooltip(t!("tasks.detail.delete"))
                            .disabled(self.acting)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.delete_comment(id, window, cx)
                            })),
                    )
                })
        });
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(avatar)
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(name),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(c.text_muted)
                            .child(local_time(created_at)),
                    )
                    .when(edited, |d| {
                        d.child(
                            div()
                                .text_xs()
                                .text_color(c.text_muted)
                                .child(t!("tasks.detail.edited")),
                        )
                    })
                    .child(div().flex_1())
                    .children(actions),
            )
            .map(|d| {
                if deleted {
                    d.child(
                        div()
                            .text_sm()
                            .italic()
                            .text_color(c.text_muted)
                            .child(t!("tasks.detail.deleted_comment")),
                    )
                } else if editing {
                    d.child(Textarea::new(&self.edit_input).h(px(80.))).child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                Button::new("save-comment-edit")
                                    .primary()
                                    .small()
                                    .label(t!("tasks.detail.save"))
                                    .disabled(self.acting)
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.save_comment_edit(cx)),
                                    ),
                            )
                            .child(
                                Button::new("cancel-comment-edit")
                                    .ghost()
                                    .small()
                                    .label(t!("tasks.detail.cancel"))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.editing_comment = None;
                                        cx.notify();
                                    })),
                            ),
                    )
                } else {
                    d.child(
                        TextView::markdown(
                            SharedString::from(format!("comment-{id}")),
                            hard_line_breaks(&text),
                        )
                        .w_full()
                        .text_sm(),
                    )
                }
            })
            .into_any_element()
    }

    // ---- サブタスク ----

    fn task_row(
        &self,
        id: &'static str,
        task: &TaskResponse,
        c: Colors,
        trailing: Option<AnyElement>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let status = self.statuses.iter().find(|s| s.id == task.status_id);
        let done = status.is_some_and(|s| s.is_done_state);
        let color = status
            .and_then(|s| parse_hex_color(&s.color))
            .unwrap_or(c.text_muted);
        let target = task.id;
        div()
            .id(SharedString::from(format!("{id}-{}", task.id)))
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .h(px(32.))
            .rounded_md()
            .cursor_pointer()
            .hover(|d| d.bg(c.hover))
            .on_click(cx.listener(move |this, _, _, cx| this.open_task(target, cx)))
            .child(status_dot(color))
            .child(
                div()
                    .flex_shrink_0()
                    .text_xs()
                    .text_color(c.text_muted)
                    .child(self.task_key(task.seq_id)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .text_ellipsis()
                    .when(done, |d| d.line_through().text_color(c.text_muted))
                    .child(task.title.clone()),
            )
            .children(status.map(|s| {
                div()
                    .flex_shrink_0()
                    .text_xs()
                    .text_color(c.text_muted)
                    .child(s.name.clone())
            }))
            .children(trailing)
            .into_any_element()
    }

    #[inline(never)]
    fn render_subtasks(&self, c: Colors, cx: &mut Context<Self>) -> AnyElement {
        let subtasks = self
            .relations
            .as_ref()
            .map(|r| r.subtasks.clone())
            .unwrap_or_default();
        let done = subtasks
            .iter()
            .filter(|t| {
                self.statuses
                    .iter()
                    .any(|s| s.id == t.status_id && s.is_done_state)
            })
            .count();
        let mut list = div().flex().flex_col().gap_0p5();
        if self.relations.is_none() && self.relations_failed {
            list = list.child(self.retry_note(c, Tab::Subtasks, cx));
        } else if self.relations.is_none() {
            list = list.child(empty_note(c, t!("tasks.detail.loading")));
        } else if subtasks.is_empty() {
            list = list.child(empty_note(c, t!("tasks.list.subtasks_empty")));
        }
        for task in &subtasks {
            list = list.child(self.task_row("subtask", task, c, None, cx));
        }
        div()
            .flex()
            .flex_col()
            .gap_2()
            .when(!subtasks.is_empty(), |d| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_xs()
                        .text_color(c.text_muted)
                        .child(t!(
                            "tasks.detail.subtasks_done",
                            done = done,
                            total = subtasks.len()
                        ))
                        .child(
                            div().w(px(120.)).child(
                                Progress::new("subtask-progress")
                                    .value(done as f32 * 100. / subtasks.len() as f32),
                            ),
                        ),
                )
            })
            .child(list)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(Icon::new(IconName::Plus).size_4().text_color(c.text_muted))
                    .child(
                        div().flex_1().min_w_0().child(
                            Input::new(&self.subtask_input)
                                .id("subtask-title")
                                .disabled(self.acting),
                        ),
                    )
                    .child(
                        Button::new("add-subtask")
                            .small()
                            .label(t!("tasks.list.add"))
                            .disabled(self.acting)
                            .on_click(cx.listener(|this, _, _, cx| this.create_subtask(cx))),
                    ),
            )
            .into_any_element()
    }

    // ---- 依存関係 ----

    #[inline(never)]
    fn render_relations(&self, c: Colors, cx: &mut Context<Self>) -> AnyElement {
        let Some(relations) = self.relations.clone() else {
            if self.relations_failed {
                return self.retry_note(c, Tab::Relations, cx);
            }
            return empty_note(c, t!("tasks.detail.loading")).into_any_element();
        };
        let mut body = div().flex().flex_col().gap_3();
        for (title, entries) in [
            (t!("tasks.detail.blocked_by"), &relations.blocked_by),
            (t!("tasks.detail.blocks"), &relations.blocks),
        ] {
            let mut group = div().flex().flex_col().gap_0p5().child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(c.text_muted)
                    .child(format!("{title} ({})", entries.len())),
            );
            if entries.is_empty() {
                group = group.child(empty_note(c, t!("tasks.detail.none")));
            }
            for entry in entries {
                // RelationEntry は TaskResponse + relation_id。行の描画を共有する。
                let Ok(task) =
                    serde_json::to_value(entry).and_then(serde_json::from_value::<TaskResponse>)
                else {
                    continue;
                };
                let relation = entry.relation_id;
                let remove = Button::new(SharedString::from(format!("remove-relation-{relation}")))
                    .ghost()
                    .xsmall()
                    .icon(IconName::X)
                    .tooltip(t!("tasks.detail.remove_relation"))
                    .disabled(self.acting)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.remove_relation(relation, cx)
                    }))
                    .into_any_element();
                group = group.child(self.task_row("relation", &task, c, Some(remove), cx));
            }
            body = body.child(group);
        }
        let kind = self.relation_kind;
        let owner = cx.entity().downgrade();
        let kind_label = match kind {
            RelationKind::Blocks => t!("tasks.detail.kind_blocks"),
            RelationKind::BlockedBy => t!("tasks.detail.kind_blocked_by"),
        };
        let hits = self.relation_hits.iter().map(|hit| {
            let target = hit.id;
            div()
                .id(SharedString::from(format!("relation-hit-{}", hit.id)))
                .flex()
                .items_center()
                .gap_2()
                .px_2()
                .h(px(30.))
                .rounded_md()
                .cursor_pointer()
                .hover(|d| d.bg(c.hover))
                .on_click(cx.listener(move |this, _, _, cx| this.add_relation(target, cx)))
                .child(Icon::new(IconName::Plus).size_3().text_color(c.text_muted))
                .child(
                    div()
                        .text_xs()
                        .text_color(c.text_muted)
                        .child(self.task_key(hit.seq_id)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_sm()
                        .text_ellipsis()
                        .child(hit.title.clone()),
                )
        });
        body.child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .pt_2()
                .border_t_1()
                .border_color(c.border)
                .child(section_title(t!("tasks.detail.add_relation")))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            Button::new("relation-kind")
                                .outline()
                                .small()
                                .dropdown_caret(true)
                                .label(kind_label)
                                .dropdown_menu(move |mut menu, _, _| {
                                    for (k, label) in [
                                        (
                                            RelationKind::BlockedBy,
                                            t!("tasks.detail.kind_blocked_by"),
                                        ),
                                        (RelationKind::Blocks, t!("tasks.detail.kind_blocks")),
                                    ] {
                                        let owner = owner.clone();
                                        menu = menu.item(
                                            PopupMenuItem::new(label).checked(kind == k).on_click(
                                                move |_, _, cx| {
                                                    let _ = owner.update(cx, |this, cx| {
                                                        this.relation_kind = k;
                                                        cx.notify();
                                                    });
                                                },
                                            ),
                                        );
                                    }
                                    menu
                                }),
                        )
                        .child(
                            div().flex_1().min_w_0().child(
                                Input::new(&self.relation_input)
                                    .id("relation-search")
                                    .disabled(self.acting),
                            ),
                        ),
                )
                .children(hits),
        )
        .into_any_element()
    }

    // ---- 履歴 ----

    #[inline(never)]
    fn render_activity(&self, c: Colors, cx: &mut Context<Self>) -> AnyElement {
        let mut list = div().flex().flex_col().gap_2();
        match self.activity_state {
            Load::Loading if self.activities.is_empty() => {
                list = list.child(empty_note(c, t!("tasks.detail.loading")))
            }
            Load::Failed if self.activities.is_empty() => {
                list = list.child(self.retry_note(c, Tab::Activity, cx))
            }
            _ if self.activities.is_empty() => {
                list = list.child(empty_note(c, t!("tasks.detail.no_activity")))
            }
            _ => {}
        }
        for item in &self.activities {
            let actor = item
                .user
                .as_ref()
                .map(|u| u.name.clone())
                .unwrap_or_else(|| t!("tasks.detail.system").into());
            list = list.child(
                div()
                    .flex()
                    .items_start()
                    .gap_2()
                    .child(
                        div()
                            .mt(px(6.))
                            .size(px(6.))
                            .flex_shrink_0()
                            .rounded_full()
                            .bg(c.text_muted),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .child(div().child(format!(
                                "{actor} — {}",
                                activity_text(&item.event_type, &item.payload)
                            )))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(c.text_muted)
                                    .child(local_time(&item.created_at)),
                            ),
                    ),
            );
        }
        if self.activity_cursor.is_some() {
            list = list.child(
                Button::new("more-activity")
                    .ghost()
                    .small()
                    .label(t!("tasks.list.more"))
                    .disabled(self.activity_state == Load::Loading)
                    .on_click(cx.listener(|this, _, _, cx| this.load_activity(true, cx))),
            );
        }
        list.into_any_element()
    }

    fn retry_note(&self, c: Colors, tab: Tab, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .items_center()
            .gap_2()
            .py_2()
            .child(
                div()
                    .text_sm()
                    .text_color(c.danger)
                    .child(t!("tasks.list.load_failed")),
            )
            .child(
                Button::new(SharedString::from(format!("retry-{tab:?}")))
                    .outline()
                    .small()
                    .label(t!("tasks.list.retry"))
                    .on_click(cx.listener(move |this, _, _, cx| match tab {
                        Tab::Activity => this.load_activity(false, cx),
                        Tab::Time => this.reload_time(cx),
                        Tab::Files => this.reload_files(cx),
                        Tab::Subtasks | Tab::Relations => this.reload_relations(cx),
                        Tab::Comments => {}
                    })),
            )
            .into_any_element()
    }

    // ---- 作業時間 ----

    #[inline(never)]
    fn render_time(
        &self,
        detail: &TaskDetailResponse,
        c: Colors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.time_state == Load::Failed && self.time_summary.is_none() {
            return self.retry_note(c, Tab::Time, cx);
        }
        let running = self.timer.as_ref().filter(|t| t.is_running);
        let elapsed = running.and_then(|t| t.started_at).map(|start| {
            let minutes = (Utc::now() - start).num_minutes().max(0) as i32;
            if minutes == 0 {
                t!("tasks.detail.timer_just_started").to_string()
            } else {
                t!(
                    "tasks.detail.timer_elapsed",
                    time = format_minutes(minutes),
                    since = start.with_timezone(&Local).format("%H:%M")
                )
            }
        });
        let timer = div()
            .flex()
            .items_center()
            .gap_3()
            .child(
                Button::new("timer-toggle")
                    .small()
                    .when(running.is_some(), |b| {
                        b.danger()
                            .icon(IconName::Square)
                            .label(t!("tasks.detail.timer_stop"))
                    })
                    .when(running.is_none(), |b| {
                        b.primary()
                            .icon(IconName::Play)
                            .label(t!("tasks.detail.timer_start"))
                    })
                    .disabled(self.acting)
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_timer(cx))),
            )
            .children(elapsed.map(|e| div().text_sm().text_color(c.success).child(e)));

        let summary = self.time_summary.as_ref().map(|s| {
            let estimate = s.estimated_minutes.or(detail.estimated_minutes);
            let mut parts = vec![t!(
                "tasks.detail.time_actual",
                time = format_minutes(s.actual_minutes)
            )];
            if let Some(e) = estimate {
                parts.push(t!("tasks.detail.time_estimate", time = format_minutes(e)));
            }
            if let Some(r) = s.remaining_minutes {
                parts.push(t!(
                    "tasks.detail.time_remaining",
                    time = format_minutes(r.max(0))
                ));
            }
            let by_user = s
                .by_user
                .iter()
                .map(|u| format!("{} {}", u.name, format_minutes(u.minutes)))
                .collect::<Vec<_>>()
                .join(" · ");
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_sm()
                        .child(parts.join(" / "))
                        .when(s.is_over, |d| {
                            d.child(
                                div()
                                    .px_2()
                                    .rounded_full()
                                    .bg(c.danger)
                                    .text_xs()
                                    .text_color(gpui_kit::white())
                                    .child(t!("tasks.detail.time_over")),
                            )
                        }),
                )
                .when_some(estimate.filter(|e| *e > 0), |d, e| {
                    d.child(
                        div().max_w(px(240.)).child(
                            Progress::new("time-progress")
                                .value(s.actual_minutes as f32 * 100. / e as f32)
                                .when(s.is_over, |p| p.color(c.danger)),
                        ),
                    )
                })
                .when(!by_user.is_empty(), |d| {
                    d.child(div().text_xs().text_color(c.text_muted).child(by_user))
                })
        });

        let form = div()
            .flex()
            .flex_col()
            .gap_1()
            .pt_2()
            .border_t_1()
            .border_color(c.border)
            .child(section_title(t!("tasks.detail.add_log")))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .w(px(110.))
                            .child(Input::new(&self.log_minutes_input).id("log-minutes")),
                    )
                    .child(
                        div()
                            .w(px(150.))
                            .child(DatePicker::new(&self.log_date_picker)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(120.))
                            .child(Input::new(&self.log_note_input).id("log-note")),
                    )
                    .child(
                        Button::new("add-log")
                            .small()
                            .label(t!("tasks.list.add"))
                            .disabled(self.acting)
                            .on_click(cx.listener(|this, _, _, cx| this.add_time_log(cx))),
                    ),
            );

        let names = |user: Uuid| {
            self.time_summary
                .as_ref()
                .and_then(|s| s.by_user.iter().find(|u| u.user_id == user))
                .map(|u| u.name.clone())
                .unwrap_or_default()
        };
        let mut logs = div().flex().flex_col().gap_0p5();
        if self.time_state == Load::Loading && self.time_logs.is_empty() {
            logs = logs.child(empty_note(c, t!("tasks.detail.loading")));
        } else if self.time_logs.is_empty() {
            logs = logs.child(empty_note(c, t!("tasks.detail.no_logs")));
        }
        let mut sorted = self.time_logs.clone();
        sorted.sort_by(|a, b| {
            b.logged_at
                .cmp(&a.logged_at)
                .then(b.created_at.cmp(&a.created_at))
        });
        for log in sorted {
            let own = self.me == Some(log.user_id);
            let id = log.id;
            logs = logs.child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_2()
                    .h(px(30.))
                    .text_sm()
                    .child(
                        div()
                            .w(px(84.))
                            .flex_shrink_0()
                            .text_color(c.text_muted)
                            .child(log.logged_at.format("%Y-%m-%d").to_string()),
                    )
                    .child(
                        div()
                            .w(px(64.))
                            .flex_shrink_0()
                            .font_weight(FontWeight::MEDIUM)
                            .child(format_minutes(log.logged_minutes)),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(c.text_muted)
                            .child(names(log.user_id)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_ellipsis()
                            .child(log.note.clone().unwrap_or_default()),
                    )
                    .when(own, |d| {
                        d.child(
                            Button::new(SharedString::from(format!("delete-log-{id}")))
                                .ghost()
                                .xsmall()
                                .icon(IconName::Trash)
                                .tooltip(t!("tasks.detail.delete"))
                                .disabled(self.acting)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.delete_time_log(id, window, cx)
                                })),
                        )
                    }),
            );
        }

        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(timer)
            .children(summary)
            .child(form)
            .child(logs)
            .into_any_element()
    }

    // ---- 添付 ----

    #[inline(never)]
    fn render_files(&self, c: Colors, cx: &mut Context<Self>) -> AnyElement {
        let mut list = div().flex().flex_col().gap_0p5();
        match self.files_state {
            Load::Failed if self.attachments.is_empty() => {
                return self.retry_note(c, Tab::Files, cx);
            }
            Load::Loading | Load::Idle if self.attachments.is_empty() => {
                list = list.child(empty_note(c, t!("tasks.detail.loading")));
            }
            _ if self.attachments.is_empty() => {
                list = list.child(empty_note(c, t!("tasks.detail.no_files")));
            }
            _ => {}
        }
        for file in &self.attachments {
            let url = self
                .client
                .as_ref()
                .map_or_else(|| file.url.clone(), |c| c.absolute_url(&file.url));
            let (id, name) = (file.id, file.name.clone());
            list = list.child(
                div()
                    .id(SharedString::from(format!("file-{id}")))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .h(px(34.))
                    .rounded_md()
                    .cursor_pointer()
                    .hover(|d| d.bg(c.hover))
                    .on_click(cx.listener(move |_, _, _, cx| cx.open_url(&url)))
                    .child(
                        Icon::new(IconName::Paperclip)
                            .size_4()
                            .text_color(c.text_muted),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .text_ellipsis()
                            .child(file.name.clone()),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(c.text_muted)
                            .child(format!("{} · {}", file_size(file.size), file.mime_type)),
                    )
                    .child(
                        Button::new(SharedString::from(format!("detach-{id}")))
                            .ghost()
                            .xsmall()
                            .icon(IconName::X)
                            .tooltip(t!("tasks.detail.detach"))
                            .disabled(self.acting)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.detach_file(id, name.clone(), window, cx)
                            })),
                    ),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(list)
            .child(
                div()
                    .text_xs()
                    .text_color(c.text_muted)
                    .child(t!("tasks.detail.files_hint")),
            )
            .into_any_element()
    }
}
