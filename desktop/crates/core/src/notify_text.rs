//! 通知行 → OS 通知文言（desktop.md §10）。
//! Backend は文言を返さないので、notification_type + payload + target から
//! クライアントが組み立てる。Finding の title / body は出さない（機密になり得る）。

use api::spec::NotificationItem;
use platform::OsNotification;

/// 種別（§13）。`review.` 接頭辞の有無で分ける（task.md §9 kind と同じ規則）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Task,
    Review,
    Due,
}

pub fn toast_kind(notification_type: &str) -> ToastKind {
    if notification_type.starts_with("review.") {
        ToastKind::Review
    } else if notification_type == "deadline_soon" {
        ToastKind::Due
    } else {
        ToastKind::Task
    }
}

/// OS 通知に出すか（§10: enabled && 種別トグル）。OFF でも Center には出る。
pub fn enabled_for(prefs: &crate::settings::NotificationPrefs, notification_type: &str) -> bool {
    prefs.enabled
        && match toast_kind(notification_type) {
            ToastKind::Task => prefs.task,
            ToastKind::Review => prefs.review,
            ToastKind::Due => prefs.due_date,
        }
}

fn payload_str<'a>(payload: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    payload.get(key)?.as_str()
}

fn payload_i64(payload: &serde_json::Value, key: &str) -> Option<i64> {
    payload.get(key)?.as_i64()
}

fn title_for(notification_type: &str) -> String {
    let known = match notification_type {
        "assigned" => "Task assigned",
        "mentioned" => "Mentioned in task",
        "status_changed" => "Task status changed",
        "comment_added" => "New comment",
        "deadline_soon" => "Due date approaching",
        "review.round_created" => "Review round created",
        "review.finding_fixed" => "Review finding fixed",
        "review.finding_reopened" => "Finding reopened",
        "review.finding_verified" => "Finding verified",
        "review.finding_deferred" => "Finding deferred",
        "review.outdated" => "Review is outdated",
        _ => "",
    };
    if !known.is_empty() {
        return known.to_string();
    }
    // 未知の種別は `review.finding_x` → `Finding x` のように読み替える
    let s = notification_type.replace(['.', '_'], " ");
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => "Notification".into(),
    }
}

/// 1 件分の表示文言。§10 の例:
/// `Review finding fixed` / `PR #742 · R2 · yupix`
pub fn toast(item: &NotificationItem) -> OsNotification {
    let payload = &item.payload;
    let mut parts: Vec<String> = vec![];

    if let Some(pr) = payload_i64(payload, "pr_number") {
        let mut s = format!("PR #{pr}");
        if let Some(r) = payload_i64(payload, "round") {
            s += &format!(" · R{r}");
        }
        parts.push(s);
    }
    if let Some(task) = &item.task {
        let key = item
            .project
            .as_ref()
            .map(|p| format!("{}-{}", p.key, task.seq_id))
            .unwrap_or_else(|| task.seq_id.to_string());
        parts.push(format!("{key} {}", task.title));
    }
    if let Some(actor) = payload_str(payload, "actor") {
        parts.push(actor.to_string());
    }
    // round_created は件数が payload に入る（§10）
    if let Some(n) = payload_i64(payload, "finding_count").or_else(|| payload_i64(payload, "count"))
    {
        parts.push(format!("{n} findings"));
    }

    OsNotification {
        title: title_for(&item.notification_type),
        body: parts.join(" · "),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::spec::{NotificationItem, NotificationProject};

    fn item(ty: &str, payload: serde_json::Value) -> NotificationItem {
        NotificationItem {
            id: uuid::Uuid::new_v4(),
            notification_type: ty.into(),
            project: Some(NotificationProject {
                tenant_id: uuid::Uuid::new_v4(),
                id: uuid::Uuid::new_v4(),
                key: "TASK".into(),
            }),
            task: None,
            payload,
            target: None,
            cursor: None,
            read_at: None,
            created_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn review_toast_matches_spec_example() {
        // §10 の例そのまま
        let n = toast(&item(
            "review.finding_fixed",
            serde_json::json!({"actor": "yupix", "pr_number": 742, "round": 2}),
        ));
        assert_eq!(n.title, "Review finding fixed");
        assert_eq!(n.body, "PR #742 · R2 · yupix");
    }

    #[test]
    fn task_toast_uses_task_summary() {
        let mut i = item("assigned", serde_json::json!({"actor": "yupix"}));
        i.task = Some(api::spec::NotificationTaskSummary {
            id: uuid::Uuid::new_v4(),
            seq_id: 482,
            title: "Fix login".into(),
        });
        let n = toast(&i);
        assert_eq!(n.body, "TASK-482 Fix login · yupix");
    }

    #[test]
    fn toggles_gate_by_kind() {
        let prefs = crate::settings::NotificationPrefs {
            enabled: true,
            task: true,
            review: false,
            due_date: true,
        };
        assert!(enabled_for(&prefs, "assigned"));
        assert!(!enabled_for(&prefs, "review.finding_fixed"));
        assert!(enabled_for(&prefs, "deadline_soon"));
        assert!(matches!(toast_kind("review.x"), ToastKind::Review));
    }
}
