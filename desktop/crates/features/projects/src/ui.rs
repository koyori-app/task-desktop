//! 画面間で共有する小さな表示部品と値の変換。

use gpui_kit::component::WindowExt;
use gpui_kit::component::calendar::Date;
use gpui_kit::component::date_picker::DatePickerState;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use i18n::t;

/// ラベル・ステータスの色の候補（Web の既定パレットに近い色）。
pub const PRESET_COLORS: [&str; 8] = [
    "#8b949e", "#1f6feb", "#8b5cf6", "#d29922", "#e5484d", "#238636", "#f97316", "#db2777",
];

pub use ::ui::Colors;

/// 項目をまとめるカード。行の間に区切り線を入れる（設定画面と同じ見た目）。
pub fn card(c: &Colors, rows: Vec<AnyElement>) -> AnyElement {
    ::ui::card(c, rows).into_any_element()
}

pub fn heading(c: &Colors, text: impl Into<SharedString>) -> AnyElement {
    ::ui::section_heading(c, text).pt_2().into_any_element()
}

/// 左に名前と説明、右に操作を置く 1 行。
pub fn row(
    c: &Colors,
    leading: Option<AnyElement>,
    title: impl Into<SharedString>,
    description: Option<String>,
    controls: impl IntoElement,
) -> AnyElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap_3()
        .px_4()
        .py_2p5()
        .children(leading)
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap_0p5()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .text_ellipsis()
                        .child(title.into()),
                )
                .when_some(description.filter(|d| !d.is_empty()), |d, text| {
                    d.child(div().text_xs().text_color(c.text_muted).child(text))
                }),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .flex_shrink_0()
                .items_center()
                .gap_1()
                .child(controls),
        )
        .into_any_element()
}

/// 読み込み中・空のときの 1 行。
pub fn placeholder(c: &Colors, text: impl Into<SharedString>) -> AnyElement {
    div()
        .p_4()
        .text_sm()
        .text_color(c.text_muted)
        .child(text.into())
        .into_any_element()
}

/// ラベル付きの入力欄。
pub fn field(c: &Colors, label: &'static str, input: impl IntoElement) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .min_w_0()
        .child(div().text_xs().text_color(c.text_muted).child(label))
        .child(input)
}

/// 名前の頭文字の丸（画像は読み込まない）。
pub fn initial(c: &Colors, name: &str) -> Div {
    div()
        .size(px(24.))
        .flex_shrink_0()
        .rounded_full()
        .bg(c.hover)
        .flex()
        .items_center()
        .justify_center()
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .child(
            name.chars()
                .next()
                .map(|ch| ch.to_uppercase().to_string())
                .unwrap_or_default(),
        )
}

pub fn swatch(color: Hsla) -> Div {
    div().size(px(12.)).flex_shrink_0().rounded_full().bg(color)
}

/// 小さな丸角の印（Default / Done など）。
pub fn badge(c: &Colors, text: impl Into<SharedString>, color: Option<Hsla>) -> Div {
    ::ui::badge(c, text, color)
}

/// `#rrggbb` を色へ。それ以外は None。
pub fn parse_hex(s: &str) -> Option<Hsla> {
    let h = s.trim().strip_prefix('#')?;
    if h.len() != 6 {
        return None;
    }
    Some(rgb(u32::from_str_radix(h, 16).ok()?).into())
}

pub fn picked(state: &Entity<DatePickerState>, cx: &App) -> Option<chrono::NaiveDate> {
    match state.read(cx).date() {
        Date::Single(date) => date,
        _ => None,
    }
}

/// 取り消せない操作の確認ダイアログ。OK で `ok` を呼ぶ。
pub fn confirm(
    window: &mut Window,
    cx: &mut App,
    title: String,
    description: String,
    ok: impl Fn(&mut Window, &mut App) + 'static,
) {
    let ok = std::rc::Rc::new(ok);
    window.open_alert_dialog(cx, move |alert, _, _| {
        let ok = ok.clone();
        alert
            .confirm()
            .title(title.clone())
            .description(description.clone())
            .button_props(
                gpui_kit::component::dialog::DialogButtonProps::default()
                    .ok_text(t!("projects.common.confirm"))
                    .ok_variant(gpui_kit::component::button::ButtonVariant::Danger)
                    .cancel_text(t!("projects.common.cancel"))
                    .show_cancel(true),
            )
            .on_ok(move |_, window, cx| {
                ok(window, cx);
                true
            })
    });
}

/// 相対時刻（Web の relativeTime と同じ区切り。1 日以上は日付）。
pub fn relative_time(
    at: &chrono::DateTime<chrono::Utc>,
    now: chrono::DateTime<chrono::Utc>,
) -> String {
    let minutes = (now - *at).num_minutes();
    if minutes < 1 {
        t!("projects.time.just_now").into()
    } else if minutes < 60 {
        t!("projects.time.minutes", n = minutes)
    } else if minutes < 60 * 24 {
        t!("projects.time.hours", n = minutes / 60)
    } else {
        at.with_timezone(&core::timezone::current())
            .format("%Y-%m-%d")
            .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::parse_hex;

    #[test]
    fn hex_requires_hash_and_six_digits() {
        assert!(parse_hex("#1f6feb").is_some());
        assert!(parse_hex("1f6feb").is_none());
        assert!(parse_hex("#fff").is_none());
        assert!(parse_hex("#zzzzzz").is_none());
    }
}
