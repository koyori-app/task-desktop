//! 一覧行の正規化モデル。MyTaskItem / TaskResponse の両方から作る。

use api::types::{MyTaskItem, TaskPriority, TaskResponse};
use chrono::{DateTime, Local, NaiveDate, TimeZone, Utc};
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
}

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

/// "today" / "tomorrow" / "2026-09-24" / "3d ago" 程度の簡易表記。
pub fn due_label(due: &DateTime<Utc>) -> String {
    let today = Local::now().date_naive();
    let d = due.with_timezone(&Local).date_naive();
    match (d - today).num_days() {
        0 => "today".into(),
        1 => "tomorrow".into(),
        n if n < 0 => format!("{}d overdue", -n),
        _ => d.format("%Y-%m-%d").to_string(),
    }
}

/// 期限の強調度。一覧で overdue / today を色分けする。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DueTone {
    Overdue,
    Today,
    Later,
}

pub fn due_tone(due: &DateTime<Utc>) -> DueTone {
    let today = Local::now().date_naive();
    let d = due.with_timezone(&Local).date_naive();
    match d.cmp(&today) {
        std::cmp::Ordering::Less => DueTone::Overdue,
        std::cmp::Ordering::Equal => DueTone::Today,
        std::cmp::Ordering::Greater => DueTone::Later,
    }
}

/// API の enum 名（`CriticalFire` 等）をそのまま出さない表示名。
pub fn priority_label(priority: TaskPriority) -> &'static str {
    match priority {
        TaskPriority::CriticalFire => "Critical 🔥",
        TaskPriority::Critical => "Critical",
        TaskPriority::High => "High",
        TaskPriority::Medium => "Medium",
        TaskPriority::Low => "Low",
        TaskPriority::Trivial => "Trivial",
    }
}

/// 一覧で目立たせる優先度。
pub fn priority_is_urgent(priority: TaskPriority) -> bool {
    matches!(
        priority,
        TaskPriority::CriticalFire | TaskPriority::Critical
    )
}

/// A date entered by the user belongs to their local calendar day.
pub fn due_timestamp(value: &str, timezone: &impl TimeZone) -> Option<DateTime<Utc>> {
    let date = NaiveDate::parse_from_str(value, "%Y-%m-%d").ok()?;
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
    fn due_tone_follows_local_calendar_day() {
        let now = Utc::now();
        assert_eq!(due_tone(&now), DueTone::Today);
        assert_eq!(
            due_tone(&(now - chrono::Duration::days(2))),
            DueTone::Overdue
        );
        assert_eq!(due_tone(&(now + chrono::Duration::days(2))), DueTone::Later);
    }

    #[test]
    fn priority_labels_are_human_readable() {
        assert_eq!(priority_label(TaskPriority::CriticalFire), "Critical 🔥");
        assert!(priority_is_urgent(TaskPriority::Critical));
        assert!(!priority_is_urgent(TaskPriority::High));
    }

    #[test]
    fn entered_due_day_roundtrips_east_and_west_of_utc() {
        for seconds in [9 * 3600, -7 * 3600] {
            let timezone = chrono::FixedOffset::east_opt(seconds).unwrap();
            let due = due_timestamp("2026-09-24", &timezone).unwrap();
            assert_eq!(
                due.with_timezone(&timezone).format("%Y-%m-%d").to_string(),
                "2026-09-24"
            );
        }
        assert!(due_timestamp("2026-02-30", &Utc).is_none());
    }
}
