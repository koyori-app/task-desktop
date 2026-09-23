//! 通知同期エンジン（desktop.md §7.1 / task.md §9-10）。
//! `after` カーソルによる catch-up ポーリング。初回は OS 通知を出さない
//! （起動直後に過去分が一斉に鳴るのを防ぐ、§7.1）。

use std::collections::HashSet;

use api::spec::{NotificationItem, NotificationList};
use api::{ApiError, NotificationsQuery};
use base64::Engine;

use crate::notify_text::{enabled_for, toast};
use crate::settings::NotificationPrefs;

/// `limit` 既定（task.md §9.1）。
const PAGE_LIMIT: u32 = 50;
/// 無限ループ防止の catch-up ページ上限。
const MAX_PAGES: u32 = 10;
/// 見た id の記憶数（prod が cursor を返さない時期の重複抑止用）。
const SEEN_CAPACITY: usize = 500;

/// カーソル非対応の backend に対して、行の (created_at, id) から
/// `after` 用カーソルを組み立てる。backend の NotificationCursor
/// `{"created_at": …, "id": …}` と同じ形（common::cursor）。
fn row_cursor(item: &NotificationItem) -> Option<String> {
    if let Some(c) = &item.cursor {
        return Some(c.clone());
    }
    let v = serde_json::json!({
        "created_at": item.created_at,
        "id": item.id,
    });
    Some(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(v.to_string()))
}

pub struct TickOutcome {
    /// レスポンスの未読総数（最新ページの値）。
    pub unread_count: i64,
    /// 今回新たに受け取った行（古い順。OS 通知はこの順で出す）。
    pub new_items: Vec<NotificationItem>,
    /// OS 通知を実際に出した件数（トグル OFF や初回抑制を除いた数）。
    pub notified: usize,
    /// 初回同期（baseline 確立だけで新着扱いしない）。
    pub baseline_only: bool,
    /// 更新後の高水位カーソル。設定へ永続化する値。
    pub last_cursor: Option<String>,
}

/// catch-up の状態機械。Cursor は engine 内に持ち、呼び出し側は
/// `last_cursor()` を設定へ保存すればよい。
pub struct NotificationEngine {
    client: api::Client,
    notifier: platform::Notifier,
    last_cursor: Option<String>,
    /// prod（cursor 非対応）の間の重複抑止。cursor 対応後も害はない。
    seen: HashSet<uuid::Uuid>,
    seen_order: std::collections::VecDeque<uuid::Uuid>,
}

impl NotificationEngine {
    /// `saved_cursor`: 設定に保存しておいた高水位（初回起動は None）。
    pub fn new(
        client: api::Client,
        notifier: platform::Notifier,
        saved_cursor: Option<String>,
    ) -> Self {
        Self {
            client,
            notifier,
            last_cursor: saved_cursor,
            seen: HashSet::new(),
            seen_order: std::collections::VecDeque::new(),
        }
    }

    pub fn last_cursor(&self) -> Option<&str> {
        self.last_cursor.as_deref()
    }

    fn mark_seen(&mut self, id: uuid::Uuid) -> bool {
        if self.seen.insert(id) {
            self.seen_order.push_back(id);
            if self.seen_order.len() > SEEN_CAPACITY
                && let Some(old) = self.seen_order.pop_front() {
                    self.seen.remove(&old);
                }
            true
        } else {
            false
        }
    }

    async fn fetch(&self, q: &NotificationsQuery) -> Result<NotificationList, ApiError> {
        self.client.list_notifications(q).await
    }

    /// 1 回分の同期（§7.1）。`prefs` で OS 通知の出し分けをする。
    /// 呼び出し側は 30 秒ごとに呼び、outcome を UI へ反映する。
    /// 通信障害は `Err(ApiError::Transport)` — Connection Status の扱い（§23）。
    pub async fn tick(&mut self, prefs: &NotificationPrefs) -> Result<TickOutcome, ApiError> {
        match &self.last_cursor {
            None => self.baseline().await,
            Some(cursor) => self.catch_up(cursor.clone(), prefs).await,
        }
    }

    /// 初回（カーソル無し）: 最新 1 ページで高水位だけ確立し、toast は出さない。
    async fn baseline(&mut self) -> Result<TickOutcome, ApiError> {
        let page = self
            .fetch(&NotificationsQuery {
                limit: Some(PAGE_LIMIT),
                ..Default::default()
            })
            .await?;
        for item in &page.notifications {
            self.mark_seen(item.id);
        }
        // 先頭行が最新（DESC）。そこを高水位にする。
        self.last_cursor = page.notifications.first().and_then(row_cursor);
        Ok(TickOutcome {
            unread_count: page.unread_count,
            new_items: vec![],
            notified: 0,
            baseline_only: true,
            last_cursor: self.last_cursor.clone(),
        })
    }

    /// 通常回: `after=` で差分を全部取る（ASC で返る）。
    async fn catch_up(
        &mut self,
        cursor: String,
        prefs: &NotificationPrefs,
    ) -> Result<TickOutcome, ApiError> {
        let mut after = Some(cursor);
        let mut new_items: Vec<NotificationItem> = vec![];
        let mut unread_count = 0i64;

        for _ in 0..MAX_PAGES {
            let page = self
                .fetch(&NotificationsQuery {
                    limit: Some(PAGE_LIMIT),
                    after,
                    ..Default::default()
                })
                .await?;
            unread_count = page.unread_count;
            let page_was_empty = page.notifications.is_empty();
            for item in page.notifications {
                if self.mark_seen(item.id) {
                    new_items.push(item);
                }
            }
            match page.next_cursor {
                Some(next) if !page_was_empty => after = Some(next),
                _ => break,
            }
        }

        // 高水位 = 最後に受け取った（最新の）行のカーソル。
        if let Some(c) = new_items.last().and_then(row_cursor) {
            self.last_cursor = Some(c);
        }

        let mut notified = 0;
        for item in &new_items {
            if enabled_for(prefs, &item.notification_type) {
                // 通知失敗で同期自体は止めない（§23: 軽微なエラーは Toast 運用）。
                if self.notifier.show(&toast(item)).is_ok() {
                    notified += 1;
                }
            }
        }

        Ok(TickOutcome {
            unread_count,
            new_items,
            notified,
            baseline_only: false,
            last_cursor: self.last_cursor.clone(),
        })
    }
}
