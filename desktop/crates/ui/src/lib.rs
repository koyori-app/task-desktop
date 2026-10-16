//! Koyori 固有の見た目の決まり。§21 のトークンと、各画面で繰り返す最小の部品だけを置く。
//! Feature は色を直書きせず [`colors`] から取り、下の部品で画面を組む。
//!
//! 決まり:
//! - 文字: 画面見出し `text_base` SEMIBOLD / セクション見出し `text_xs` MEDIUM 薄色 /
//!   本文 `text_sm` / 補助・メタ `text_xs` 薄色
//! - 余白: 画面の外周 `px_4`、セクション間 `gap_6`、セクション内 `gap_2`、行は `px_3 py_2`
//! - ボタン: 画面内は `.small()`。主操作だけ `primary`、他は `ghost` か `outline`
//! - 行ごとの操作はホバーした行だけに出す（[`reveal_on_hover`]）

use gpui_kit::assets::IconName;
use gpui_kit::component::{Icon, Sizable, Theme};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

/// desktop.md §21 のトークン。実体は gpui-kit の Theme（`app` の theme::apply で調整済み）。
#[derive(Debug, Clone, Copy)]
pub struct Colors {
    pub background: Hsla,
    /// カード・ポップオーバーの面。
    pub surface: Hsla,
    pub sidebar: Hsla,
    pub text: Hsla,
    pub text_muted: Hsla,
    pub border: Hsla,
    pub accent: Hsla,
    /// accent の上に載せる文字。
    pub accent_text: Hsla,
    pub success: Hsla,
    pub warning: Hsla,
    pub danger: Hsla,
    /// 行のホバー（前景色を薄く敷く）。
    pub hover: Hsla,
    /// 行の選択中。
    pub selected: Hsla,
}

pub fn colors(cx: &App) -> Colors {
    let t = Theme::global(cx);
    Colors {
        background: t.background,
        surface: t.popover,
        sidebar: t.sidebar,
        text: t.foreground,
        text_muted: t.muted_foreground,
        border: t.border,
        accent: t.primary,
        accent_text: t.primary_foreground,
        success: t.success,
        warning: t.warning,
        danger: t.danger,
        hover: t.list_hover,
        selected: t.list_active,
    }
}

/// プロパティ行のラベル列の幅。
const LABEL_WIDTH: Pixels = px(124.);

/// 画面上部の見出し。右に操作を足すときは `.child(...)` を続ける。
pub fn page_header(
    c: &Colors,
    title: impl Into<SharedString>,
    subtitle: Option<SharedString>,
) -> Div {
    div()
        .flex()
        .flex_row()
        .flex_shrink_0()
        .items_center()
        .gap_2()
        .min_w_0()
        .px_4()
        .pt_4()
        .pb_3()
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap_0p5()
                .child(
                    div()
                        .text_base()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_ellipsis()
                        .child(title.into()),
                )
                .when_some(subtitle, |d, s| {
                    d.child(
                        div()
                            .text_xs()
                            .text_color(c.text_muted)
                            .text_ellipsis()
                            .child(s),
                    )
                }),
        )
}

/// セクション見出し（小さく薄い文字）。右に操作を足すときは `.child(...)` を続ける。
pub fn section_heading(c: &Colors, title: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap_2()
        .min_h(px(24.))
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(c.text_muted)
                .child(title.into()),
        )
        .child(div().flex_1())
}

/// 行をまとめるカード。行の間に区切り線を入れる。
pub fn card(c: &Colors, rows: impl IntoIterator<Item = AnyElement>) -> Div {
    div()
        .flex()
        .flex_col()
        .flex_shrink_0()
        .w_full()
        .rounded_lg()
        .border_1()
        .border_color(c.border)
        .bg(c.surface)
        .overflow_hidden()
        .children(rows.into_iter().enumerate().map(|(ix, row)| {
            div()
                .when(ix > 0, |d| d.border_t_1().border_color(c.border))
                .child(row)
        }))
}

/// 「アイコン付きラベル → 値」の 1 行。値は左寄せで中身の幅に収める。
pub fn property(
    c: &Colors,
    icon: IconName,
    label: impl Into<SharedString>,
    value: impl IntoElement,
) -> Div {
    let muted = c.text_muted;
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap_2()
        .min_h(px(30.))
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .w(LABEL_WIDTH)
                .flex_shrink_0()
                .text_sm()
                .text_color(muted)
                .child(Icon::new(icon).small())
                .child(div().min_w_0().truncate().child(label.into())),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap_1()
                .flex_1()
                .min_w_0()
                .text_sm()
                .child(value),
        )
}

/// 左ナビの 1 行（Sidebar・設定の区分）。薄いアイコン + 名前、選択中だけ面を敷く。
pub fn nav_item(
    c: &Colors,
    id: impl Into<ElementId>,
    icon: impl IntoElement,
    label: impl Into<SharedString>,
    active: bool,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_row()
        .items_center()
        .gap_2()
        .h(px(28.))
        .px_2()
        .rounded_md()
        .text_sm()
        .cursor_pointer()
        .when(active, |d| d.bg(c.selected))
        .when(!active, |d| d.hover(|d| d.bg(c.hover)))
        .child(
            div()
                .w(px(16.))
                .flex()
                .flex_shrink_0()
                .justify_center()
                .text_color(c.text_muted)
                .child(icon),
        )
        .child(div().flex_1().min_w_0().truncate().child(label.into()))
}

/// 一覧の上に置く 1 行の操作帯（検索・絞り込み・作成など）。
pub fn toolbar(c: &Colors) -> Div {
    div()
        .flex()
        .flex_row()
        .flex_shrink_0()
        .items_center()
        .gap_2()
        .min_w_0()
        .min_h(px(44.))
        .px_4()
        .py_1p5()
        .border_b_1()
        .border_color(c.border)
}

/// 何もないときの表示。操作を出すときは `.child(...)` を続ける。
pub fn empty_state(
    c: &Colors,
    icon: IconName,
    title: impl Into<SharedString>,
    description: Option<SharedString>,
) -> Div {
    div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_2()
        .py_10()
        .px_4()
        .w_full()
        .child(
            div()
                .size(px(36.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_lg()
                .bg(c.hover)
                .text_color(c.text_muted)
                .child(Icon::new(icon)),
        )
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .child(title.into()),
        )
        .when_some(description, |d, text| {
            d.child(
                div()
                    .max_w(px(360.))
                    .text_xs()
                    .text_color(c.text_muted)
                    .text_center()
                    .child(text),
            )
        })
}

/// 読み込み失敗の表示。再試行ボタンは `.child(...)` で足す。
pub fn error_state(c: &Colors, message: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap_2()
        .px_3()
        .py_2()
        .rounded_md()
        .border_1()
        .border_color(c.danger.opacity(0.4))
        .bg(c.danger.opacity(0.08))
        .text_sm()
        .child(
            div()
                .text_color(c.danger)
                .child(Icon::new(IconName::CircleAlert).small()),
        )
        .child(div().flex_1().min_w_0().child(message.into()))
}

/// 小さな丸角の印（状態名・キーなど）。色を省くと薄色。
pub fn badge(c: &Colors, text: impl Into<SharedString>, color: Option<Hsla>) -> Div {
    let color = color.unwrap_or(c.text_muted);
    div()
        .flex()
        .flex_shrink_0()
        .items_center()
        .gap_1()
        .h(px(18.))
        .px_1p5()
        .rounded_md()
        .bg(color.opacity(0.12))
        .text_color(color)
        .text_size(px(11.))
        .font_weight(FontWeight::MEDIUM)
        .child(text.into())
}

/// ホバー中の行（`.group(group)` を付けた要素）だけで見える。
pub fn reveal_on_hover(group: impl Into<SharedString>, child: impl IntoElement) -> Div {
    div()
        .flex()
        .items_center()
        .gap_1()
        .invisible()
        .group_hover(group, |s| s.visible())
        .child(child)
}
