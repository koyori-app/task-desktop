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
    initialized: bool,
    activation: Option<std::sync::Arc<dyn Fn(NotificationItem) + Send + Sync>>,
}

impl NotificationEngine {
    /// `saved_cursor`: 設定に保存しておいた高水位（初回起動は None）。
    pub fn new(
        client: api::Client,
        notifier: platform::Notifier,
        saved_cursor: Option<String>,
    ) -> Self {
        let initialized = saved_cursor.is_some();
        Self {
            client,
            notifier,
            last_cursor: saved_cursor,
            seen: HashSet::new(),
            seen_order: std::collections::VecDeque::new(),
            initialized,
            activation: None,
        }
    }

    pub fn last_cursor(&self) -> Option<&str> {
        self.last_cursor.as_deref()
    }

    /// OS callbacks run outside GPUI; send the item to the app's UI event queue.
    pub fn on_activation(
        mut self,
        callback: impl Fn(NotificationItem) + Send + Sync + 'static,
    ) -> Self {
        self.activation = Some(std::sync::Arc::new(callback));
        self
    }

    fn notify(&self, items: &[NotificationItem], prefs: &NotificationPrefs) -> usize {
        items
            .iter()
            .filter(|item| {
                if !enabled_for(prefs, &item.notification_type) {
                    return false;
                }
                let content = toast(item);
                if let Some(callback) = &self.activation {
                    let callback = callback.clone();
                    let item = (*item).clone();
                    self.notifier
                        .show_with_activation(&content, move || callback(item.clone()))
                        .is_ok()
                } else {
                    self.notifier.show(&content).is_ok()
                }
            })
            .count()
    }

    fn mark_seen(&mut self, id: uuid::Uuid) -> bool {
        if self.seen.insert(id) {
            self.seen_order.push_back(id);
            if self.seen_order.len() > SEEN_CAPACITY
                && let Some(old) = self.seen_order.pop_front()
            {
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
            None => self.baseline(prefs).await,
            Some(cursor) => self.catch_up(cursor.clone(), prefs).await,
        }
    }

    /// 初回（カーソル無し）: 最新 1 ページで高水位だけ確立し、toast は出さない。
    /// 空の初回同期を終えた後は、新着の全ページを受信してから高水位を進める。
    async fn baseline(&mut self, prefs: &NotificationPrefs) -> Result<TickOutcome, ApiError> {
        let page = self
            .fetch(&NotificationsQuery {
                limit: Some(PAGE_LIMIT),
                ..Default::default()
            })
            .await?;
        let baseline_only = !self.initialized;
        let mut items = page.notifications;
        let mut unread_count = page.unread_count;
        let mut next_cursor = page.next_cursor;
        let mut page_cursors = HashSet::new();
        if !baseline_only {
            while let Some(cursor) = next_cursor {
                if !page_cursors.insert(cursor.clone()) {
                    return Err(ApiError::InvalidConfig(
                        "notification API repeated a page cursor".into(),
                    ));
                }
                let page = self
                    .fetch(&NotificationsQuery {
                        limit: Some(PAGE_LIMIT),
                        cursor: Some(cursor),
                        ..Default::default()
                    })
                    .await?;
                unread_count = page.unread_count;
                let page_was_empty = page.notifications.is_empty();
                items.extend(page.notifications);
                next_cursor = if page_was_empty {
                    None
                } else {
                    page.next_cursor
                };
            }
        }
        // No state is committed until all requested pages succeed, so a failed
        // fetch retries the complete first batch instead of silently losing it.
        self.initialized = true;
        items.sort_by_key(|item| (item.created_at, item.id));
        let mut ids = HashSet::new();
        items.retain(|item| ids.insert(item.id));
        for item in &items {
            self.mark_seen(item.id);
        }
        self.last_cursor = items.last().and_then(row_cursor);
        let new_items = if baseline_only { vec![] } else { items };
        let notified = self.notify(&new_items, prefs);
        Ok(TickOutcome {
            unread_count,
            new_items,
            notified,
            baseline_only,
            last_cursor: self.last_cursor.clone(),
        })
    }

    /// 通常回: `after=` で差分を全部取る（ASC で返る）。
    async fn catch_up(
        &mut self,
        cursor: String,
        prefs: &NotificationPrefs,
    ) -> Result<TickOutcome, ApiError> {
        let mut after = cursor;
        let mut new_items: Vec<NotificationItem> = vec![];
        let mut unread_count;
        let mut page_cursors = HashSet::new();
        let mut pending_ids = HashSet::new();

        loop {
            if !page_cursors.insert(after.clone()) {
                return Err(ApiError::InvalidConfig(
                    "notification API repeated a catch-up cursor".into(),
                ));
            }
            let page = self
                .fetch(&NotificationsQuery {
                    limit: Some(PAGE_LIMIT),
                    after: Some(after.clone()),
                    ..Default::default()
                })
                .await?;
            unread_count = page.unread_count;
            let page_was_empty = page.notifications.is_empty();
            for item in page.notifications {
                // Commit seen IDs only after all pages succeeded. Otherwise a
                // retry would silently discard the pages fetched before failure.
                if !self.seen.contains(&item.id) && pending_ids.insert(item.id) {
                    new_items.push(item);
                }
            }
            match page.next_cursor {
                Some(next) if !page_was_empty => after = next,
                _ => break,
            }
        }

        new_items.sort_by_key(|item| (item.created_at, item.id));
        for item in &new_items {
            self.mark_seen(item.id);
        }
        // 高水位 = 最後に受け取った（最新の）行のカーソル。
        if let Some(c) = new_items.last().and_then(row_cursor) {
            self.last_cursor = Some(c);
        }

        let notified = self.notify(&new_items, prefs);

        Ok(TickOutcome {
            unread_count,
            new_items,
            notified,
            baseline_only: false,
            last_cursor: self.last_cursor.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    fn notification(number: u128) -> serde_json::Value {
        serde_json::json!({"id": uuid::Uuid::from_u128(number), "notification_type": "assigned",
            "created_at": "2026-09-24T00:00:00Z", "cursor": format!("c{number}")})
    }

    fn page(number: u128, next: Option<String>) -> serde_json::Value {
        serde_json::json!({"notifications": [notification(number)], "unread_count": 12, "next_cursor": next})
    }

    fn scripted_server(
        responses: Vec<(u16, serde_json::Value)>,
    ) -> (api::Client, std::thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let client = api::Client::new(
            &format!("http://{}/api", listener.local_addr().unwrap()),
            "dev",
        )
        .unwrap();
        let server = std::thread::spawn(move || {
            for (status, body) in responses {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(10)))
                    .unwrap();
                let mut buf = [0; 4096];
                let read = stream.read(&mut buf).unwrap();
                assert!(read > 0);
                let body = body.to_string();
                write!(stream, "HTTP/1.1 {status} Result\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
        });
        (client, server)
    }

    fn prefs() -> NotificationPrefs {
        NotificationPrefs {
            enabled: false,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn retries_all_pages_after_partial_failure() {
        let (client, server) = scripted_server(vec![
            (200, page(1, Some("c1".into()))),
            (500, serde_json::json!({"message": "temporary"})),
            (200, page(1, Some("c1".into()))),
            (200, page(2, None)),
        ]);
        let mut engine =
            NotificationEngine::new(client, platform::Notifier::new("test"), Some("c0".into()));
        assert!(engine.tick(&prefs()).await.is_err());
        assert_eq!(engine.last_cursor(), Some("c0"));
        let outcome = engine.tick(&prefs()).await.unwrap();
        assert_eq!(outcome.new_items.len(), 2);
        assert_eq!(outcome.last_cursor.as_deref(), Some("c2"));
        server.join().unwrap();
    }

    #[tokio::test]
    async fn drains_more_than_ten_pages() {
        let (client, server) = scripted_server(
            (1..=12)
                .map(|n| (200, page(n, (n < 12).then(|| format!("c{n}")))))
                .collect(),
        );
        let mut engine =
            NotificationEngine::new(client, platform::Notifier::new("test"), Some("c0".into()));
        assert_eq!(engine.tick(&prefs()).await.unwrap().new_items.len(), 12);
        server.join().unwrap();
    }

    #[tokio::test]
    async fn empty_initial_account_still_receives_its_first_notification() {
        let (client, server) = scripted_server(vec![
            (
                200,
                serde_json::json!({"notifications": [], "unread_count": 0}),
            ),
            (200, page(1, None)),
        ]);
        let mut engine = NotificationEngine::new(client, platform::Notifier::new("test"), None);
        assert!(engine.tick(&prefs()).await.unwrap().baseline_only);
        let next = engine.tick(&prefs()).await.unwrap();
        assert!(!next.baseline_only);
        assert_eq!(next.new_items.len(), 1);
        server.join().unwrap();
    }

    #[tokio::test]
    async fn empty_baseline_drains_the_entire_first_batch() {
        let (client, server) = scripted_server(vec![
            (
                200,
                serde_json::json!({"notifications": [], "unread_count": 0}),
            ),
            (200, page(3, Some("older-c3".into()))),
            (
                200,
                serde_json::json!({"notifications": [notification(2), notification(1)], "unread_count": 3}),
            ),
        ]);
        let mut engine = NotificationEngine::new(client, platform::Notifier::new("test"), None);
        assert!(engine.tick(&prefs()).await.unwrap().baseline_only);
        let outcome = engine.tick(&prefs()).await.unwrap();
        assert_eq!(
            outcome
                .new_items
                .iter()
                .map(|n| n.id.as_u128())
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert_eq!(outcome.last_cursor.as_deref(), Some("c3"));
        assert_eq!(outcome.unread_count, 3);
        server.join().unwrap();
    }

    #[tokio::test]
    async fn existing_baseline_is_suppressed_and_sets_the_newest_cursor() {
        let (client, server) = scripted_server(vec![
            (
                200,
                serde_json::json!({"notifications": [notification(2), notification(1)], "unread_count": 2}),
            ),
            (200, page(3, None)),
        ]);
        let mut engine = NotificationEngine::new(client, platform::Notifier::new("test"), None);
        let baseline = engine.tick(&prefs()).await.unwrap();
        assert!(baseline.baseline_only);
        assert!(baseline.new_items.is_empty());
        assert_eq!(baseline.notified, 0);
        assert_eq!(baseline.last_cursor.as_deref(), Some("c2"));
        let next = engine.tick(&prefs()).await.unwrap();
        assert_eq!(next.new_items.len(), 1);
        assert_eq!(next.last_cursor.as_deref(), Some("c3"));
        server.join().unwrap();
    }

    #[tokio::test]
    async fn repeated_cursor_fails_without_advancing_or_marking_items_seen() {
        let (client, server) = scripted_server(vec![(200, page(1, Some("c0".into())))]);
        let mut engine =
            NotificationEngine::new(client, platform::Notifier::new("test"), Some("c0".into()));
        assert!(matches!(
            engine.tick(&prefs()).await,
            Err(ApiError::InvalidConfig(_))
        ));
        assert_eq!(engine.last_cursor(), Some("c0"));
        assert!(engine.seen.is_empty());
        server.join().unwrap();
    }
}
