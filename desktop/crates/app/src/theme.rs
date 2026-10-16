//! §21 Theme。Koyori のセマンティックトークンを一箇所に集約する。
//! Feature は色を直書きせず [`colors`] から取る。

use gpui_kit::component::{Theme, ThemeMode, ThemeTokens};
use gpui_kit::{App, Hsla, Window, rgb};

pub use ui::{Colors as KoyoriColors, colors};

/// Settings の Appearance を gpui-kit のテーマへ適用する。
/// `System` は現在の OS 外観に解決する（変化の追従は window 側で
/// `observe_window_appearance` して再度呼ぶ）。
pub fn apply(appearance: core::settings::Appearance, window: Option<&mut Window>, cx: &mut App) {
    let mode = match appearance {
        core::settings::Appearance::Light => ThemeMode::Light,
        core::settings::Appearance::Dark => ThemeMode::Dark,
        core::settings::Appearance::System => ThemeMode::from(cx.window_appearance()),
    };
    Theme::change(mode, None, cx);
    // リンクは既定だと primary（黒）で本文と見分けにくいので、Web と同じ青にする。
    let link = if mode.is_dark() {
        rgb(0x4493f8)
    } else {
        rgb(0x0969da)
    };
    let theme = Theme::global_mut(cx);
    theme.link = link.into();
    theme.link_hover = link.into();
    theme.link_active = link.into();
    refine(theme, mode.is_dark());
    Theme::sync_base(cx);
    if let Some(window) = window {
        window.refresh();
    }
}

/// 既定（shadcn neutral）を落ち着いた配色へ寄せる。境界線は背景に近づけ、
/// サイドバーは本文より一段だけ違う面にし、ホバー・選択は前景色を薄く敷く。
fn refine(theme: &mut Theme, dark: bool) {
    let hex = |v: u32| -> Hsla { rgb(v).into() };
    let (background, sidebar, surface, border, secondary, muted) = if dark {
        (
            hex(0x151515),
            hex(0x1a1a1a),
            hex(0x1e1e1e),
            hex(0x2a2a2a),
            hex(0x262626),
            hex(0x9a9a9a),
        )
    } else {
        (
            hex(0xfbfbfb),
            hex(0xf3f3f3),
            hex(0xffffff),
            hex(0xe6e6e6),
            hex(0xededed),
            hex(0x6b6b6b),
        )
    };
    let fg = theme.foreground;
    let (hover, selected) = if dark { (0.06, 0.1) } else { (0.04, 0.07) };
    theme.background = background;
    theme.sidebar = sidebar;
    theme.sidebar_border = border;
    theme.sidebar_accent = fg.opacity(selected);
    theme.popover = surface;
    theme.border = border;
    theme.input = border;
    theme.secondary = secondary;
    theme.secondary_hover = fg.opacity(hover + 0.04);
    theme.muted = secondary;
    theme.muted_foreground = muted;
    theme.colors.list = background;
    theme.list_hover = fg.opacity(hover);
    theme.list_active = fg.opacity(selected);
    theme.list_active_border = border;
    theme.table_hover = fg.opacity(hover);
    theme.table_row_border = border;
    theme.tab_bar = background;
    theme.title_bar = background;
    theme.title_bar_border = border;
    theme.group_box = surface;
    theme.accordion = background;
    theme.tokens = ThemeTokens::from(&theme.colors);
}
