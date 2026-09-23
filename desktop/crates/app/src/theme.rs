//! §21 Theme。Koyori のセマンティックトークンを一箇所に集約する。
//! Feature は色を直書きせず [`colors`] から取る。

use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::{App, Hsla, Window};

/// desktop.md §21 のトークン名に合わせた面。
/// 実体は gpui-component の Theme / SemanticThemeTokens。
#[allow(dead_code)] // feature が全フィールドを使うまで警告を止める
#[derive(Debug, Clone, Copy)]
pub struct KoyoriColors {
    pub background: Hsla,
    pub surface: Hsla,
    pub text: Hsla,
    pub text_muted: Hsla,
    pub border: Hsla,
    pub accent: Hsla,
    pub success: Hsla,
    pub warning: Hsla,
    pub danger: Hsla,
}

/// 現在のテーマからトークンを取る。
pub fn colors(cx: &App) -> KoyoriColors {
    let theme = Theme::global(cx);
    let c = theme.semantic_tokens().colors;
    KoyoriColors {
        background: c.background,
        surface: c.surface,
        text: c.foreground,
        text_muted: c.muted_foreground,
        border: c.border,
        accent: c.accent,
        success: theme.success,
        warning: theme.warning,
        danger: theme.danger,
    }
}

/// Settings の Appearance を gpui-kit のテーマへ適用する。
/// `System` は現在の OS 外観に解決する（変化の追従は window 側で
/// `observe_window_appearance` して再度呼ぶ）。
pub fn apply(appearance: core::settings::Appearance, window: Option<&mut Window>, cx: &mut App) {
    let mode = match appearance {
        core::settings::Appearance::Light => ThemeMode::Light,
        core::settings::Appearance::Dark => ThemeMode::Dark,
        core::settings::Appearance::System => ThemeMode::from(cx.window_appearance()),
    };
    Theme::change(mode, window, cx);
}
