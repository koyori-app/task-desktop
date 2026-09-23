//! desktop.md §12 Notification Center。
//! 一覧（unread/read・種別フィルタ・カーソル継続読み込み）と
//! クリック時の内部遷移要求（§11）。OS 通知そのものは core::sync が出す。

use api::NotificationsQuery;
use api::spec::{NotificationItem, NotificationKind, NotificationProject, NotificationTarget};
use chrono::{DateTime, Utc};
use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::component::Theme;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

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
            Self::All => "All",
            Self::Unread => "Unread",
            Self::Task => "Task",
            Self::Review => "Review",
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
            Self::Task => item.notification_type.starts_with("task"),
            // 本番の古い type（"assigned" 等）は task 系扱いにしない
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
        self.loading = true;
        self.error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let res = client.list_notifications(&query).await;
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                match res {
                    Ok(page) => {
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
        let (Some(client), Some(cursor)) = (self.client.clone(), self.history_cursor.clone())
        else {
            return;
        };
        let mut query = self.filter.query();
        query.cursor = Some(cursor);
        self.loading = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let res = client.list_notifications(&query).await;
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                match res {
                    Ok(page) => {
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
        self.refresh(cx);
    }

    /// §12 "Mark all read"。パレットからも呼ぶので pub。
    pub fn mark_all_read(&mut self, cx: &mut Context<Self>) {
        let now = Utc::now();
        for item in &mut self.items {
            if item.read_at.is_none() {
                item.read_at = Some(now);
            }
        }
        cx.notify();
        if let Some(client) = self.client.clone() {
            cx.spawn(async move |_, _| {
                let _ = client.mark_all_notifications_read().await;
            })
            .detach();
        }
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
                cx.spawn(async move |_, _| {
                    let _ = client.mark_notification_read(id).await;
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
        } else if t.starts_with("task") || t == "assigned" {
            IconName::ClipboardList
        } else {
            IconName::Bell
        }
    }

    fn row(&self, item: &NotificationItem, ix: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let c = Theme::global(cx).semantic_tokens().colors.clone();
        let unread = item.read_at.is_none();
        let text = core::notify_text::toast(item);
        let title: SharedString = text.title.into();
        let body: SharedString = text.body.into();
        let time: SharedString = relative_time(&item.created_at).into();

        div()
            .id(("nc-row", ix))
            .flex()
            .flex_row()
            .items_start()
            .gap_3()
            .px_4()
            .py_3()
            .border_b_1()
            .border_color(c.border)
            .when(unread, |d| d.bg(c.muted))
            .hover(|s| s.bg(c.muted))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| this.open(ix, cx)))
            .child(
                div()
                    .mt_1()
                    .size(px(8.))
                    .rounded_full()
                    .flex_shrink_0()
                    .when(unread, |d| d.bg(c.accent)),
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
                            .text_sm()
                            .font_weight(if unread {
                                FontWeight::SEMIBOLD
                            } else {
                                FontWeight::NORMAL
                            })
                            .child(title),
                    )
                    .when(!body.is_empty(), |d| {
                        d.child(div().text_xs().text_color(c.muted_foreground).child(body))
                    })
                    .child(div().text_xs().text_color(c.muted_foreground).child(time)),
            )
    }
}

impl EventEmitter<CenterEvent> for NotificationCenter {}

impl Render for NotificationCenter {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (c, danger) = {
            let theme = Theme::global(cx);
            (theme.semantic_tokens().colors.clone(), theme.danger)
        };

        let mut list = div()
            .id("nc-list")
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .overflow_y_scroll();
        for (ix, item) in self.items.iter().enumerate() {
            list = list.child(self.row(item, ix, cx));
        }
        if self.items.is_empty() && !self.loading {
            list =
                list.child(div().flex_1().p_8().child(
                    div().text_sm().text_color(c.muted_foreground).child(
                        if self.client.is_some() {
                            "No notifications"
                        } else {
                            "Sign in to see notifications"
                        },
                    ),
                ));
        }
        if self.loading {
            list = list.child(
                div().p_4().child(
                    div()
                        .text_xs()
                        .text_color(c.muted_foreground)
                        .child("Loading…"),
                ),
            );
        }
        if self.history_cursor.is_some() && !self.loading {
            list = list.child(
                div().p_2().child(
                    Button::new("nc-load-more")
                        .ghost()
                        .label("Load more")
                        .on_click(cx.listener(|this, _, _, cx| this.load_more(cx))),
                ),
            );
        }

        div()
            .flex()
            .flex_col()
            .size_full()
            .child(
                div()
                    .flex()
                    .flex_row()
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
                            .child("Notifications"),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("nc-refresh")
                            .ghost()
                            .icon(IconName::RefreshCcwDot)
                            .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                    )
                    .child(
                        Button::new("nc-mark-all")
                            .ghost()
                            .icon(IconName::Check)
                            .label("Mark all read")
                            .on_click(cx.listener(|this, _, _, cx| this.mark_all_read(cx))),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap_1()
                    .px_4()
                    .py_2()
                    .border_b_1()
                    .border_color(c.border)
                    .children(Filter::ALL.iter().map(|f| {
                        let active = *f == self.filter;
                        div()
                            .id(("nc-filter", *f as usize))
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .text_sm()
                            .cursor_pointer()
                            .when(active, |d| d.bg(c.accent).text_color(c.accent_foreground))
                            .when(!active, |d| {
                                d.text_color(c.muted_foreground).hover(|s| s.bg(c.muted))
                            })
                            .child(f.label())
                            .on_click(cx.listener(move |this, _, _, cx| this.set_filter(*f, cx)))
                    })),
            )
            .when_some(self.error.clone(), |d, e| {
                d.child(
                    div()
                        .px_4()
                        .py_2()
                        .child(div().text_sm().text_color(danger).child(e)),
                )
            })
            .child(list)
    }
}

/// "just now" / "5m" / "3h" / "2d" / "2026-09-24"。
fn relative_time(dt: &DateTime<Utc>) -> String {
    let secs = (Utc::now() - *dt).num_seconds().max(0);
    if secs < 60 {
        "just now".into()
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86400 {
        format!("{}h", secs / 3600)
    } else if secs < 7 * 86400 {
        format!("{}d", secs / 86400)
    } else {
        dt.format("%Y-%m-%d").to_string()
    }
}

#[cfg(test)]
mod tests {
    // `use super::*` だと gpui::test が #[test] を shadow するので個別 import。
    use super::{Filter, relative_time};
    use api::spec::NotificationKind;
    use chrono::Utc;

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
    fn relative_time_buckets() {
        let now = Utc::now();
        assert_eq!(
            relative_time(&(now - chrono::Duration::seconds(10))),
            "just now"
        );
        assert_eq!(relative_time(&(now - chrono::Duration::minutes(5))), "5m");
        assert_eq!(relative_time(&(now - chrono::Duration::hours(3))), "3h");
        assert_eq!(relative_time(&(now - chrono::Duration::days(2))), "2d");
    }
}
