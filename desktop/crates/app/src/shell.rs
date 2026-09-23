//! メインウィンドウの骨組み（desktop.md §14）。
//! Header / Sidebar / Content / Detail。中身は feature crate が後から埋める。

use gpui_kit::assets::IconName;
use gpui_kit::component::badge::Badge;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::command::{Command, CommandItem, CommandState};
use gpui_kit::component::resizable::{ResizableState, h_resizable, resizable_panel};
use gpui_kit::component::sidebar::{Sidebar, SidebarGroup, SidebarMenuItem};
use gpui_kit::component::{Icon, IndexPath};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use feature_notifications::{CenterEvent, NavTarget, NotificationCenter};
use feature_reviews::{ReviewDetailView, ReviewListEvent, ReviewListView};
use feature_settings::{SettingsEvent, SettingsView};
use feature_tasks::{ListMode, TaskDetailView, TaskListEvent, TaskListView};

use crate::theme::{self, KoyoriColors};

/// §7: 通知ポーリング間隔。
const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);
/// §9: Tray メニューイベントのポーリング間隔。
const TRAY_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(300);

// §20 Command Palette (Ctrl+K) / Quick Search (Ctrl+P)。
actions!(shell, [OpenPalette, OpenQuickSearch]);

/// パレットの 2 モード。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PaletteKind {
    Commands,
    QuickSearch,
}

/// パレット項目が確定した時に実行する内部アクション。
#[derive(Debug, Clone)]
enum PaletteAct {
    Navigate(Route),
    OpenTask {
        project: uuid::Uuid,
        task: uuid::Uuid,
    },
    SwitchTenant(uuid::Uuid),
    MarkAllRead,
    RefreshTasks,
}

#[derive(Debug, Clone)]
struct PaletteEntry {
    label: SharedString,
    icon: IconName,
    keywords: Vec<SharedString>,
    act: PaletteAct,
}

/// 画面内の遷移先。通知の `target` → 内部ルート変換（§11）は
/// feature 側に実装される。
#[allow(dead_code)] // TaskDetail / Reviews は feature 実装で生成される
#[derive(Debug, Clone, PartialEq)]
pub enum Route {
    MyTasks,
    Today,
    Upcoming,
    Notifications,
    Project {
        id: uuid::Uuid,
        label: String,
    },
    TaskDetail {
        project: uuid::Uuid,
        task: uuid::Uuid,
    },
    Reviews {
        project: uuid::Uuid,
    },
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
#[allow(dead_code)] // client / tenants 等は feature 結線で読まれる
pub struct AppShell {
    pub settings_store: core::SettingsStore,
    pub settings: core::Settings,
    /// ログイン済みなら Device Token 付きクライアント。
    pub client: Option<api::Client>,
    /// §12 Notification Center。
    pub center: Entity<NotificationCenter>,
    /// §15 Tasks 一覧（Content 側）。
    pub task_list: Entity<TaskListView>,
    /// §15 Task 詳細（Detail 側）。
    pub task_detail: Entity<TaskDetailView>,
    /// §16 PR 一覧（Content 側）。
    pub review_list: Entity<ReviewListView>,
    /// §17-18 Review 詳細（Detail 側）。
    pub review_detail: Entity<ReviewDetailView>,
    /// §22 Settings。
    pub settings_view: Entity<SettingsView>,
    pub route: Route,
    pub unread_count: i64,
    pub connection: ConnectionStatus,
    pub tenants: Vec<api::types::TenantListItemResponse>,
    pub projects: Vec<api::types::ProjectResponse>,
    /// Content / Detail の分割位置（§14: レイアウト状態はローカル保存）。
    pub resizable: Entity<ResizableState>,
    /// §9 System Tray。非対応環境では None（閉じたら終了、§8）。
    tray: Option<::platform::AppTray>,
    /// Window close → Hide を動的に切り替える共有フラグ（§8）。
    keep_running: std::rc::Rc<std::cell::Cell<bool>>,
    /// Tray からの再表示に使う Window ハンドル。
    window_handle: AnyWindowHandle,
    /// §6: ブラウザ承認待ちの間 true（Login 画面の状態表示用）。
    auth_waiting: bool,
    auth_error: Option<SharedString>,
    /// §20 パレット。Some(kind) の間だけオーバーレイ表示。
    palette: Option<PaletteKind>,
    palette_state: Entity<CommandState>,
    palette_entries: Vec<PaletteEntry>,
    _subs: Vec<Subscription>,
}

impl AppShell {
    #[allow(clippy::too_many_arguments)] // 起動時注入が多い。実引数は main 側だけ
    pub fn new(
        settings: core::Settings,
        settings_store: core::SettingsStore,
        client: Option<api::Client>,
        engine: Option<core::NotificationEngine>,
        tray: Option<::platform::AppTray>,
        keep_running: std::rc::Rc<std::cell::Cell<bool>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let center = cx.new(|_| NotificationCenter::new(client.clone()));
        let sub = cx.subscribe(&center, |this, _center, ev: &CenterEvent, cx| {
            let CenterEvent::Navigate(target) = ev;
            this.navigate_target(target.clone(), cx);
        });
        let tenant = settings.last_tenant_id;
        let task_list = cx.new(|cx| TaskListView::new(client.clone(), tenant, window, cx));
        let task_detail = cx.new(|cx| TaskDetailView::new(client.clone(), tenant, window, cx));
        let sub2 = cx.subscribe(&task_list, |this, _list, ev: &TaskListEvent, cx| {
            let TaskListEvent::Select { project, task } = ev;
            this.task_detail
                .update(cx, |d, cx| d.open(*project, *task, cx));
        });
        let review_list = cx.new(|cx| ReviewListView::new(client.clone(), tenant, window, cx));
        let review_detail = cx.new(|_| ReviewDetailView::new(client.clone(), tenant));
        let sub3 = cx.subscribe(&review_list, |this, _list, ev: &ReviewListEvent, cx| {
            let ReviewListEvent::Select { pr, title } = ev;
            this.review_detail
                .update(cx, |d, cx| d.open(*pr, title.clone(), cx));
        });

        let settings_view = cx.new(|cx| {
            SettingsView::new(
                settings.clone(),
                settings_store.clone(),
                client.clone(),
                window,
                cx,
            )
        });
        let sub4 = cx.subscribe(
            &settings_view,
            |this, _view, ev: &SettingsEvent, cx| match ev {
                SettingsEvent::Changed(settings) => {
                    this.settings = settings.clone();
                    // §8: Tray ありの時だけ常駐を有効化。
                    this.keep_running
                        .set(settings.keep_running_in_background && this.tray.is_some());
                    theme::apply(settings.appearance, None, cx);
                    cx.notify();
                }
                SettingsEvent::LoggedOut => {
                    this.client = None;
                    this.task_list.update(cx, |l, cx| l.clear_client(cx));
                    this.task_detail.update(cx, |d, _| d.clear_client());
                    this.review_list.update(cx, |l, _| l.clear_client());
                    this.review_detail.update(cx, |d, _| d.clear_client());
                    this.center.update(cx, |c, cx| c.clear_client(cx));
                    this.navigate(Route::MyTasks, cx);
                }
            },
        );

        let mut this = Self {
            settings,
            settings_store,
            client: client.clone(),
            settings_view,
            center,
            task_list,
            task_detail,
            review_list,
            review_detail,
            route: Route::MyTasks,
            unread_count: 0,
            connection: ConnectionStatus::Online,
            tenants: vec![],
            projects: vec![],
            resizable: cx.new(|_| ResizableState::default()),
            tray,
            keep_running,
            window_handle: window.window_handle(),
            auth_waiting: false,
            auth_error: None,
            palette: None,
            palette_state: cx.new(|cx| CommandState::new(window, cx)),
            palette_entries: vec![],
            _subs: vec![sub, sub2, sub3, sub4],
        };
        this.start_polling(engine, cx);
        this.start_tray_polling(cx);
        if let Some(client) = client {
            this.bootstrap(client, cx);
        }
        this
    }

    /// 起動時: tenant 解決（未設定なら先頭を保存）→ projects ロード →
    /// task views にクライアントを渡して初期ロード。
    fn bootstrap(&mut self, client: api::Client, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let tenants = client.list_tenants().await.unwrap_or_default();
            let tenant = this
                .update(cx, |s, cx| {
                    s.tenants = tenants;
                    if s.settings.last_tenant_id.is_none() {
                        s.settings.last_tenant_id = s.tenants.first().map(|t| t.id);
                        let _ = s.settings_store.save(&s.settings);
                    }
                    let tenant = s.settings.last_tenant_id;
                    s.task_list
                        .update(cx, |l, cx| l.set_client(client.clone(), tenant, cx));
                    s.task_detail
                        .update(cx, |d, _| d.set_client(client.clone(), tenant));
                    s.review_list
                        .update(cx, |l, _| l.set_client(client.clone(), tenant));
                    s.review_detail
                        .update(cx, |d, _| d.set_client(client.clone(), tenant));
                    cx.notify();
                    tenant
                })
                .ok()
                .flatten();
            let Some(tenant) = tenant else {
                return;
            };
            if let Ok(projects) = client.list_projects(tenant).await {
                let _ = this.update(cx, |s, cx| {
                    s.projects = projects;
                    cx.notify();
                });
            }
        })
        .detach();
    }

    // ---- §6 Device Token ログイン ----

    /// Sign in ボタン: authorize URL をブラウザで開き、loopback callback を
    /// 別スレッドで待つ（blocking のため executor を塞がない）。
    fn begin_login(&mut self, cx: &mut Context<Self>) {
        let pending = match core::auth::PendingAuth::start(
            &self.settings.web_base,
            &core::auth::default_device_name(),
        ) {
            Ok(p) => p,
            Err(_) => {
                self.auth_error = Some("Failed to start authorization".into());
                cx.notify();
                return;
            }
        };
        if ::platform::open_url(pending.authorize_url()).is_err() {
            self.auth_error = Some("Failed to open browser".into());
            cx.notify();
            return;
        }
        self.auth_waiting = true;
        self.auth_error = None;
        cx.notify();

        let api_base = self.settings.api_base.clone();
        let (tx, rx) = futures::channel::oneshot::channel();
        std::thread::spawn(move || {
            let _ = tx.send(pending.wait_for_grant(std::time::Duration::from_secs(300)));
        });
        cx.spawn(async move |this, cx| {
            let grant = rx.await.ok().and_then(|r| r.ok());
            let client = match grant {
                Some(grant) => match core::auth::redeem(&api_base, &grant).await {
                    Ok(token) => api::Client::new(&api_base, &token).ok(),
                    Err(_) => None,
                },
                None => None,
            };
            let _ = this.update(cx, |s, cx| match client {
                Some(client) => s.complete_login(client, cx),
                None => {
                    s.auth_waiting = false;
                    s.auth_error = Some("Sign-in failed or timed out".into());
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// ログイン完了: client/engine を生成し全 view に結線する。
    fn complete_login(&mut self, client: api::Client, cx: &mut Context<Self>) {
        self.client = Some(client.clone());
        self.auth_waiting = false;
        let tenant = self.settings.last_tenant_id;
        self.center
            .update(cx, |c, cx| c.set_client(client.clone(), cx));
        self.task_list
            .update(cx, |l, cx| l.set_client(client.clone(), tenant, cx));
        self.task_detail
            .update(cx, |d, _| d.set_client(client.clone(), tenant));
        self.review_list
            .update(cx, |l, _| l.set_client(client.clone(), tenant));
        self.review_detail
            .update(cx, |d, _| d.set_client(client.clone(), tenant));
        self.settings_view
            .update(cx, |v, cx| v.set_client(client.clone(), cx));
        let engine = core::NotificationEngine::new(
            client.clone(),
            ::platform::Notifier::new(::platform::WINDOWS_AUMID),
            self.settings.notification_cursor.clone(),
        );
        self.start_polling(Some(engine), cx);
        self.bootstrap(client, cx);
        cx.notify();
    }

    /// §7/§10: 30 秒ごとに catch-up。エンジンはこのタスクが所有する
    /// （&mut を await 跨ぎで持てないため Entity に置かない）。
    fn start_polling(&mut self, engine: Option<core::NotificationEngine>, cx: &mut Context<Self>) {
        let Some(mut engine) = engine else {
            return;
        };
        cx.spawn(async move |this, cx| {
            loop {
                let prefs = this
                    .update(cx, |s, _| s.settings.notifications.clone())
                    .unwrap_or_default();
                match engine.tick(&prefs).await {
                    Ok(outcome) => {
                        let cursor = engine.last_cursor().map(str::to_owned);
                        let _ = this.update(cx, |s, cx| s.apply_tick(outcome, cursor, cx));
                    }
                    Err(_) => {
                        // §23: 接続失敗は非致命的。401 のログアウト処理は認証タスク側。
                        let _ = this.update(cx, |s, cx| {
                            s.connection = ConnectionStatus::Offline;
                            cx.notify();
                        });
                    }
                }
                cx.background_executor().timer(POLL_INTERVAL).await;
            }
        })
        .detach();
    }

    /// §9: Tray メニューイベントをポーリングして処理する。
    /// Open → 前面化 / Notifications → 前面化 + Center へ / Quit → 終了。
    fn start_tray_polling(&mut self, cx: &mut Context<Self>) {
        if self.tray.is_none() {
            return;
        }
        cx.spawn(async move |this, cx| {
            loop {
                let action = this
                    .update(cx, |s, _| s.tray.as_ref().and_then(|t| t.poll_action()))
                    .ok()
                    .flatten();
                match action {
                    Some(::platform::TrayAction::Open) => {
                        let handle = this.update(cx, |s, _| s.window_handle).ok();
                        if let Some(handle) = handle {
                            let _ =
                                cx.update_window(handle, |_, window, _| window.activate_window());
                        }
                    }
                    Some(::platform::TrayAction::ShowNotifications) => {
                        let _ = this.update(cx, |s, cx| {
                            s.navigate(Route::Notifications, cx);
                            cx.update_window(s.window_handle, |_, window, _| {
                                window.activate_window()
                            })
                            .ok();
                        });
                    }
                    Some(::platform::TrayAction::Quit) => {
                        cx.update(|cx| cx.quit());
                    }
                    None => {}
                }
                cx.background_executor().timer(TRAY_POLL_INTERVAL).await;
            }
        })
        .detach();
    }

    /// tick の結果を UI へ反映（unread badge / Center 新着 / 接続状態 / カーソル保存）。
    fn apply_tick(
        &mut self,
        outcome: core::TickOutcome,
        cursor: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.unread_count = outcome.unread_count;
        self.connection = ConnectionStatus::Online;
        // §9: 未読数を Tray ツールチップへ反映。
        if let Some(tray) = &self.tray {
            let tip = match self.unread_count {
                0 => "Koyori".to_string(),
                n => format!("Koyori — {n} unread"),
            };
            let _ = tray.set_tooltip(&tip);
        }
        if cursor.is_some() && cursor != self.settings.notification_cursor {
            self.settings.notification_cursor = cursor;
            let _ = self.settings_store.save(&self.settings);
        }
        self.center
            .update(cx, |c, cx| c.prepend_new(&outcome.new_items, cx));
        cx.notify();
    }

    /// §11: 通知クリック → target を内部ルートへ変換。
    /// project が取れない古い通知は遷移しない（既読化だけは済んでいる）。
    fn navigate_target(&mut self, target: NavTarget, cx: &mut Context<Self>) {
        let (Some(t), Some(project)) = (target.target, target.project.map(|p| p.id)) else {
            return;
        };
        let route = match t {
            api::spec::NotificationTarget::Task { task_id } => Route::TaskDetail {
                project,
                task: task_id,
            },
            api::spec::NotificationTarget::Review { .. }
            | api::spec::NotificationTarget::ReviewFinding { .. } => Route::Reviews { project },
        };
        self.navigate(route, cx);
    }

    fn navigate(&mut self, route: Route, cx: &mut Context<Self>) {
        match &route {
            Route::MyTasks => self
                .task_list
                .update(cx, |l, cx| l.set_mode(ListMode::MyTasks, cx)),
            Route::Today => self
                .task_list
                .update(cx, |l, cx| l.set_mode(ListMode::Today, cx)),
            Route::Upcoming => self
                .task_list
                .update(cx, |l, cx| l.set_mode(ListMode::Upcoming, cx)),
            Route::Project { id, label } => {
                let key = label.clone();
                self.task_list.update(cx, |l, cx| {
                    l.set_mode(ListMode::Project { id: *id, key }, cx)
                })
            }
            Route::TaskDetail { project, task } => {
                let (p, t) = (*project, *task);
                self.task_detail.update(cx, |d, cx| d.open(p, t, cx));
            }
            Route::Reviews { project } => {
                let p = *project;
                self.review_list.update(cx, |l, cx| l.set_project(p, cx));
                self.review_detail.update(cx, |d, _| d.set_project(p));
            }
            _ => {}
        }
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
            .on_click(cx.listener(move |this, _, _, cx| this.navigate(route.clone(), cx)))
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
            .on_click(cx.listener(|this, _, _, cx| this.navigate(Route::Notifications, cx)));

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
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::BOLD)
                    .child("Koyori"),
            )
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
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.toggle_palette(PaletteKind::QuickSearch, window, cx)
                    })),
            )
            .child(
                Button::new("notifications")
                    .ghost()
                    .icon(IconName::Bell)
                    .on_click(
                        cx.listener(|this, _, _, cx| this.navigate(Route::Notifications, cx)),
                    ),
            )
            .child(
                Button::new("settings")
                    .ghost()
                    .icon(IconName::Settings)
                    .on_click(cx.listener(|this, _, _, cx| this.navigate(Route::Settings, cx))),
            )
    }

    // ---- §20 Command Palette / Quick Search ----

    fn toggle_palette(&mut self, kind: PaletteKind, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette == Some(kind) {
            self.close_palette(cx);
            return;
        }
        self.palette_entries = self.build_entries(kind, cx);
        self.palette = Some(kind);
        self.palette_state
            .update(cx, |s, cx| s.set_query("", window, cx));
        let focus = self.palette_state.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        cx.notify();
    }

    fn close_palette(&mut self, cx: &mut Context<Self>) {
        self.palette = None;
        self.palette_entries.clear();
        cx.notify();
    }

    fn build_entries(&self, kind: PaletteKind, cx: &mut Context<Self>) -> Vec<PaletteEntry> {
        let mut v = vec![];
        let nav = |label: &'static str, icon: IconName, route: Route| PaletteEntry {
            label: label.into(),
            icon,
            keywords: vec![],
            act: PaletteAct::Navigate(route),
        };
        match kind {
            PaletteKind::Commands => {
                v.push(nav("Go to My Tasks", IconName::ListTodo, Route::MyTasks));
                v.push(nav("Go to Today", IconName::Calendar, Route::Today));
                v.push(nav("Go to Upcoming", IconName::Calendar, Route::Upcoming));
                v.push(nav(
                    "Go to Notifications",
                    IconName::Bell,
                    Route::Notifications,
                ));
                v.push(nav("Go to Settings", IconName::Settings, Route::Settings));
                for p in &self.projects {
                    v.push(PaletteEntry {
                        label: format!("{}: Tasks", p.key).into(),
                        icon: IconName::Folder,
                        keywords: vec![p.key.clone().into()],
                        act: PaletteAct::Navigate(Route::Project {
                            id: p.id,
                            label: p.key.clone(),
                        }),
                    });
                    v.push(PaletteEntry {
                        label: format!("{}: Reviews", p.key).into(),
                        icon: IconName::SquareCheck,
                        keywords: vec![p.key.clone().into()],
                        act: PaletteAct::Navigate(Route::Reviews { project: p.id }),
                    });
                }
                v.push(PaletteEntry {
                    label: "Mark all notifications read".into(),
                    icon: IconName::Check,
                    keywords: vec![],
                    act: PaletteAct::MarkAllRead,
                });
                // §19: Switch Tenant（複数 tenant がある時だけ出す）。
                if self.tenants.len() > 1 {
                    for t in &self.tenants {
                        v.push(PaletteEntry {
                            label: format!("Switch to {}", t.name).into(),
                            icon: IconName::Users,
                            keywords: vec!["tenant".into(), t.name.clone().into()],
                            act: PaletteAct::SwitchTenant(t.id),
                        });
                    }
                }
                v.push(PaletteEntry {
                    label: "Refresh tasks".into(),
                    icon: IconName::RefreshCcwDot,
                    keywords: vec![],
                    act: PaletteAct::RefreshTasks,
                });
            }
            PaletteKind::QuickSearch => {
                for r in self.task_list.read(cx).rows_snapshot() {
                    let label = format!("{} {}", r.seq_key, r.title);
                    v.push(PaletteEntry {
                        label: label.clone().into(),
                        icon: IconName::ClipboardList,
                        keywords: vec![r.seq_key.clone().into(), r.title.clone().into()],
                        act: PaletteAct::OpenTask {
                            project: r.project_id,
                            task: r.id,
                        },
                    });
                }
                for p in &self.projects {
                    v.push(PaletteEntry {
                        label: format!("{} — {}", p.key, p.name).into(),
                        icon: IconName::Folder,
                        keywords: vec![p.key.clone().into(), p.name.clone().into()],
                        act: PaletteAct::Navigate(Route::Project {
                            id: p.id,
                            label: p.key.clone(),
                        }),
                    });
                }
            }
        }
        v
    }

    fn palette_confirm(&mut self, path: IndexPath, cx: &mut Context<Self>) {
        let Some(entry) = self.palette_entries.get(path.row).cloned() else {
            self.close_palette(cx);
            return;
        };
        match entry.act {
            PaletteAct::Navigate(route) => self.navigate(route, cx),
            PaletteAct::OpenTask { project, task } => {
                self.navigate(Route::TaskDetail { project, task }, cx)
            }
            PaletteAct::SwitchTenant(id) => self.switch_tenant(id, cx),
            PaletteAct::MarkAllRead => self.center.update(cx, |c, cx| c.mark_all_read(cx)),
            PaletteAct::RefreshTasks => self.task_list.update(cx, |l, cx| l.reload(cx)),
        }
        self.close_palette(cx);
    }

    /// 選択 tenant を切り替えて全 view を再ロードする（§19 Switch Tenant）。
    fn switch_tenant(&mut self, id: uuid::Uuid, cx: &mut Context<Self>) {
        self.settings.last_tenant_id = Some(id);
        let _ = self.settings_store.save(&self.settings);
        if let Some(client) = self.client.clone() {
            let tenant = Some(id);
            self.task_list
                .update(cx, |l, cx| l.set_client(client.clone(), tenant, cx));
            self.task_detail
                .update(cx, |d, _| d.set_client(client.clone(), tenant));
            self.review_list
                .update(cx, |l, _| l.set_client(client.clone(), tenant));
            self.review_detail
                .update(cx, |d, _| d.set_client(client.clone(), tenant));
        }
        self.navigate(Route::MyTasks, cx);
    }

    fn palette_overlay(&self, kind: PaletteKind, cx: &mut Context<Self>) -> impl IntoElement {
        let weak = cx.entity().downgrade();
        let items: Vec<CommandItem> = self
            .palette_entries
            .iter()
            .map(|e| {
                CommandItem::new()
                    .label(e.label.clone())
                    .icon(Icon::new(e.icon))
                    .keywords(e.keywords.clone())
            })
            .collect();
        let placeholder = match kind {
            PaletteKind::Commands => "Type a command…",
            PaletteKind::QuickSearch => "Jump to a task or project…",
        };
        let weak2 = weak.clone();
        div()
            .id("palette-backdrop")
            .absolute()
            .inset_0()
            .flex()
            .justify_center()
            .pt(px(120.))
            .bg(hsla(0.0, 0.0, 0.0, 0.45))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.close_palette(cx)),
            )
            .child(
                div()
                    .w(px(560.))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        Command::new(&self.palette_state)
                            .items(items)
                            .placeholder(placeholder)
                            .max_h(px(420.))
                            .on_cancel(move |_, cx| {
                                let _ = weak2.update(cx, |this, cx| this.close_palette(cx));
                            })
                            .on_confirm(move |path, _, cx| {
                                let _ = weak.update(cx, |this, cx| this.palette_confirm(path, cx));
                            }),
                    ),
            )
    }

    /// プロジェクト配下の Tasks/Reviews タブ（§14）。
    fn project_tabs(
        &self,
        project: uuid::Uuid,
        key: String,
        tasks_active: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let colors = theme::colors(cx);
        let mk = |label: &'static str, active: bool, route: Route| {
            Button::new(SharedString::from(format!("ptab-{label}")))
                .ghost()
                .label(label)
                .when(active, |b| b.text_color(colors.accent))
                .on_click(cx.listener(move |this, _, _, cx| this.navigate(route.clone(), cx)))
        };
        div()
            .flex()
            .flex_row()
            .gap_2()
            .px_4()
            .py_1()
            .border_b_1()
            .border_color(colors.border)
            .child(mk(
                "Tasks",
                tasks_active,
                Route::Project {
                    id: project,
                    label: key,
                },
            ))
            .child(mk("Reviews", !tasks_active, Route::Reviews { project }))
    }

    fn content(&self, _colors: &KoyoriColors, cx: &mut Context<Self>) -> impl IntoElement {
        // feature crate の View が入る場所。
        if self.route == Route::Settings {
            return div().flex_1().h_full().child(self.settings_view.clone());
        }
        if self.route == Route::Notifications {
            return div().flex_1().h_full().child(self.center.clone());
        }
        // タスク系ルートは全て §15 の一覧を表示。
        if matches!(
            self.route,
            Route::MyTasks | Route::Today | Route::Upcoming | Route::TaskDetail { .. }
        ) {
            return div().flex_1().h_full().child(self.task_list.clone());
        }
        // プロジェクト配下は Tasks/Reviews のタブ切替（§14）。
        if let Route::Project { id, label } = &self.route {
            return div()
                .flex_1()
                .h_full()
                .flex()
                .flex_col()
                .child(self.project_tabs(*id, label.clone(), true, cx))
                .child(div().flex_1().min_h_0().child(self.task_list.clone()));
        }
        if let Route::Reviews { project } = &self.route {
            let key = self
                .projects
                .iter()
                .find(|p| p.id == *project)
                .map(|p| p.key.clone())
                .unwrap_or_else(|| "Project".into());
            return div()
                .flex_1()
                .h_full()
                .flex()
                .flex_col()
                .child(self.project_tabs(*project, key, false, cx))
                .child(div().flex_1().min_h_0().child(self.review_list.clone()));
        }
        let title: SharedString = match &self.route {
            Route::Settings => "Settings".into(),
            _ => "—".into(),
        };
        div().flex_1().h_full().p_4().child(
            div()
                .text_xl()
                .font_weight(FontWeight::SEMIBOLD)
                .child(title),
        )
    }

    fn detail(&self, colors: &KoyoriColors) -> impl IntoElement {
        // タスク系ルートでは §15 Detail ペイン。
        if matches!(
            self.route,
            Route::MyTasks
                | Route::Today
                | Route::Upcoming
                | Route::Project { .. }
                | Route::TaskDetail { .. }
        ) {
            return div()
                .h_full()
                .border_l_1()
                .border_color(colors.border)
                .child(self.task_detail.clone());
        }
        if matches!(self.route, Route::Reviews { .. }) {
            return div()
                .h_full()
                .border_l_1()
                .border_color(colors.border)
                .child(self.review_detail.clone());
        }
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

    /// §6 未ログイン画面。client が無い時は shell 全体の代わりに出す。
    fn login_view(&self, colors: &KoyoriColors, cx: &mut Context<Self>) -> impl IntoElement {
        let waiting = self.auth_waiting;
        div()
            .flex_1()
            .h_full()
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_4()
                    .w(px(360.))
                    .child(
                        div()
                            .text_2xl()
                            .font_weight(FontWeight::BOLD)
                            .child("Koyori"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(colors.text_muted)
                            .child("Sign in with your Koyori account"),
                    )
                    .child(
                        Button::new("signin")
                            .primary()
                            .icon(Icon::new(IconName::LogIn))
                            .label(if waiting {
                                "Waiting for browser…"
                            } else {
                                "Sign in with Koyori"
                            })
                            .when(!waiting, |b| {
                                b.on_click(cx.listener(|this, _, _, cx| this.begin_login(cx)))
                            }),
                    )
                    .when_some(self.auth_error.clone(), |d, e| {
                        d.child(div().text_sm().text_color(colors.danger).child(e))
                    }),
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
        let weak_palette = cx.entity().downgrade();
        let weak_search = weak_palette.clone();
        div()
            .id("shell-root")
            .flex()
            .flex_col()
            .size_full()
            .bg(colors.background)
            .text_color(colors.text)
            .on_action::<OpenPalette>(move |_, window, cx| {
                let _ = weak_palette.update(cx, |this, cx| {
                    this.toggle_palette(PaletteKind::Commands, window, cx)
                });
            })
            .on_action::<OpenQuickSearch>(move |_, window, cx| {
                let _ = weak_search.update(cx, |this, cx| {
                    this.toggle_palette(PaletteKind::QuickSearch, window, cx)
                });
            })
            .when(self.client.is_none(), |d| {
                d.child(self.login_view(&colors, cx))
            })
            .when(self.client.is_some(), |d| d.child(self.header(&colors, cx)))
            .when(self.client.is_some(), |d| {
                d.child(
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
                                            if let Some(w) = state.read(cx).sizes().last().copied()
                                            {
                                                shell.update(cx, |this, _| {
                                                    this.settings.window_layout =
                                                        Some(serde_json::json!({
                                                            "detail_width": f64::from(w)
                                                        }));
                                                    let _ =
                                                        this.settings_store.save(&this.settings);
                                                });
                                            }
                                        }
                                    })
                                    .child(resizable_panel().child(self.content(&colors, cx)))
                                    .child(
                                        resizable_panel()
                                            .size(self.detail_width())
                                            .size_range(px(280.)..px(560.))
                                            .child(self.detail(&colors)),
                                    ),
                            ),
                        ),
                )
            })
            .when_some(self.palette, |d, kind| {
                d.child(self.palette_overlay(kind, cx))
            })
    }
}
