//! task.md のうち production の openapi.json にまだ出ていない型。
//! backend が実装したら openapi.json を更新して types:: 側へ移し、ここは消す。
//!
//! 移行対象の残り:
//! - notifications の `project` / `target` / `next_cursor`（task.md §3, §9）
//! - Device Token 認証（§17）・端末管理
//! - Finding の `available_actions`（§18.1）・Summary の `gate`（§18.2）

use serde::{Deserialize, Serialize};

pub use crate::types::{
    FindingSeverity, FindingState, FindingTransitionResponse, SeverityStateCount,
};

// ---- notifications (task.md §3, §9) ----

#[derive(Debug, Clone, Deserialize)]
pub struct NotificationList {
    pub unread_count: i64,
    #[serde(default)]
    pub next_cursor: Option<String>,
    #[serde(default)]
    pub notifications: Vec<NotificationItem>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NotificationItem {
    pub id: uuid::Uuid,
    pub notification_type: String,
    /// task.md §9.1 で追加。本番が返すまでは None。
    #[serde(default)]
    pub project: Option<NotificationProject>,
    #[serde(default)]
    pub task: Option<NotificationTaskSummary>,
    #[serde(default)]
    pub payload: serde_json::Value,
    /// task.md §4 で追加。遷移はここだけを見る。
    #[serde(default)]
    pub target: Option<NotificationTarget>,
    /// この行を指すカーソル。catch-up の高水位マークに使う
    /// （本番が返すまでは None → (created_at, id) から組み立てる）。
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub read_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NotificationTaskSummary {
    pub id: uuid::Uuid,
    pub seq_id: i64,
    pub title: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NotificationProject {
    pub tenant_id: uuid::Uuid,
    pub id: uuid::Uuid,
    pub key: String,
}

/// task.md §4。内部ルートへの変換はこの値だけで行う。
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NotificationTarget {
    Task {
        task_id: uuid::Uuid,
    },
    Review {
        review_id: uuid::Uuid,
    },
    ReviewFinding {
        review_id: uuid::Uuid,
        finding_id: uuid::Uuid,
        #[serde(default)]
        deferred_task_id: Option<uuid::Uuid>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationKind {
    Task,
    Review,
}

impl NotificationKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Task => "task",
            Self::Review => "review",
        }
    }
}

// ---- desktop auth (task.md §17.2) ----

#[derive(Debug, Serialize)]
pub struct DesktopAuthTokenRequest {
    pub code: String,
    pub code_verifier: String,
}

#[derive(Debug, Deserialize)]
pub struct DesktopAuthTokenResponse {
    /// `kdt_` 接頭辞の Device Token。
    pub token: String,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

// ---- devices (task.md §17.3) ----

#[derive(Debug, Clone, Deserialize)]
pub struct DeviceList {
    #[serde(default)]
    pub devices: Vec<Device>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Device {
    pub id: uuid::Uuid,
    pub name: String,
    #[serde(default)]
    pub last_used_at: Option<chrono::DateTime<chrono::Utc>>,
    pub expires_at: chrono::DateTime<chrono::Utc>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

// ---- review extensions (task.md §18) ----

/// `GET …/reviews/summary` + task.md §18.2 の `gate`。
/// 生成型 `ReviewSummaryResponse` と同じ形 + Option の gate。
#[derive(Debug, Clone, Deserialize)]
pub struct ReviewSummary {
    pub pr_number: i64,
    #[serde(default)]
    pub repository: Option<String>,
    pub rounds: i64,
    #[serde(default)]
    pub counts: Vec<SeverityStateCount>,
    pub blocking: i64,
    #[serde(default)]
    pub owner_override_rejections: i64,
    pub mergeable: bool,
    #[serde(default)]
    pub latest_head_sha: Option<String>,
    #[serde(default)]
    pub cached_pr_head_sha: Option<String>,
    #[serde(default)]
    pub pr_head_checked_at: Option<chrono::DateTime<chrono::Utc>>,
    /// backend 未実装の間は None（"Freshness unknown" 相当として扱わない。
    /// 未実装検知自体が表示側の仕事）。
    #[serde(default)]
    pub gate: Option<Gate>,
}

/// task.md §18.2。値は backend が決め、Desktop は再計算しない。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Gate {
    Unlinked,
    Unreviewed,
    Blocked,
    StaleUnknown,
    Outdated,
    Ready,
}

/// 生成型 `FindingResponse` + task.md §18.1 の `available_actions`。
/// backend 未実装の間は空（= Action を出さない、仕様どおり）。
#[derive(Debug, Clone, Deserialize)]
pub struct Finding {
    pub id: uuid::Uuid,
    pub review_id: uuid::Uuid,
    pub pr_number: i64,
    pub round: i64,
    pub severity: FindingSeverity,
    pub title: String,
    pub body: String,
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default)]
    pub line: Option<i64>,
    pub state: FindingState,
    #[serde(default)]
    pub fixed_by: Option<uuid::Uuid>,
    #[serde(default)]
    pub deferred_task_id: Option<uuid::Uuid>,
    #[serde(default)]
    pub transitions: Vec<FindingTransitionResponse>,
    /// 要求者がいま遷移できる state の一覧（task.md §18.1）。
    #[serde(default)]
    pub available_actions: Vec<FindingState>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notification_item_parses_task_md_example() {
        // task.md §9.1 の例
        let json = serde_json::json!({
            "id": "8d2581e1-803a-4fbe-90e0-eda144642c0b",
            "notification_type": "review.finding_fixed",
            "project": { "tenant_id": "8d2581e1-803a-4fbe-90e0-eda144642c0b", "id": "8d2581e1-803a-4fbe-90e0-eda144642c0b", "key": "TASK" },
            "task": null,
            "payload": { "actor": "yupix", "pr_number": 742, "round": 2 },
            "target": { "type": "review_finding", "review_id": "8d2581e1-803a-4fbe-90e0-eda144642c0b", "finding_id": "8d2581e1-803a-4fbe-90e0-eda144642c0b" },
            "read_at": null,
            "created_at": "2026-09-24T00:00:00Z"
        });
        let item: NotificationItem = serde_json::from_value(json).unwrap();
        assert_eq!(item.notification_type, "review.finding_fixed");
        match item.target.unwrap() {
            NotificationTarget::ReviewFinding {
                deferred_task_id, ..
            } => {
                assert!(deferred_task_id.is_none())
            }
            other => panic!("unexpected target {other:?}"),
        }
    }

    #[test]
    fn notification_list_tolerates_missing_extension_fields() {
        // 現行本番の形（project / target / next_cursor 無し）でも落ちない
        let json = serde_json::json!({
            "unread_count": 3,
            "notifications": [{
                "id": "8d2581e1-803a-4fbe-90e0-eda144642c0b",
                "notification_type": "assigned",
                "payload": {},
                "read_at": null,
                "created_at": "2026-09-24T00:00:00Z"
            }]
        });
        let list: NotificationList = serde_json::from_value(json).unwrap();
        assert_eq!(list.unread_count, 3);
        assert!(list.next_cursor.is_none());
        assert!(list.notifications[0].project.is_none());
        assert!(list.notifications[0].target.is_none());
    }

    #[test]
    fn gate_deserializes_snake_case() {
        assert_eq!(
            serde_json::from_value::<Gate>(serde_json::json!("stale_unknown")).unwrap(),
            Gate::StaleUnknown
        );
    }
}
