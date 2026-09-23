//! 一覧行の正規化モデル。MyTaskItem / TaskResponse の両方から作る。

use api::types::{MyTaskItem, TaskPriority, TaskResponse};
use chrono::{DateTime, Utc};
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
            priority: item.priority.clone(),
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
            priority: item.priority.clone(),
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
    let r = u8::from_str_radix(&h[0..2], 16).ok()?;
    let g = u8::from_str_radix(&h[2..4], 16).ok()?;
    let b = u8::from_str_radix(&h[4..6], 16).ok()?;
    Some(gpui_kit::hsla(
        (r as f32) / 255.0,
        (g as f32) / 255.0,
        (b as f32) / 255.0,
        1.0,
    ))
}

/// "today" / "tomorrow" / "2026-09-24" / "3d ago" 程度の簡易表記。
pub fn due_label(due: &DateTime<Utc>) -> String {
    let today = Utc::now().date_naive();
    let d = due.date_naive();
    match (d - today).num_days() {
        0 => "today".into(),
        1 => "tomorrow".into(),
        n if n < 0 => format!("{}d overdue", -n),
        _ => d.format("%Y-%m-%d").to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_color_parses() {
        let c = parse_hex_color("#ff0000").unwrap();
        assert!((c.h - 1.0).abs() < 0.01);
        assert!(parse_hex_color("nope").is_none());
    }
}
