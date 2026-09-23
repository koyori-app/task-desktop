//! メインウィンドウの骨組み（desktop.md §14）。
//! Header / Sidebar / Content / Detail。中身は feature crate が後から埋める。

use gpui_kit::component::badge::Badge;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::resizable::{ResizableState, h_resizable, resizable_panel};
use gpui_kit::component::sidebar::{Sidebar, SidebarGroup, SidebarMenuItem};
use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::theme::{self, KoyoriColors};

/// 画面内の遷移先。通知の `target` → 内部ルート変換（§11）は
/// feature 側に実装される。
#[allow(dead_code)] // TaskDetail / Reviews は feature 実装で生成される
#[derive(Debug, Clone, PartialEq)]
pub enum Route {
    MyTasks,
    Today,
    Upcoming,
    Notifications,
    Project { id: uuid::Uuid, label: String },
    TaskDetail { project: uuid::Uuid, task: uuid::Uuid },
    Reviews { project: uuid::Uuid },
    Settings,
}

/// §23 Connection Status（Header に出す）。
#[allow(dead_code)] // Offline は同期エンジン結線で設定される
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionStatus {
    Online,
    Offline,
}

/// メインウィンドウの状態。MVP ではこの 1 Entity が全体を持ち、
/// feature の View は Content / Detail の中身として後から差し込む。
#[allow(dead_code)] // client / engine / tenants 等は feature 結線で読まれる
pub struct AppShell {
    pub settings_store: core::SettingsStore,
    pub settings: core::Settings,
    /// ログイン済みなら Device Token 付きクライアント。
    pub client: Option<api::Client>,
    /// 通知同期エンジン（TASKDESKTO-7 でポーリングを回す）。
    pub engine: Option<core::NotificationEngine>,
    pub route: Route,
    pub unread_count: i64,
    pub connection: ConnectionStatus,
    pub tenants: Vec<api::types::TenantListItemResponse>,
    pub projects: Vec<api::types::ProjectResponse>,
    /// Content / Detail の分割位置（§14: レイアウト状態はローカル保存）。
    pub resizable: Entity<ResizableState>,
}

impl AppShell {
    pub fn new(
        settings: core::Settings,
        settings_store: core::SettingsStore,
        client: Option<api::Client>,
        engine: Option<core::NotificationEngine>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            settings,
            settings_store,
            client,
            engine,
            route: Route::MyTasks,
            unread_count: 0,
            connection: ConnectionStatus::Online,
            tenants: vec![],
            projects: vec![],
            resizable: cx.new(|_| ResizableState::default()),
        }
    }

    fn navigate(&mut self, route: Route, cx: &mut Context<Self>) {
        self.route = route;
        cx.notify();
    }

    fn nav_item(
        &self,
        route: Route,
        label: impl Into<SharedString>,
        icon: IconName,
        cx: &mut Context<Self>,
    ) -> SidebarMenuItem {
        let active = self.route == route;
        SidebarMenuItem::new(label)
            .icon(icon)
            .active(active)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.navigate(route.clone(), cx)
            }))
    }

    fn sidebar(&self, colors: &KoyoriColors, cx: &mut Context<Self>) -> impl IntoElement {
        let main_group = SidebarGroup::new("").children([
            self.nav_item(Route::MyTasks, "My Tasks", IconName::ListTodo, cx),
            self.nav_item(Route::Today, "Today", IconName::Calendar, cx),
            self.nav_item(Route::Upcoming, "Upcoming", IconName::CalendarClock, cx),
        ]);

        let project_items: Vec<SidebarMenuItem> = self
            .projects
            .iter()
            .map(|p| {
                self.nav_item(
                    Route::Project {
                        id: p.id,
                        label: p.key.clone(),
                    },
                    p.key.clone(),
                    IconName::Folder,
                    cx,
                )
            })
            .collect();

        let unread = self.unread_count;
        let notifications = SidebarMenuItem::new("Notifications")
            .icon(IconName::Bell)
            .active(self.route == Route::Notifications)
            .suffix(move |_, _| {
                div().when(unread > 0, |d| {
                    d.child(Badge::new().count(unread.max(0) as usize))
                })
            })
            .on_click(cx.listener(|this, _, _, cx| {
                this.navigate(Route::Notifications, cx)
            }));

        div()
            .w(px(230.))
            .flex_shrink_0()
            .h_full()
            .border_r_1()
            .border_color(colors.border)
            .child(
                Sidebar::new("sidebar")
                    .child(main_group)
                    .child(SidebarGroup::new("Projects").children(project_items))
                    .child(SidebarGroup::new("").child(notifications)),
            )
    }

    fn header(&self, colors: &KoyoriColors, cx: &mut Context<Self>) -> impl IntoElement {
        let tenant_name = self
            .settings
            .last_tenant_id
            .map(|id| id.to_string())
            .unwrap_or_else(|| "no tenant".into());

        let status_color = match self.connection {
            ConnectionStatus::Online => colors.success,
            ConnectionStatus::Offline => colors.danger,
        };

        div()
            .h(px(48.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .px_4()
            .gap_3()
            .border_b_1()
            .border_color(colors.border)
            .child(div().text_lg().font_weight(FontWeight::BOLD).child("Koyori"))
            .child(
                div()
                    .text_sm()
                    .text_color(colors.text_muted)
                    .child(tenant_name),
            )
            .child(div().flex_1())
            // §23 Connection Status
            .child(
                div()
                    .size(px(8.))
                    .rounded_full()
                    .bg(status_color)
                    .id("connection-status"),
            )
            .child(
                Button::new("search")
                    .ghost()
                    .icon(IconName::Search)
                    .on_click(cx.listener(|this, _, _, cx| {
                        // Quick Search は TASKDESKTO-10。検索 UI が来るまで
                        // My Tasks へ戻すだけの仮導線。
                        this.navigate(Route::MyTasks, cx)
                    })),
            )
            .child(
                Button::new("notifications")
                    .ghost()
                    .icon(IconName::Bell)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.navigate(Route::Notifications, cx)
                    })),
            )
            .child(
                Button::new("settings")
                    .ghost()
                    .icon(IconName::Settings)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.navigate(Route::Settings, cx)
                    })),
            )
    }

    fn content(&self, _colors: &KoyoriColors) -> impl IntoElement {
        // feature crate の View が入る場所。骨組みではルート名だけ出す。
        let title: SharedString = match &self.route {
            Route::MyTasks => "My Tasks".into(),
            Route::Today => "Today".into(),
            Route::Upcoming => "Upcoming".into(),
            Route::Notifications => "Notifications".into(),
            Route::Project { label, .. } => label.clone().into(),
            Route::TaskDetail { .. } => "Task".into(),
            Route::Reviews { .. } => "Reviews".into(),
            Route::Settings => "Settings".into(),
        };
        div()
            .flex_1()
            .h_full()
            .p_4()
            .child(div().text_xl().font_weight(FontWeight::SEMIBOLD).child(title))
    }

    fn detail(&self, colors: &KoyoriColors) -> impl IntoElement {
        div()
            .h_full()
            .p_4()
            .border_l_1()
            .border_color(colors.border)
            .child(
                div()
                    .text_sm()
                    .text_color(colors.text_muted)
                    .child("Select an item"),
            )
    }

    /// §14: 分割位置はローカル設定に保存して次回復元する。
    fn detail_width(&self) -> Pixels {
        self.settings
            .window_layout
            .as_ref()
            .and_then(|v| v.get("detail_width")?.as_f64())
            .map(|w| px(w as f32))
            .unwrap_or(px(360.))
    }
}

impl Render for AppShell {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = theme::colors(cx);
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(colors.background)
            .text_color(colors.text)
            .child(self.header(&colors, cx))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h_0()
                    .child(self.sidebar(&colors, cx))
                    .child(
                        div().flex_1().min_w_0().child(
                            h_resizable("content-detail")
                                .with_state(&self.resizable)
                                .on_resize({
                                    let shell = cx.entity();
                                    move |state, _, cx| {
                                        if let Some(w) =
                                            state.read(cx).sizes().last().copied()
                                        {
                                            shell.update(cx, |this, _| {
                                                this.settings.window_layout = Some(
                                                    serde_json::json!({
                                                        "detail_width": f64::from(w)
                                                    }),
                                                );
                                                let _ = this
                                                    .settings_store
                                                    .save(&this.settings);
                                            });
                                        }
                                    }
                                })
                                .child(resizable_panel().child(self.content(&colors)))
                                .child(
                                    resizable_panel()
                                        .size(self.detail_width())
                                        .size_range(px(280.)..px(560.))
                                        .child(self.detail(&colors)),
                                ),
                        ),
                    ),
            )
    }
}
