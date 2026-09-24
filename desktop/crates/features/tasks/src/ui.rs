//! 一覧と Detail で共有する小さな表示部品。

use gpui_kit::*;

/// ステータス色の丸 + 名前。色付き文字だけより状態が目で拾いやすい。
pub fn status_pill(name: &str, color: Hsla) -> Div {
    div()
        .flex()
        .flex_shrink_0()
        .items_center()
        .gap_1()
        .child(status_dot(color))
        .child(name.to_string())
}

pub fn status_dot(color: Hsla) -> Div {
    div().size(px(8.)).flex_shrink_0().rounded_full().bg(color)
}
