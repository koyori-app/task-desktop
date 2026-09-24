//! desktop.md §12 Notification Center。
//! 一覧（unread/read・種別フィルタ・カーソル継続読み込み）と
//! クリック時の内部遷移要求（§11）。OS 通知そのものは core::sync が出す。

use api::NotificationsQuery;
use api::spec::{NotificationItem, NotificationKind, NotificationProject, NotificationTarget};
use chrono::{DateTime, Utc};
use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::component::IndexPath;
use gpui_kit::component::Theme;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::list::{List, ListDelegate, ListItem, ListState};
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use i18n::t;

const PAGE_SIZE: u32 = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    All,
    Unread,
    Task,
    Review,
}

impl Filter {
    const ALL: [Filter; 4] = [Self::All, Self::Unread, Self::Task, Self::Review];

    fn label(self) -> &'static str {
        match self {
            Self::All => t!("notifications.filter.all"),
            Self::Unread => t!("notifications.filter.unread"),
            Self::Task => t!("notifications.filter.task"),
            Self::Review => t!("notifications.filter.review"),
        }
    }

    fn query(self) -> NotificationsQuery {
        NotificationsQuery {
            unread: (self == Self::Unread).then_some(true),
            kind: match self {
                Self::Task => Some(NotificationKind::Task),
                Self::Review => Some(NotificationKind::Review),
                _ => None,
            },
            limit: Some(PAGE_SIZE),
            cursor: None,
            after: None,
        }
    }

    fn matches(self, item: &NotificationItem) -> bool {
        match self {
            Self::All => true,
            Self::Unread => item.read_at.is_none(),
            Self::Task => {
                item.notification_type.starts_with("task")
                    || matches!(
                        item.notification_type.as_str(),
                        "assigned"
                            | "mentioned"
                            | "status_changed"
                            | "comment_added"
                            | "deadline_soon"
                    )
            }
            Self::Review => {
                item.notification_type.starts_with("review")
                    || item.notification_type.starts_with("finding")
            }
        }
    }
}

/// クリック時に AppShell へ投げる遷移要求（§11）。
/// target が無い古い通知は `target: None` で上げ、遷移側が握り潰す。
#[derive(Debug, Clone)]
pub struct NavTarget {
    pub project: Option<NotificationProject>,
    pub target: Option<NotificationTarget>,
}

#[derive(Debug, Clone)]
pub enum CenterEvent {
    Navigate(NavTarget),
    /// Authoritative count returned by the list endpoint, after reads too.
    UnreadCount(i64),
}

struct NotificationListDelegate {
    owner: WeakEntity<NotificationCenter>,
    items: Vec<NotificationItem>,
    selected: Option<IndexPath>,
    loading: bool,
    signed_in: bool,
    has_more: bool,
}

impl ListDelegate for NotificationListDelegate {
    type Item = ListItem;

    fn items_count(&self, _: usize, _: &App) -> usize {
        self.items.len()
    }

    fn render_item(
        &mut self,
        index: IndexPath,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<Self::Item> {
        self.items
            .get(index.row)
            .map(|item| NotificationCenter::row(item, index.row, cx))
    }

    fn set_selected_index(
        &mut self,
        index: Option<IndexPath>,
        _: &mut Window,
        _: &mut Context<ListState<Self>>,
    ) {
        self.selected = index;
    }

    fn confirm(&mut self, _: bool, _: &mut Window, cx: &mut Context<ListState<Self>>) {
        let Some(item) = self.selected.and_then(|index| self.items.get(index.row)) else {
            return;
        };
        let id = item.id;
        let owner = self.owner.clone();
        cx.defer(move |cx| {
            let _ = owner.update(cx, |owner, cx| {
                if let Some(index) = owner.items.iter().position(|item| item.id == id) {
                    owner.open(index, cx);
                }
            });
        });
    }

    fn loading(&self, _: &App) -> bool {
        self.loading && self.items.is_empty()
    }

    fn render_empty(
        &mut self,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> impl IntoElement {
        div()
            .p_8()
            .text_sm()
            .text_color(Theme::global(cx).muted_foreground)
            .child(if self.signed_in {
                t!("notifications.center.empty")
            } else {
                t!("notifications.center.signed_out")
            })
    }

    fn has_more(&self, _: &App) -> bool {
        self.has_more && !self.loading
    }
    fn load_more_threshold(&self) -> usize {
        5
    }

    fn load_more(&mut self, _: &mut Window, cx: &mut Context<ListState<Self>>) {
        let owner = self.owner.clone();
        cx.defer(move |cx| {
            let _ = owner.update(cx, |owner, cx| owner.load_more(cx));
        });
    }
}

/// Notification Center の View。履歴ページング用カーソルは
/// 同期エンジンの高水位カーソルとは別物（§9: cursor=過去方向）。
pub struct NotificationCenter {
    client: Option<api::Client>,
    items: Vec<NotificationItem>,
    history_cursor: Option<String>,
    filter: Filter,
    loading: bool,
    error: Option<String>,
    request_generation: u64,
    list: Option<Entity<ListState<NotificationListDelegate>>>,
    reset_list: bool,
}

impl NotificationCenter {
    pub fn new(client: Option<api::Client>) -> Self {
        Self {
            client,
            items: vec![],
            history_cursor: None,
            filter: Filter::All,
            loading: false,
            error: None,
            request_generation: 0,
            list: None,
            reset_list: true,
        }
    }

    /// ログイン後にクライアントが出来た時に差し替えて再読込する。
    pub fn set_client(&mut self, client: api::Client, cx: &mut Context<Self>) {
        self.client = Some(client);
        self.refresh(cx);
    }

    /// ログアウト時に呼ぶ。以降の API 呼び出しを止める。
    pub fn clear_client(&mut self, cx: &mut Context<Self>) {
        self.client = None;
        self.request_generation += 1;
        self.items.clear();
        self.history_cursor = None;
        self.loading = false;
        self.error = None;
        self.reset_list = true;
        cx.notify();
    }

    /// 同期エンジンの catch-up で届いた新着を先頭へ差す（重複は弾く）。
    pub fn prepend_new(&mut self, new_items: &[NotificationItem], cx: &mut Context<Self>) {
        // catch-up は ASC で来るので 1 件ずつ先頭挿入すると DESC になる。
        for item in new_items {
            if self.items.iter().any(|i| i.id == item.id) {
                continue;
            }
            if self.filter.matches(item) {
                self.items.insert(0, item.clone());
            }
        }
        cx.notify();
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(client) = self.client.clone() else {
            return;
        };
        let query = self.filter.query();
        self.request_generation += 1;
        let generation = self.request_generation;
        self.loading = true;
        self.error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let res = client.list_notifications(&query).await;
            let _ = this.update(cx, |this, cx| {
                if this.request_generation != generation {
                    return;
                }
                this.loading = false;
                match res {
                    Ok(page) => {
                        cx.emit(CenterEvent::UnreadCount(page.unread_count));
                        this.items = page.notifications;
                        this.history_cursor = page.next_cursor;
                    }
                    Err(e) => this.error = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 履歴の続き（`cursor` = このリストの最古行より古いもの）。
    fn load_more(&mut self, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        let (Some(client), Some(cursor)) = (self.client.clone(), self.history_cursor.clone())
        else {
            return;
        };
        let mut query = self.filter.query();
        query.cursor = Some(cursor);
        let generation = self.request_generation;
        self.loading = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let res = client.list_notifications(&query).await;
            let _ = this.update(cx, |this, cx| {
                if this.request_generation != generation {
                    return;
                }
                this.loading = false;
                match res {
                    Ok(page) => {
                        cx.emit(CenterEvent::UnreadCount(page.unread_count));
                        for item in page.notifications {
                            if !this.items.iter().any(|i| i.id == item.id) {
                                this.items.push(item);
                            }
                        }
                        this.history_cursor = page.next_cursor;
                    }
                    Err(e) => this.error = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn set_filter(&mut self, filter: Filter, cx: &mut Context<Self>) {
        if self.filter == filter {
            return;
        }
        self.filter = filter;
        self.items.clear();
        self.history_cursor = None;
        self.reset_list = true;
        self.refresh(cx);
    }

    /// §12 "Mark all read"。パレットからも呼ぶので pub。
    pub fn mark_all_read(&mut self, cx: &mut Context<Self>) {
        let Some(client) = self.client.clone() else {
            return;
        };
        let old_items = self.items.clone();
        let generation = self.request_generation;
        let now = Utc::now();
        for item in &mut self.items {
            if item.read_at.is_none() {
                item.read_at = Some(now);
            }
        }
        if self.filter == Filter::Unread {
            self.items.clear();
        }
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = client.mark_all_notifications_read().await;
            let _ = this.update(cx, |this, cx| {
                if this.request_generation != generation || this.client.is_none() {
                    return;
                }
                match result {
                    Ok(()) => this.refresh(cx),
                    Err(error) => {
                        this.items = old_items;
                        this.error = Some(error.to_string());
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    /// §11: クリック → 既読化 + 遷移要求。
    fn open(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(item) = self.items.get(ix).cloned() else {
            return;
        };
        if item.read_at.is_none() {
            if let Some(slot) = self.items.get_mut(ix) {
                slot.read_at = Some(Utc::now());
            }
            if let Some(client) = self.client.clone() {
                let id = item.id;
                let generation = self.request_generation;
                cx.spawn(async move |this, cx| {
                    let result = client.mark_notification_read(id).await;
                    let _ = this.update(cx, |this, cx| {
                        if this.request_generation != generation || this.client.is_none() {
                            return;
                        }
                        match result {
                            Ok(()) => this.refresh(cx),
                            Err(error) => {
                                if let Some(item) = this.items.iter_mut().find(|item| item.id == id)
                                {
                                    item.read_at = None;
                                }
                                this.error = Some(error.to_string());
                                cx.notify();
                            }
                        }
                    });
                })
                .detach();
            }
        }
        cx.emit(CenterEvent::Navigate(NavTarget {
            project: item.project.clone(),
            target: item.target.clone(),
        }));
        cx.notify();
    }

    fn kind_icon(item: &NotificationItem) -> IconName {
        let t = item.notification_type.as_str();
        if t.starts_with("review") {
            IconName::SquareCheck
        } else if t.starts_with("finding") {
            IconName::TriangleAlert
        } else if Filter::Task.matches(item) {
            IconName::ClipboardList
        } else {
            IconName::Bell
        }
    }

    fn row(item: &NotificationItem, ix: usize, cx: &App) -> ListItem {
        let c = Theme::global(cx).semantic_tokens().colors;
        let primary = Theme::global(cx).primary;
        let unread = item.read_at.is_none();
        let text = core::notify_text::toast(item);
        let title: SharedString = text.title.into();
        let body: SharedString = text.body.into();
        let time: SharedString = relative_time(&item.created_at).into();

        ListItem::new(("nc-row", ix))
            .h(px(92.))
            .px_4()
            .py_2()
            .when(unread, |row| row.bg(c.muted))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_start()
                    .gap_3()
                    .w_full()
                    .min_w_0()
                    .child(
                        div()
                            .mt_1()
                            .size(px(8.))
                            .rounded_full()
                            .flex_shrink_0()
                            .when(unread, |d| d.bg(primary)),
                    )
                    .child(
                        Icon::new(Self::kind_icon(item))
                            .size_4()
                            .text_color(c.muted_foreground),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .truncate()
                                    .text_sm()
                                    .font_weight(if unread {
                                        FontWeight::SEMIBOLD
                                    } else {
                                        FontWeight::NORMAL
                                    })
                                    .child(title),
                            )
                            .when(!body.is_empty(), |d| {
                                d.child(
                                    div()
                                        .truncate()
                                        .text_xs()
                                        .text_color(c.muted_foreground)
                                        .child(body),
                                )
                            })
                            .child(div().text_xs().text_color(c.muted_foreground).child(time)),
                    ),
            )
    }
}

impl EventEmitter<CenterEvent> for NotificationCenter {}

impl Render for NotificationCenter {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (c, danger) = {
            let theme = Theme::global(cx);
            (theme.semantic_tokens().colors, theme.danger)
        };

        let list = self
            .list
            .get_or_insert_with(|| {
                let owner = cx.entity().downgrade();
                cx.new(|cx| {
                    ListState::new(
                        NotificationListDelegate {
                            owner,
                            items: vec![],
                            selected: None,
                            loading: false,
                            signed_in: false,
                            has_more: false,
                        },
                        window,
                        cx,
                    )
                })
            })
            .clone();
        let _ = list.read(cx).focus_handle(cx).tab_stop(true);
        list.update(cx, |state, cx| {
            let delegate = state.delegate_mut();
            delegate.items = self.items.clone();
            delegate.loading = self.loading;
            delegate.signed_in = self.client.is_some();
            delegate.has_more = self.history_cursor.is_some() && self.error.is_none();
            if self.reset_list {
                state.set_selected_index(None, window, cx);
                state.scroll_to_item(IndexPath::default(), ScrollStrategy::Top, window, cx);
            }
            cx.notify();
        });
        self.reset_list = false;

        div()
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
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .px_4()
                    .py_3()
                    .border_b_1()
                    .border_color(c.border)
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(t!("notifications.center.title")),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("nc-refresh")
                            .ghost()
                            .icon(IconName::RefreshCcwDot)
                            .tooltip(t!("notifications.center.refresh"))
                            .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                    )
                    .child(
                        Button::new("nc-mark-all")
                            .ghost()
                            .icon(IconName::Check)
                            .label(t!("notifications.center.mark_all_read"))
                            .on_click(cx.listener(|this, _, _, cx| this.mark_all_read(cx))),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_shrink_0()
                    .gap_1()
                    .px_4()
                    .py_2()
                    .border_b_1()
                    .border_color(c.border)
                    .child(
                        TabBar::new("notification-filters")
                            .underline()
                            .selected_index(self.filter as usize)
                            .children(
                                Filter::ALL
                                    .iter()
                                    .map(|filter| Tab::new().label(filter.label())),
                            )
                            .on_click(cx.listener(|this, index: &usize, _, cx| {
                                if let Some(filter) = Filter::ALL.get(*index) {
                                    this.set_filter(*filter, cx);
                                }
                            })),
                    ),
            )
            .when_some(self.error.clone(), |d, e| {
                d.child(
                    div()
                        .px_4()
                        .py_2()
                        .child(div().text_sm().text_color(danger).child(e)),
                )
            })
            .child(div().flex_1().min_h_0().child(List::new(&list).size_full()))
            .when(self.loading && !self.items.is_empty(), |view| {
                view.child(
                    div()
                        .p_2()
                        .text_xs()
                        .text_color(c.muted_foreground)
                        .child(t!("notifications.center.loading")),
                )
            })
    }
}

/// "たった今" / "5分前" / "3時間前" / "昨日" / "2日前" / "2026-09-24"
/// （en: "just now" / "5m" / "3h" / "1d" / "2d"）。
fn relative_time(dt: &DateTime<Utc>) -> String {
    let secs = (Utc::now() - *dt).num_seconds().max(0);
    if secs < 60 {
        t!("notifications.time.just_now").into()
    } else if secs < 3600 {
        t!("notifications.time.minutes_ago", n = secs / 60)
    } else if secs < 86400 {
        t!("notifications.time.hours_ago", n = secs / 3600)
    } else if secs < 2 * 86400 {
        t!("notifications.time.yesterday").into()
    } else if secs < 7 * 86400 {
        t!("notifications.time.days_ago", n = secs / 86400)
    } else {
        dt.format("%Y-%m-%d").to_string()
    }
}

#[cfg(test)]
mod tests {
    // `use super::*` だと gpui::test が #[test] を shadow するので個別 import。
    use super::{Filter, relative_time};
    use api::spec::{NotificationItem, NotificationKind};
    use chrono::Utc;
    use i18n::t;

    #[test]
    fn filter_query_shape() {
        let q = Filter::Unread.query();
        assert_eq!(q.unread, Some(true));
        assert!(q.cursor.is_none() && q.after.is_none());
        let q = Filter::Task.query();
        assert_eq!(q.kind, Some(NotificationKind::Task));
        assert!(q.unread.is_none());
    }

    #[test]
    fn task_filter_accepts_backend_notification_names() {
        let mut item = NotificationItem {
            id: uuid::Uuid::nil(),
            notification_type: String::new(),
            project: None,
            task: None,
            payload: Default::default(),
            target: None,
            cursor: None,
            read_at: None,
            created_at: Utc::now(),
        };
        for kind in [
            "assigned",
            "mentioned",
            "status_changed",
            "comment_added",
            "task.assigned",
        ] {
            item.notification_type = kind.into();
            assert!(Filter::Task.matches(&item), "{kind}");
            assert!(!Filter::Review.matches(&item), "{kind}");
        }
        item.notification_type = "review.finding_fixed".into();
        assert!(Filter::Review.matches(&item));
        assert!(!Filter::Task.matches(&item));
    }

    #[test]
    fn relative_time_buckets() {
        let now = Utc::now();
        assert_eq!(
            relative_time(&(now - chrono::Duration::seconds(10))),
            t!("notifications.time.just_now")
        );
        assert_eq!(
            relative_time(&(now - chrono::Duration::minutes(5))),
            t!("notifications.time.minutes_ago", n = 5)
        );
        assert_eq!(
            relative_time(&(now - chrono::Duration::hours(3))),
            t!("notifications.time.hours_ago", n = 3)
        );
        assert_eq!(
            relative_time(&(now - chrono::Duration::hours(30))),
            t!("notifications.time.yesterday")
        );
        assert_eq!(
            relative_time(&(now - chrono::Duration::days(2))),
            t!("notifications.time.days_ago", n = 2)
        );
    }
}
