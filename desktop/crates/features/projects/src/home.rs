//! ホーム（ダッシュボード）。件数・期限別のタスク・プロジェクトの進捗・最近の更新。

use api::types::{DashboardResponse, DashboardTask};
use api::{Client, DashboardQuery};
use chrono::{Local, NaiveDate, Utc};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::progress::Progress;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{Disableable, Sizable, WindowExt};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use i18n::t;
use uuid::Uuid;

use crate::activity::{activity_actor, activity_text};
use crate::ui::{Colors, card, heading, parse_hex, placeholder, relative_time, swatch};

const PAGE: u32 = 50;

pub enum HomeEvent {
    OpenTask { project: Uuid, task: Uuid },
    OpenProject(Uuid),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Filter {
    Today,
    Week,
    Overdue,
    All,
}

impl Filter {
    const ALL: [Filter; 4] = [Filter::Today, Filter::Week, Filter::Overdue, Filter::All];

    fn api(self) -> &'static str {
        match self {
            Filter::Today => "today",
            Filter::Week => "week",
            Filter::Overdue => "overdue",
            Filter::All => "all",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Filter::Today => t!("projects.home.filter_today"),
            Filter::Week => t!("projects.home.filter_week"),
            Filter::Overdue => t!("projects.home.filter_overdue"),
            Filter::All => t!("projects.home.filter_all"),
        }
    }
}

pub struct HomeView {
    client: Option<Client>,
    tenant: Option<Uuid>,
    filter: Filter,
    data: Option<DashboardResponse>,
    tasks: Vec<DashboardTask>,
    loading: bool,
    error: Option<String>,
    shown_error: Option<String>,
    generation: u64,
}

impl EventEmitter<HomeEvent> for HomeView {}

impl HomeView {
    pub fn new(
        client: Option<Client>,
        tenant: Option<Uuid>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self {
            client,
            tenant,
            filter: Filter::Today,
            data: None,
            tasks: vec![],
            loading: false,
            error: None,
            shown_error: None,
            generation: 0,
        };
        this.reload(cx);
        this
    }

    pub fn set_client(&mut self, client: Client, tenant: Option<Uuid>, cx: &mut Context<Self>) {
        if self.tenant != tenant {
            self.data = None;
            self.tasks.clear();
        }
        self.client = Some(client);
        self.tenant = tenant;
        self.reload(cx);
    }

    pub fn clear_client(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        self.client = None;
        self.data = None;
        self.tasks.clear();
        self.loading = false;
        cx.notify();
    }

    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.fetch(0, cx);
    }

    fn fetch(&mut self, offset: u64, cx: &mut Context<Self>) {
        let (Some(client), Some(tenant)) = (self.client.clone(), self.tenant) else {
            return;
        };
        self.generation += 1;
        let generation = self.generation;
        self.loading = true;
        self.error = None;
        let query = DashboardQuery {
            filter: Some(self.filter.api().into()),
            timezone: local_timezone(),
            limit: Some(PAGE),
            offset: Some(offset),
        };
        cx.notify();
        cx.spawn(async move |this, cx| {
            let res = client.get_dashboard(tenant, &query).await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.loading = false;
                match res {
                    Ok(data) => {
                        if offset == 0 {
                            this.tasks = data.tasks.clone();
                        } else {
                            this.tasks.extend(data.tasks.iter().cloned());
                        }
                        this.data = Some(data);
                    }
                    Err(e) => this.error = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn set_filter(&mut self, filter: Filter, cx: &mut Context<Self>) {
        if self.filter != filter {
            self.filter = filter;
            self.tasks.clear();
            self.reload(cx);
        }
    }

    fn tiles(&self, c: &Colors) -> Div {
        let counts = self.data.as_ref().map(|d| d.counts.clone());
        let tile = |label: &'static str, value: Option<i64>, color: Option<Hsla>| {
            div()
                .flex()
                .flex_col()
                .gap_1()
                .flex_1()
                .min_w(px(110.))
                .p_3()
                .rounded_lg()
                .border_1()
                .border_color(c.border)
                .bg(c.surface)
                .child(div().text_xs().text_color(c.text_muted).child(label))
                .child(
                    div()
                        .text_xl()
                        .font_weight(FontWeight::SEMIBOLD)
                        .when_some(color.filter(|_| value.unwrap_or(0) > 0), |d, color| {
                            d.text_color(color)
                        })
                        .child(value.map(|v| v.to_string()).unwrap_or_else(|| "—".into())),
                )
        };
        div()
            .flex()
            .flex_wrap()
            .gap_3()
            .child(tile(
                t!("projects.home.open"),
                counts.as_ref().map(|x| x.open),
                None,
            ))
            .child(tile(
                t!("projects.home.today"),
                counts.as_ref().map(|x| x.today),
                Some(c.accent),
            ))
            .child(tile(
                t!("projects.home.overdue"),
                counts.as_ref().map(|x| x.overdue),
                Some(c.danger),
            ))
            .child(tile(
                t!("projects.home.week"),
                counts.as_ref().map(|x| x.week),
                None,
            ))
            .child(tile(
                t!("projects.home.completed_week"),
                counts.as_ref().map(|x| x.completed_week),
                Some(c.success),
            ))
    }

    fn task_rows(&self, c: &Colors, cx: &mut Context<Self>) -> AnyElement {
        if self.tasks.is_empty() {
            return card(
                c,
                vec![placeholder(
                    c,
                    if self.loading {
                        t!("projects.common.loading")
                    } else {
                        t!("projects.home.no_tasks")
                    },
                )],
            );
        }
        let today = Local::now().date_naive();
        let rows = self
            .tasks
            .iter()
            .enumerate()
            .map(|(ix, item)| {
                let task = &item.task;
                let (project, id) = (task.project.id, task.id);
                let due = task.soft_deadline.or(task.hard_deadline);
                let due_day = due.map(|d| d.with_timezone(&Local).date_naive());
                let overdue = due_day.is_some_and(|d| d < today);
                div()
                    .id(("home-task", ix))
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_4()
                    .py_2()
                    .cursor_pointer()
                    .hover(|d| d.bg(c.hover))
                    .child(swatch(
                        parse_hex(&task.status.color).unwrap_or(c.text_muted),
                    ))
                    .child(
                        div()
                            .w(px(84.))
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(c.text_muted)
                            .child(task.seq_key.clone()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .text_ellipsis()
                            .child(task.title.clone()),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(c.text_muted)
                            .child(task.status.name.clone()),
                    )
                    .when_some(due_day, |d, day| {
                        d.child(
                            div()
                                .w(px(80.))
                                .flex_shrink_0()
                                .text_xs()
                                .text_color(if overdue { c.danger } else { c.text_muted })
                                .child(day.format("%m/%d").to_string()),
                        )
                    })
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.emit(HomeEvent::OpenTask { project, task: id })
                    }))
                    .into_any_element()
            })
            .collect();
        card(c, rows)
    }

    fn side(&self, c: &Colors, cx: &mut Context<Self>) -> Div {
        let Some(data) = &self.data else {
            return div();
        };
        let projects: Vec<AnyElement> = data
            .projects
            .iter()
            .enumerate()
            .map(|(ix, p)| {
                let id = p.project.id;
                let pct = if p.total > 0 {
                    p.completed as f32 * 100. / p.total as f32
                } else {
                    0.
                };
                div()
                    .id(("home-project", ix))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .px_4()
                    .py_2()
                    .cursor_pointer()
                    .hover(|d| d.bg(c.hover))
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .text_sm()
                            .child(div().text_ellipsis().child(match &p.project.icon_emoji {
                                Some(e) if !e.is_empty() => {
                                    format!("{e} {}", p.project.name)
                                }
                                _ => p.project.name.clone(),
                            }))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(c.text_muted)
                                    .child(format!("{}/{}", p.completed, p.total)),
                            ),
                    )
                    .child(Progress::new(("home-project-progress", ix)).value(pct))
                    .on_click(cx.listener(move |_, _, _, cx| cx.emit(HomeEvent::OpenProject(id))))
                    .into_any_element()
            })
            .collect();
        let now = Utc::now();
        let activities: Vec<AnyElement> = data
            .activities
            .iter()
            .enumerate()
            .map(|(ix, a)| {
                let project = data
                    .projects
                    .iter()
                    .find(|p| p.project.key == a.project_key)
                    .map(|p| p.project.id);
                div()
                    .id(("home-activity", ix))
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .px_4()
                    .py_2()
                    .when(project.is_some(), |d| {
                        d.cursor_pointer().hover(|d| d.bg(c.hover))
                    })
                    .child(div().text_sm().child(format!(
                        "{} {}",
                        activity_actor(&a.activity),
                        activity_text(&a.activity)
                    )))
                    .child(
                        div()
                            .text_xs()
                            .text_color(c.text_muted)
                            .text_ellipsis()
                            .child(format!(
                                "{}-{} {} · {}",
                                a.project_key,
                                a.task_seq_id,
                                a.task_title,
                                relative_time(&a.activity.created_at, now)
                            )),
                    )
                    .when_some(project, |d, id| {
                        d.on_click(
                            cx.listener(move |_, _, _, cx| cx.emit(HomeEvent::OpenProject(id))),
                        )
                    })
                    .into_any_element()
            })
            .collect();
        div()
            .flex()
            .flex_col()
            .gap_3()
            .w(px(320.))
            .flex_shrink_0()
            .child(heading(c, t!("projects.home.completed_history")))
            .child(history(c, &data.days))
            .child(heading(c, t!("projects.home.projects")))
            .child(if projects.is_empty() {
                card(c, vec![placeholder(c, t!("projects.home.no_projects"))])
            } else {
                card(c, projects)
            })
            .child(heading(c, t!("projects.home.activity")))
            .child(if activities.is_empty() {
                card(c, vec![placeholder(c, t!("projects.home.no_activity"))])
            } else {
                card(c, activities)
            })
    }
}

impl Render for HomeView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.error != self.shown_error {
            self.shown_error = self.error.clone();
            if let Some(error) = &self.error {
                window.push_notification(Notification::new().message(error.clone()), cx);
            }
        }
        let c = ::ui::colors(cx);
        if self.client.is_none() {
            return div().id("home").size_full().into_any_element();
        }
        let selected = Filter::ALL
            .iter()
            .position(|f| *f == self.filter)
            .unwrap_or(0);
        let total = self.data.as_ref().map(|d| d.total).unwrap_or(0);
        let more = (total - self.tasks.len() as i64).max(0);
        let offset = self.tasks.len() as u64;
        let tabs = TabBar::new("home-filter")
            .underline()
            .small()
            .selected_index(selected)
            .children(Filter::ALL.map(|f| Tab::new().label(f.label())))
            .on_click(cx.listener(|this, ix: &usize, _, cx| {
                if let Some(f) = Filter::ALL.get(*ix) {
                    this.set_filter(*f, cx);
                }
            }));
        div()
            .id("home")
            .size_full()
            .overflow_y_scroll()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_6()
                    .px_4()
                    .pt_1()
                    .pb_6()
                    .child(self.tiles(&c))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_start()
                            .gap_4()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_3()
                                    .flex_1()
                                    .min_w(px(360.))
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .border_b_1()
                                            .border_color(c.border)
                                            .child(div().flex_1().child(tabs))
                                            .child(
                                                Button::new("home-refresh")
                                                    .ghost()
                                                    .small()
                                                    .icon(IconName::RefreshCw)
                                                    .tooltip(t!("projects.home.refresh"))
                                                    .disabled(self.loading)
                                                    .on_click(cx.listener(|this, _, _, cx| {
                                                        this.reload(cx)
                                                    })),
                                            ),
                                    )
                                    .child(self.task_rows(&c, cx))
                                    .when(more > 0, |d| {
                                        d.child(
                                            div().flex().justify_center().child(
                                                Button::new("home-more")
                                                    .small()
                                                    .ghost()
                                                    .disabled(self.loading)
                                                    .label(t!("projects.home.more", count = more))
                                                    .on_click(cx.listener(
                                                        move |this, _, _, cx| {
                                                            this.fetch(offset, cx)
                                                        },
                                                    )),
                                            ),
                                        )
                                    }),
                            )
                            .child(self.side(&c, cx)),
                    ),
            )
            .into_any_element()
    }
}

/// 日ごとの完了数（古い順）の小さな棒グラフ。
fn history(c: &Colors, days: &[api::types::DashboardDay]) -> AnyElement {
    if days.is_empty() {
        return card(c, vec![placeholder(c, t!("projects.home.no_history"))]);
    }
    let max = days.iter().map(|d| d.count).max().unwrap_or(0).max(1) as f32;
    let height = 48.;
    let total: i64 = days.iter().map(|d| d.count).sum();
    div()
        .flex()
        .flex_col()
        .gap_2()
        .p_3()
        .rounded_lg()
        .border_1()
        .border_color(c.border)
        .bg(c.surface)
        .child(
            div()
                .flex()
                .items_end()
                .gap(px(2.))
                .h(px(height))
                .children(days.iter().map(|d| {
                    div()
                        .flex_1()
                        .h(px((d.count as f32 / max * height).max(2.)))
                        .rounded_sm()
                        .bg(if d.count > 0 { c.success } else { c.border })
                })),
        )
        .child(
            div()
                .flex()
                .justify_between()
                .text_xs()
                .text_color(c.text_muted)
                .child(day_label(days.first().map(|d| d.date.as_str())))
                .child(t!("projects.home.history_total", count = total))
                .child(day_label(days.last().map(|d| d.date.as_str()))),
        )
        .into_any_element()
}

fn day_label(date: Option<&str>) -> String {
    date.and_then(|d| NaiveDate::parse_from_str(d.get(..10)?, "%Y-%m-%d").ok())
        .map(|d| d.format("%m/%d").to_string())
        .unwrap_or_default()
}

/// OS のタイムゾーン名（IANA）。`TZ` か `/etc/localtime` のリンク先から取る。
/// 取れなければ None（backend は UTC で数える）。
fn local_timezone() -> Option<String> {
    if let Ok(tz) = std::env::var("TZ")
        && !tz.is_empty()
        && !tz.starts_with(':')
    {
        return Some(tz);
    }
    let target = std::fs::read_link("/etc/localtime").ok()?;
    zone_from_path(&target.to_string_lossy())
}

fn zone_from_path(path: &str) -> Option<String> {
    let (_, zone) = path.split_once("zoneinfo/")?;
    (!zone.is_empty()).then(|| zone.to_string())
}

#[cfg(test)]
mod tests {
    use super::zone_from_path;

    #[test]
    fn zone_is_taken_after_zoneinfo() {
        assert_eq!(
            zone_from_path("/var/db/timezone/zoneinfo/Asia/Tokyo").as_deref(),
            Some("Asia/Tokyo")
        );
        assert_eq!(
            zone_from_path("/usr/share/zoneinfo/America/New_York").as_deref(),
            Some("America/New_York")
        );
        assert!(zone_from_path("/etc/foo").is_none());
    }
}
