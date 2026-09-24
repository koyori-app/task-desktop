//! 一覧行の正規化モデル。MyTaskItem / TaskResponse の両方から作る。

use api::types::{LabelResponse, MyTaskItem, TaskPriority, TaskResponse};
use chrono::{DateTime, Local, NaiveDate, TimeZone, Utc};
use gpui_kit::Hsla;
use i18n::t;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct TaskRow {
    pub id: Uuid,
    pub project_id: Uuid,
    pub project_key: String,
    pub seq_key: String,
    pub title: String,
    pub priority: TaskPriority,
    pub status_id: Uuid,
    pub status_name: String,
    pub status_color: String,
    /// 表示用の期限（soft 優先、無ければ hard）。
    pub due: Option<DateTime<Utc>>,
    /// statuses ロード後に解決。未解決は false（スイッチ非表示扱い）。
    pub is_done: bool,
    /// 担当者。My Tasks の API は返さない（全て自分の担当）ので空。
    pub assignees: Vec<RowAssignee>,
    /// ラベル。My Tasks の API は返さないので空。
    pub labels: Vec<LabelResponse>,
    pub parent_task_id: Option<Uuid>,
}

#[derive(Debug, Clone)]
pub struct RowAssignee {
    pub id: Uuid,
    pub role: String,
    pub name: String,
    pub avatar_url: Option<String>,
}

/// 一覧・Detail で並べる優先度の順（Web と同じ）。
pub const PRIORITIES: [TaskPriority; 6] = [
    TaskPriority::CriticalFire,
    TaskPriority::Critical,
    TaskPriority::High,
    TaskPriority::Medium,
    TaskPriority::Low,
    TaskPriority::Trivial,
];

impl TaskRow {
    pub fn from_my(item: &MyTaskItem) -> Self {
        Self {
            id: item.id,
            project_id: item.project.id,
            project_key: item.project.key.clone(),
            seq_key: item.seq_key.clone(),
            title: item.title.clone(),
            priority: item.priority,
            status_id: item.status.id,
            status_name: item.status.name.clone(),
            status_color: item.status.color.clone(),
            due: item.soft_deadline.or(item.hard_deadline),
            is_done: false,
            assignees: vec![],
            labels: vec![],
            parent_task_id: None,
        }
    }

    /// Project 一覧。seq_key は API に無いので key+seq_id を呼び出し側で合成。
    pub fn from_task(item: &TaskResponse, project_key: &str) -> Self {
        Self {
            id: item.id,
            project_id: item.project_id,
            project_key: project_key.to_string(),
            seq_key: format!("{project_key}-{}", item.seq_id),
            title: item.title.clone(),
            priority: item.priority,
            status_id: item.status_id,
            status_name: String::new(),
            status_color: String::new(),
            due: item.soft_deadline.or(item.hard_deadline),
            is_done: false,
            assignees: item
                .assignees
                .iter()
                .map(|a| RowAssignee {
                    id: a.user.id,
                    role: a.role.clone(),
                    name: a.user.username.clone(),
                    avatar_url: a.user.avatar_url.clone(),
                })
                .collect(),
            labels: item.labels.clone(),
            parent_task_id: item.parent_task_id,
        }
    }
}

/// status.color は "#rrggbb" 文字列。パース失敗時は None。
pub fn parse_hex_color(s: &str) -> Option<gpui_kit::Hsla> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 {
        return None;
    }
    let rgb = u32::from_str_radix(h, 16).ok()?;
    Some(gpui_kit::rgb(rgb).into())
}

/// 期限の表記（Web の `formatDeadline` と同じ規則）:
/// 今日 / N日超過 / N日後（7 日以内）/ それより先は月日。
pub fn due_label(due: &DateTime<Utc>) -> String {
    due_label_on(due, Local::now().date_naive())
}

fn due_label_on(due: &DateTime<Utc>, today: NaiveDate) -> String {
    let d = due.with_timezone(&Local).date_naive();
    match (d - today).num_days() {
        0 => t!("tasks.due.today").into(),
        n if n < 0 => t!("tasks.due.overdue", days = -n),
        n if n <= 7 => t!("tasks.due.in_days", days = n),
        _ => d.format(t!("tasks.due.month_day")).to_string(),
    }
}

/// 期限切れか（今日の期限はまだ切れていない扱い）。
pub fn is_overdue(due: &DateTime<Utc>) -> bool {
    due.with_timezone(&Local).date_naive() < Local::now().date_naive()
}

/// API の enum 名（`CriticalFire` 等）をそのまま出さない表示名。
pub fn priority_label(priority: TaskPriority) -> &'static str {
    match priority {
        TaskPriority::CriticalFire => t!("tasks.priority.critical_fire"),
        TaskPriority::Critical => t!("tasks.priority.critical"),
        TaskPriority::High => t!("tasks.priority.high"),
        TaskPriority::Medium => t!("tasks.priority.medium"),
        TaskPriority::Low => t!("tasks.priority.low"),
        TaskPriority::Trivial => t!("tasks.priority.trivial"),
    }
}

/// 優先度の色（Web の `PRIORITY_CONFIG` と同じ値）。
pub fn priority_color(priority: TaskPriority) -> Hsla {
    let hex = match priority {
        TaskPriority::CriticalFire => 0xdc2626,
        TaskPriority::Critical => 0xef4444,
        TaskPriority::High => 0xf97316,
        TaskPriority::Medium => 0xeab308,
        TaskPriority::Low => 0x6b7280,
        TaskPriority::Trivial => 0x9ca3af,
    };
    gpui_kit::rgb(hex).into()
}

/// A date entered by the user belongs to their local calendar day.
pub fn due_timestamp(date: NaiveDate, timezone: &impl TimeZone) -> Option<DateTime<Utc>> {
    timezone
        .from_local_datetime(&date.and_hms_opt(0, 0, 0)?)
        .earliest()
        .map(|date| date.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_color_parses() {
        let c = parse_hex_color("#ff0000").unwrap();
        assert!(c.h.abs() < 0.01);
        assert!((c.s - 1.0).abs() < 0.01);
        assert!((c.l - 0.5).abs() < 0.01);
        assert!(parse_hex_color("nope").is_none());
        assert!(parse_hex_color("ああ").is_none());
        let green = parse_hex_color("#00ff00").unwrap();
        assert!((green.h - 1.0 / 3.0).abs() < 0.01);
        let blue = parse_hex_color("#0000ff").unwrap();
        assert!((blue.h - 2.0 / 3.0).abs() < 0.01);
    }

    #[test]
    fn due_label_matches_web_rules() {
        let today = Local::now().date_naive();
        let at = |days: i64| {
            Local
                .from_local_datetime(
                    &(today + chrono::Duration::days(days))
                        .and_hms_opt(12, 0, 0)
                        .unwrap(),
                )
                .earliest()
                .unwrap()
                .with_timezone(&Utc)
        };
        assert_eq!(due_label_on(&at(0), today), t!("tasks.due.today"));
        assert_eq!(
            due_label_on(&at(-3), today),
            t!("tasks.due.overdue", days = 3)
        );
        assert_eq!(
            due_label_on(&at(1), today),
            t!("tasks.due.in_days", days = 1)
        );
        assert_eq!(
            due_label_on(&at(7), today),
            t!("tasks.due.in_days", days = 7)
        );
        let later = at(30);
        assert_eq!(
            due_label_on(&later, today),
            later
                .with_timezone(&Local)
                .format(t!("tasks.due.month_day"))
                .to_string()
        );
        assert!(is_overdue(&at(-1)));
        assert!(!is_overdue(&at(0)));
    }

    #[test]
    fn priority_labels_are_human_readable() {
        assert_eq!(
            priority_label(TaskPriority::CriticalFire),
            t!("tasks.priority.critical_fire")
        );
        assert_ne!(
            priority_label(TaskPriority::CriticalFire),
            "tasks.priority.critical_fire"
        );
    }

    #[test]
    fn entered_due_day_roundtrips_east_and_west_of_utc() {
        for seconds in [9 * 3600, -7 * 3600] {
            let timezone = chrono::FixedOffset::east_opt(seconds).unwrap();
            let date = NaiveDate::from_ymd_opt(2026, 9, 24).unwrap();
            let due = due_timestamp(date, &timezone).unwrap();
            assert_eq!(
                due.with_timezone(&timezone).format("%Y-%m-%d").to_string(),
                "2026-09-24"
            );
        }
    }
}
