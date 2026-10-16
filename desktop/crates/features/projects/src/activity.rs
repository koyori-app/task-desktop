//! 履歴 1 件の表示文言（Web の `lib/task-activity.ts` と同じ規則）。
//! 未知の `event_type` は落とさず「操作しました」で出す。

use api::types::ActivityItem;
use i18n::t;

pub fn activity_text(item: &ActivityItem) -> String {
    let str_of = |key: &str| item.payload.get(key).and_then(|v| v.as_str());
    match item.event_type.as_str() {
        "task_created" => t!("projects.activity.task_created").into(),
        "github_issue_imported" => t!("projects.activity.github_imported").into(),
        "github_issue_synced" => t!("projects.activity.github_synced").into(),
        "status_changed" => match str_of("to") {
            Some(to) => t!("projects.activity.status_to", to = to),
            None => t!("projects.activity.status").into(),
        },
        "priority_changed" => match str_of("to") {
            Some(to) => t!("projects.activity.priority_to", to = to),
            None => t!("projects.activity.priority").into(),
        },
        "deadline_changed" => {
            let hard = str_of("field") == Some("hard_deadline");
            match (str_of("to"), hard) {
                (Some(to), true) => t!("projects.activity.hard_deadline_to", to = date_part(to)),
                (Some(to), false) => t!("projects.activity.deadline_to", to = date_part(to)),
                (None, true) => t!("projects.activity.hard_deadline_cleared").into(),
                (None, false) => t!("projects.activity.deadline_cleared").into(),
            }
        }
        "assignee_added" => t!("projects.activity.assignee_added").into(),
        "assignee_removed" => t!("projects.activity.assignee_removed").into(),
        "relation_added" => t!("projects.activity.relation_added").into(),
        "relation_removed" => t!("projects.activity.relation_removed").into(),
        "comment_added" => t!("projects.activity.comment_added").into(),
        "comment_edited" => t!("projects.activity.comment_edited").into(),
        "comment_deleted" => t!("projects.activity.comment_deleted").into(),
        "archived" => t!("projects.activity.archived").into(),
        "forge_commit_linked" => match str_of("sha") {
            Some(sha) => t!(
                "projects.activity.commit_linked",
                sha = sha.get(..7).unwrap_or(sha)
            ),
            None => t!("projects.activity.other").into(),
        },
        _ => t!("projects.activity.other").into(),
    }
}

/// 主語。ユーザーに結べない履歴（コミット等）はホスト上の作者名、それも無ければ「システム」。
pub fn activity_actor(item: &ActivityItem) -> String {
    if let Some(user) = &item.user {
        return user.name.clone();
    }
    ["author_handle", "author_name"]
        .iter()
        .find_map(|k| item.payload.get(*k).and_then(|v| v.as_str()))
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| t!("projects.activity.system").into())
}

fn date_part(s: &str) -> &str {
    s.get(..10).unwrap_or(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(event: &str, payload: serde_json::Value) -> ActivityItem {
        serde_json::from_value(serde_json::json!({
            "id": uuid::Uuid::nil(), "event_type": event, "payload": payload,
            "created_at": "2026-09-01T00:00:00Z",
        }))
        .unwrap()
    }

    #[test]
    fn known_and_unknown_events_have_text() {
        assert_eq!(
            activity_text(&item("status_changed", serde_json::json!({"to": "Done"}))),
            t!("projects.activity.status_to", to = "Done")
        );
        assert_eq!(
            activity_text(&item(
                "deadline_changed",
                serde_json::json!({"field": "hard_deadline", "to": "2026-09-24T00:00:00Z"})
            )),
            t!("projects.activity.hard_deadline_to", to = "2026-09-24")
        );
        assert_eq!(
            activity_text(&item("something_new", serde_json::json!({}))),
            t!("projects.activity.other")
        );
        assert_eq!(
            activity_actor(&item(
                "forge_commit_linked",
                serde_json::json!({"author_name": "bob"})
            )),
            "bob"
        );
    }
}
