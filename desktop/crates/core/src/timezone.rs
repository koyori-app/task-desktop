//! 表示タイムゾーン。言語（[`i18n::set_language`]）と同じくプロセス全体で 1 つ。
//! 未設定なら OS のタイムゾーン、それも取れなければ UTC。

use std::sync::RwLock;

use chrono::{DateTime, NaiveDate, Utc};
pub use chrono_tz::{TZ_VARIANTS, Tz};

static SELECTED: RwLock<Option<Tz>> = RwLock::new(None);

/// 設定値（IANA 名）を反映する。`None` や不明な名前は OS のタイムゾーンに戻す。
pub fn set(name: Option<&str>) {
    *SELECTED.write().unwrap() = name.and_then(|n| n.parse().ok());
}

/// OS のタイムゾーン。
pub fn system() -> Tz {
    iana_time_zone::get_timezone()
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(Tz::UTC)
}

/// 表示に使うタイムゾーン。
pub fn current() -> Tz {
    SELECTED.read().unwrap().unwrap_or_else(system)
}

pub fn local(at: &DateTime<Utc>) -> DateTime<Tz> {
    at.with_timezone(&current())
}

pub fn today() -> NaiveDate {
    Utc::now().with_timezone(&current()).date_naive()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_zone_shifts_the_calendar_day() {
        let at = DateTime::parse_from_rfc3339("2026-09-24T20:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        set(Some("Asia/Tokyo"));
        assert_eq!(
            local(&at).format("%Y-%m-%d %H:%M").to_string(),
            "2026-09-25 05:00"
        );
        set(Some("America/Los_Angeles"));
        assert_eq!(
            local(&at).format("%Y-%m-%d %H:%M").to_string(),
            "2026-09-24 13:00"
        );
        set(Some("Not/AZone"));
        assert_eq!(current(), system());
        set(None);
    }
}
