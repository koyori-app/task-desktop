//! メインウィンドウの骨組み（desktop.md §14）。
//! Header / Sidebar / Content / Detail。中身は feature crate が後から埋める。

use gpui_kit::assets::IconName;
use gpui_kit::component::badge::Badge;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::command::CommandItem;
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::resizable::{ResizableState, h_resizable, resizable_panel};
use gpui_kit::component::sidebar::{Sidebar, SidebarGroup, SidebarMenuItem};
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{Icon, IndexPath, Root, WindowExt};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use feature_notifications::{CenterEvent, NavTarget, NotificationCenter};
use feature_reviews::{ReviewDetailEvent, ReviewDetailView, ReviewListEvent, ReviewListView};
use feature_settings::{SettingsEvent, SettingsView};
use feature_tasks::{ListMode, TaskDetailEvent, TaskDetailView, TaskListEvent, TaskListView};

use crate::palette::{PaletteKind, PaletteView};
use crate::theme::{self, KoyoriColors};

/// §7: 通知ポーリング間隔。
const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);
/// §9: Tray メニューイベントのポーリング間隔。
const TRAY_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(300);

// §20 Command Palette (Ctrl+K) / Quick Search (Ctrl+P)。
actions!(shell, [OpenPalette, OpenQuickSearch]);

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
    CreateTask,
    SearchTasks,
    StartReview,
    OpenCurrentReview,
    FindingAction(api::types::FindingState),
    ToggleSidebar,
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
    catalog_ready: bool,
    catalog_loading: bool,
    /// Content / Detail の分割位置（§14: レイアウト状態はローカル保存）。
    pub resizable: Entity<ResizableState>,
    review_resizable: Entity<ResizableState>,
    sidebar_visible: bool,
    focus_handle: FocusHandle,
    session_generation: u64,
    search_generation: u64,
    search_loading: bool,
    search_error: Option<SharedString>,
    restore_focus: bool,
    polling: Option<Task<()>>,
    activation_tx: std::sync::mpsc::Sender<api::spec::NotificationItem>,
    /// §9 System Tray。非対応環境では None（閉じたら終了、§8）。
    tray: Option<::platform::AppTray>,
    /// Window close → Hide を動的に切り替える共有フラグ（§8）。
    keep_running: std::rc::Rc<std::cell::Cell<bool>>,
    /// Tray からの再表示に使う Window ハンドル。
    window_handle: AnyWindowHandle,
    /// ポーリングタスクが borrow せず読む通知設定（§10）。
    /// Entity borrow は hot loop から外す（RefCell already borrowed 回避）。
    notif_prefs: std::rc::Rc<std::cell::RefCell<core::settings::NotificationPrefs>>,
    /// §6: ブラウザ承認待ちの間 true（Login 画面の状態表示用）。
    auth_waiting: bool,
    auth_error: Option<SharedString>,
    /// §20 パレット。Some(kind) の間だけオーバーレイ表示。
    palette: Option<PaletteKind>,
    palette_view: Option<Entity<PaletteView>>,
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
        let sub = cx.subscribe(&center, |this, _center, ev: &CenterEvent, cx| match ev {
            CenterEvent::Navigate(target) => this.navigate_target(target.clone(), cx),
            CenterEvent::UnreadCount(count) => this.set_unread_count(*count, cx),
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
        let task_changed = cx.subscribe(&task_detail, |this, _, _: &TaskDetailEvent, cx| {
            this.task_list.update(cx, |list, cx| list.reload(cx));
        });
        let review_navigation = cx.subscribe(
            &review_detail,
            |this, _, event: &ReviewDetailEvent, cx| match event {
                ReviewDetailEvent::OpenTask { project, task } => this.navigate(
                    Route::TaskDetail {
                        project: *project,
                        task: *task,
                    },
                    cx,
                ),
                ReviewDetailEvent::Updated { project } => {
                    if this.route == (Route::Reviews { project: *project }) {
                        this.review_list.update(cx, |list, cx| list.reload(cx));
                    }
                }
            },
        );
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
        settings_view.update(cx, |view, cx| {
            view.set_background_available(tray.is_some(), cx)
        });
        let sub4 = cx.subscribe(
            &settings_view,
            |this, _view, ev: &SettingsEvent, cx| match ev {
                SettingsEvent::Changed(settings) => {
                    // Feature settings can lag behind sync/layout changes. Preserve that state.
                    this.settings.appearance = settings.appearance;
                    this.settings.launch_at_login = settings.launch_at_login;
                    this.settings.keep_running_in_background = settings.keep_running_in_background;
                    this.settings.notifications = settings.notifications.clone();
                    this.settings.keybindings = settings.keybindings.clone();
                    let _ = this.settings_store.save(&this.settings);
                    // §8: Tray ありの時だけ常駐を有効化。
                    this.keep_running
                        .set(settings.keep_running_in_background && this.tray.is_some());
                    *this.notif_prefs.borrow_mut() = settings.notifications.clone();
                    theme::apply(settings.appearance, None, cx);
                    cx.notify();
                }
                SettingsEvent::LoggedOut => {
                    this.end_session(false, cx);
                }
            },
        );

        let notif_prefs = std::rc::Rc::new(std::cell::RefCell::new(settings.notifications.clone()));
        let (activation_tx, activation_rx) = std::sync::mpsc::channel();
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
            catalog_ready: false,
            catalog_loading: false,
            resizable: cx.new(|_| ResizableState::default()),
            review_resizable: cx.new(|_| ResizableState::default()),
            sidebar_visible: true,
            focus_handle: cx.focus_handle(),
            session_generation: 0,
            search_generation: 0,
            search_loading: false,
            search_error: None,
            restore_focus: false,
            polling: None,
            activation_tx,
            tray,
            keep_running,
            window_handle: window.window_handle(),
            notif_prefs,
            auth_waiting: false,
            auth_error: None,
            palette: None,
            palette_view: None,
            palette_entries: vec![],
            _subs: vec![sub, sub2, sub3, sub4, task_changed, review_navigation],
        };
        this.start_polling(engine, cx);
        let focus_lost = cx.on_focus_lost(window, |this, window, cx| {
            // A route change can remove the focused input from the rendered
            // tree. Restore the shell only when GPUI has lost its focus target.
            window.focus(&this.focus_handle, cx);
        });
        this._subs.push(focus_lost);
        this.start_tray_polling(activation_rx, cx);
        window.focus(&this.focus_handle, cx);
        if let Some(client) = client {
            this.bootstrap(client, cx);
        }
        this
    }

    /// 起動時: tenant 解決（未設定なら先頭を保存）→ projects ロード →
    /// task views にクライアントを渡して初期ロード。
    fn bootstrap(&mut self, client: api::Client, cx: &mut Context<Self>) {
        if self.catalog_ready || self.catalog_loading || self.client.is_none() {
            return;
        }
        self.catalog_loading = true;
        let generation = self.session_generation;
        // A project-only retry must preserve the active feature views.
        let known_tenant = self
            .settings
            .last_tenant_id
            .filter(|id| self.tenants.iter().any(|tenant| tenant.id == *id));
        cx.spawn(async move |this, cx| {
            let tenant = if let Some(tenant) = known_tenant {
                Some(tenant)
            } else {
                let tenants = match client.list_tenants().await {
                    Ok(tenants) => tenants,
                    Err(_) => {
                        let _ = this.update(cx, |s, cx| {
                            if generation != s.session_generation || s.client.is_none() {
                                return;
                            }
                            s.catalog_loading = false;
                            s.connection = ConnectionStatus::Offline;
                            cx.notify();
                        });
                        return;
                    }
                };
                this.update(cx, |s, cx| {
                    if generation != s.session_generation || s.client.is_none() {
                        return None;
                    }
                    s.tenants = tenants;
                    if !s
                        .tenants
                        .iter()
                        .any(|t| Some(t.id) == s.settings.last_tenant_id)
                    {
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
                    if tenant.is_none() {
                        // An account with no tenants is a complete, empty catalog.
                        s.projects.clear();
                        s.catalog_ready = true;
                        s.catalog_loading = false;
                    }
                    cx.notify();
                    tenant
                })
                .ok()
                .flatten()
            };
            let Some(tenant) = tenant else {
                return;
            };
            let projects = client.list_projects(tenant).await;
            let _ = this.update(cx, |s, cx| {
                if generation != s.session_generation
                    || s.client.is_none()
                    || s.settings.last_tenant_id != Some(tenant)
                {
                    return;
                }
                s.catalog_loading = false;
                match projects {
                    Ok(projects) => {
                        let keys: Vec<_> = projects.iter().map(|p| (p.id, p.key.clone())).collect();
                        s.task_detail.update(cx, |d, _| d.set_project_keys(keys));
                        s.projects = projects;
                        s.catalog_ready = true;
                    }
                    Err(_) => s.connection = ConnectionStatus::Offline,
                }
                cx.notify();
            });
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
        self.session_generation += 1;
        self.catalog_ready = false;
        self.catalog_loading = false;
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
        let Some(engine) = engine else {
            return;
        };
        let tx = self.activation_tx.clone();
        let mut engine = engine.on_activation(move |item| {
            let _ = tx.send(item);
        });
        // prefs は共有セル経由。ループ先頭で Entity を borrow すると
        // ハンドラの borrow と衝突して panic し得るため。
        let notif_prefs = self.notif_prefs.clone();
        self.polling = Some(cx.spawn(async move |this, cx| {
            loop {
                let prefs = notif_prefs.borrow().clone();
                match engine.tick(&prefs).await {
                    Ok(outcome) => {
                        let cursor = engine.last_cursor().map(str::to_owned);
                        let _ = this.update(cx, |s, cx| s.apply_tick(outcome, cursor, cx));
                    }
                    Err(api::ApiError::Unauthorized) => {
                        let _ = this.update(cx, |s, cx| s.end_session(true, cx));
                        break;
                    }
                    Err(_) => {
                        let _ = this.update(cx, |s, cx| {
                            s.connection = ConnectionStatus::Offline;
                            cx.notify();
                        });
                    }
                }
                cx.background_executor().timer(POLL_INTERVAL).await;
            }
        }));
    }

    /// §9: Tray メニューイベントをポーリングして処理する。
    /// Open → 前面化 / Notifications → 前面化 + Center へ / Quit → 終了。
    fn start_tray_polling(
        &mut self,
        activation_rx: std::sync::mpsc::Receiver<api::spec::NotificationItem>,
        cx: &mut Context<Self>,
    ) {
        // MenuId だけを切り出して受信する。hot loop で Entity を borrow すると
        // クリックハンドラ等の App borrow と衝突して panic するため。
        let ids = self.tray.as_ref().map(|t| t.ids());
        cx.spawn(async move |this, cx| {
            loop {
                while let Ok(item) = activation_rx.try_recv() {
                    let _ = this.update(cx, |s, cx| {
                        if s.client.is_none() {
                            return;
                        }
                        s.navigate_target(
                            NavTarget {
                                project: item.project,
                                target: item.target,
                            },
                            cx,
                        );
                        let _ = cx.update_window(s.window_handle, |_, window, _| {
                            let _ = ::platform::set_window_visible(window, true);
                            window.activate_window();
                        });
                        if let Some(client) = s.client.clone() {
                            let center = s.center.clone();
                            cx.spawn(async move |_, cx| {
                                if client.mark_notification_read(item.id).await.is_ok() {
                                    center.update(cx, |center, cx| center.refresh(cx));
                                }
                            })
                            .detach();
                        }
                    });
                }
                let action = ::platform::poll_menu_event().and_then(|ev| {
                    ids.as_ref()
                        .and_then(|ids| ::platform::AppTray::map_event(&ev, ids))
                });
                if this
                    .update(cx, |s, cx| {
                        if s.client.as_ref().is_some_and(api::Client::is_unauthorized) {
                            s.end_session(true, cx);
                        }
                    })
                    .is_err()
                {
                    break;
                }
                match action {
                    Some(::platform::TrayAction::Open) => {
                        let handle = this.update(cx, |s, _| s.window_handle).ok();
                        if let Some(handle) = handle {
                            let _ = cx.update_window(handle, |_, window, _| {
                                let _ = ::platform::set_window_visible(window, true);
                                window.activate_window();
                            });
                        }
                    }
                    Some(::platform::TrayAction::ShowNotifications) => {
                        let _ = this.update(cx, |s, cx| {
                            s.navigate(Route::Notifications, cx);
                            cx.update_window(s.window_handle, |_, window, _| {
                                let _ = ::platform::set_window_visible(window, true);
                                window.activate_window();
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
        self.set_unread_count(outcome.unread_count, cx);
        self.connection = ConnectionStatus::Online;
        if !self.catalog_ready
            && !self.catalog_loading
            && let Some(client) = self.client.clone()
        {
            self.bootstrap(client, cx);
        }
        if cursor.is_some() && cursor != self.settings.notification_cursor {
            self.settings.notification_cursor = cursor;
            let _ = self.settings_store.save(&self.settings);
        }
        self.center
            .update(cx, |c, cx| c.prepend_new(&outcome.new_items, cx));
        cx.notify();
    }

    fn set_unread_count(&mut self, count: i64, cx: &mut Context<Self>) {
        self.unread_count = count.max(0);
        // §9: 未読数を Tray ツールチップへ反映。
        if let Some(tray) = &self.tray {
            let _ = tray.set_unread_count(self.unread_count);
        }
        cx.notify();
    }

    fn end_session(&mut self, expired: bool, cx: &mut Context<Self>) {
        self.polling = None;
        self.session_generation += 1;
        self.search_generation += 1;
        self.client = None;
        self.tenants.clear();
        self.projects.clear();
        self.catalog_ready = false;
        self.catalog_loading = false;
        self.close_palette(cx);
        self.task_list.update(cx, |l, cx| l.clear_client(cx));
        self.task_detail.update(cx, |d, _| d.clear_client());
        self.review_list.update(cx, |l, _| l.clear_client());
        self.review_detail.update(cx, |d, _| d.clear_client());
        self.center.update(cx, |c, cx| c.clear_client(cx));
        self.settings_view.update(cx, |v, cx| v.clear_client(cx));
        self.settings.notification_cursor = None;
        let _ = self.settings_store.save(&self.settings);
        if expired {
            if std::env::var_os("KOYORI_DEV_TOKEN").is_none() {
                let _ = ::platform::CredentialStore::new(::platform::CREDENTIAL_SERVICE)
                    .delete(::platform::CREDENTIAL_ACCOUNT_TOKEN);
            }
            self.auth_error = Some("Your session expired. Please sign in again.".into());
        }
        self.set_unread_count(0, cx);
        self.route = Route::MyTasks;
        cx.notify();
    }

    /// §11: 通知クリック → target を内部ルートへ変換。
    /// project が取れない古い通知は遷移しない（既読化だけは済んでいる）。
    fn navigate_target(&mut self, target: NavTarget, cx: &mut Context<Self>) {
        let (Some(t), Some(project)) = (target.target, target.project) else {
            return;
        };
        if self.settings.last_tenant_id != Some(project.tenant_id) {
            self.switch_tenant(project.tenant_id, cx);
        }
        let project = project.id;
        let route = match t {
            api::spec::NotificationTarget::Task { task_id } => Route::TaskDetail {
                project,
                task: task_id,
            },
            api::spec::NotificationTarget::Review { review_id } => {
                self.navigate(Route::Reviews { project }, cx);
                self.review_detail
                    .update(cx, |d, cx| d.open_target(review_id, None, cx));
                return;
            }
            api::spec::NotificationTarget::ReviewFinding {
                review_id,
                finding_id,
                ..
            } => {
                self.navigate(Route::Reviews { project }, cx);
                self.review_detail
                    .update(cx, |d, cx| d.open_target(review_id, Some(finding_id), cx));
                return;
            }
        };
        self.navigate(route, cx);
    }

    fn navigate(&mut self, route: Route, cx: &mut Context<Self>) {
        match &route {
            Route::Notifications => self.center.update(cx, |center, cx| center.refresh(cx)),
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
                let key = self
                    .projects
                    .iter()
                    .find(|project| project.id == p)
                    .map(|project| project.key.clone())
                    .unwrap_or_else(|| "Project".into());
                self.task_list
                    .update(cx, |l, cx| l.set_mode(ListMode::Project { id: p, key }, cx));
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

    /// 表示中のプロジェクト（Tasks / Reviews / Task 詳細のいずれでも）。
    fn current_project(&self) -> Option<uuid::Uuid> {
        match &self.route {
            Route::Project { id, .. } => Some(*id),
            Route::Reviews { project } | Route::TaskDetail { project, .. } => Some(*project),
            _ => None,
        }
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

        // Reviews タブや Task 詳細にいる間もどのプロジェクトか分かるよう、
        // route の project で active を決める。
        let current_project = self.current_project();
        let muted = colors.text_muted;
        let project_items: Vec<SidebarMenuItem> = self
            .projects
            .iter()
            .map(|p| {
                let route = Route::Project {
                    id: p.id,
                    label: p.key.clone(),
                };
                let key: SharedString = p.key.clone().into();
                let show_key = p.name != p.key;
                SidebarMenuItem::new(p.name.clone())
                    .icon(IconName::Folder)
                    .active(current_project == Some(p.id))
                    .when(show_key, |item| {
                        item.suffix(move |_, _| {
                            div().text_xs().text_color(muted).child(key.clone())
                        })
                    })
                    .on_click(cx.listener(move |this, _, _, cx| this.navigate(route.clone(), cx)))
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
            .w(px(216.))
            .flex_shrink_0()
            .h_full()
            .border_r_1()
            .border_color(colors.border)
            .child(
                Sidebar::new("sidebar")
                    .w_full()
                    .child(main_group)
                    .child(SidebarGroup::new("Projects").children(project_items))
                    .child(SidebarGroup::new("").child(notifications)),
            )
    }

    fn header(&self, colors: &KoyoriColors, cx: &mut Context<Self>) -> impl IntoElement {
        // §305: テナントは Header で切り替える。UUID ではなく表示名を出す。
        let tenant_id = self.settings.last_tenant_id;
        let tenant_name = self
            .tenants
            .iter()
            .find(|t| Some(t.id) == tenant_id)
            .map(|t| t.name.clone())
            .unwrap_or_else(|| "Tenant".into());
        let tenant_items: Vec<(uuid::Uuid, SharedString)> = self
            .tenants
            .iter()
            .map(|t| (t.id, t.name.clone().into()))
            .collect();
        let shell = cx.entity();

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
            .min_w_0()
            .border_b_1()
            .border_color(colors.border)
            .child(
                Button::new("toggle-sidebar")
                    .ghost()
                    .icon(IconName::Menu)
                    .tooltip("Toggle sidebar")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.sidebar_visible = !this.sidebar_visible;
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::BOLD)
                    .child("Koyori"),
            )
            .child(
                Button::new("tenant-switcher")
                    .ghost()
                    .tooltip("Switch tenant")
                    .label(tenant_name)
                    .icon(IconName::ChevronDown)
                    .dropdown_menu(move |menu, _, _| {
                        let mut m = menu.min_w(px(180.));
                        for (id, name) in tenant_items.iter().cloned() {
                            let shell = shell.clone();
                            m = m.item(
                                PopupMenuItem::new(name)
                                    .checked(Some(id) == tenant_id)
                                    .on_click(move |_, _, cx| {
                                        cx.update_entity(&shell, |s, cx| {
                                            s.switch_tenant(id, cx);
                                        });
                                    }),
                            );
                        }
                        m
                    }),
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
                div()
                    .text_xs()
                    .text_color(colors.text_muted)
                    .child(match self.connection {
                        ConnectionStatus::Online => "Connected",
                        ConnectionStatus::Offline => "Reconnecting…",
                    }),
            )
            .child(
                Button::new("search")
                    .ghost()
                    .icon(IconName::Search)
                    .tooltip("Search tasks and projects (Ctrl+P) · Commands (Ctrl+K)")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.toggle_palette(PaletteKind::QuickSearch, window, cx)
                    })),
            )
            .child(
                Button::new("notifications")
                    .ghost()
                    .icon(IconName::Bell)
                    .tooltip("Notifications")
                    .label(if self.unread_count > 0 {
                        self.unread_count.to_string()
                    } else {
                        String::new()
                    })
                    .on_click(
                        cx.listener(|this, _, _, cx| this.navigate(Route::Notifications, cx)),
                    ),
            )
            .child(
                Button::new("settings")
                    .ghost()
                    .icon(IconName::Settings)
                    .tooltip("Settings")
                    .on_click(cx.listener(|this, _, _, cx| this.navigate(Route::Settings, cx))),
            )
    }

    // ---- §20 Command Palette / Quick Search ----

    fn toggle_palette(&mut self, kind: PaletteKind, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette == Some(kind) {
            window.close_dialog(cx);
            self.close_palette(cx);
            return;
        }
        if self.palette.is_some() {
            window.close_dialog(cx);
        }
        self.search_generation += 1;
        self.search_loading = false;
        self.search_error = None;
        self.palette_entries = self.build_entries(kind, cx);
        self.palette = Some(kind);
        self.restore_focus = false;
        let items = self.palette_items();
        let query_shell = cx.entity().downgrade();
        let confirm_shell = query_shell.clone();
        let cancel_shell = query_shell.clone();
        let view = cx.new(|cx| {
            PaletteView::new(kind, items, window, cx)
                .on_query(move |query, _, cx| {
                    let _ = query_shell.update(cx, |shell, cx| shell.search_tasks(query, cx));
                })
                .on_confirm(move |path, window, cx| {
                    let _ = confirm_shell
                        .update(cx, |shell, cx| shell.palette_confirm(path, window, cx));
                })
                .on_cancel(move |_, cx| {
                    let _ = cancel_shell.update(cx, |shell, cx| shell.close_palette(cx));
                })
        });
        PaletteView::open(&view, window, cx);
        self.palette_view = Some(view);
        cx.notify();
    }

    fn close_palette(&mut self, cx: &mut Context<Self>) {
        self.palette = None;
        self.palette_view = None;
        self.palette_entries.clear();
        self.search_generation += 1;
        self.search_loading = false;
        self.search_error = None;
        self.restore_focus = true;
        cx.notify();
    }

    fn palette_items(&self) -> Vec<CommandItem> {
        self.palette_entries
            .iter()
            .map(|entry| {
                CommandItem::new()
                    .label(entry.label.clone())
                    .icon(Icon::new(entry.icon))
                    .keywords(entry.keywords.clone())
            })
            .collect()
    }

    fn update_palette_results(&self, cx: &mut Context<Self>) {
        if let Some(view) = &self.palette_view {
            let items = self.palette_items();
            view.update(cx, |view, cx| {
                view.set_results(items, self.search_loading, self.search_error.clone(), cx);
            });
        }
    }

    fn search_tasks(&mut self, query: &str, cx: &mut Context<Self>) {
        if self.palette != Some(PaletteKind::QuickSearch) {
            return;
        }
        self.search_generation += 1;
        let generation = self.search_generation;
        let query = query.trim().to_string();
        self.search_error = None;
        if query.is_empty() {
            self.search_loading = false;
            self.palette_entries = self.build_entries(PaletteKind::QuickSearch, cx);
            self.update_palette_results(cx);
            cx.notify();
            return;
        }
        let (Some(client), Some(tenant)) = (self.client.clone(), self.settings.last_tenant_id)
        else {
            return;
        };
        let projects = self.projects.clone();
        let needle = query.to_lowercase();
        self.palette_entries = projects
            .iter()
            .filter(|p| {
                format!("{} {}", p.key, p.name)
                    .to_lowercase()
                    .contains(&needle)
            })
            .map(|p| PaletteEntry {
                label: format!("{} — {}", p.key, p.name).into(),
                icon: IconName::Folder,
                keywords: vec![],
                act: PaletteAct::Navigate(Route::Project {
                    id: p.id,
                    label: p.key.clone(),
                }),
            })
            .collect();
        self.search_loading = true;
        self.update_palette_results(cx);
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(200))
                .await;
            if !this
                .update(cx, |s, _| s.search_generation == generation)
                .unwrap_or(false)
            {
                return;
            }
            let results = futures::future::join_all(projects.into_iter().map(|project| {
                let client = client.clone();
                let query = query.clone();
                async move {
                    let result = client.search_tasks(tenant, project.id, &query).await;
                    (project, result)
                }
            }))
            .await;
            let _ = this.update(cx, |s, cx| {
                if s.palette != Some(PaletteKind::QuickSearch)
                    || s.search_generation != generation
                    || s.settings.last_tenant_id != Some(tenant)
                {
                    return;
                }
                s.search_loading = false;
                for (project, result) in results {
                    match result {
                        Ok(result) => {
                            s.palette_entries
                                .extend(result.tasks.into_iter().map(|task| {
                                    PaletteEntry {
                                        label: format!(
                                            "{}-{} {}",
                                            project.key, task.seq_id, task.title
                                        )
                                        .into(),
                                        icon: IconName::ClipboardList,
                                        keywords: vec![],
                                        act: PaletteAct::OpenTask {
                                            project: project.id,
                                            task: task.id,
                                        },
                                    }
                                }));
                        }
                        Err(error) => s.search_error = Some(error.to_string().into()),
                    }
                }
                s.update_palette_results(cx);
                cx.notify();
            });
        })
        .detach();
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
                if self.review_detail.read(cx).current_review().is_some() {
                    v.push(PaletteEntry {
                        label: "Open Current Review".into(),
                        icon: IconName::SquareCheck,
                        keywords: vec![],
                        act: PaletteAct::OpenCurrentReview,
                    });
                }
                v.push(PaletteEntry {
                    label: "Create Task".into(),
                    icon: IconName::Plus,
                    keywords: vec![],
                    act: PaletteAct::CreateTask,
                });
                v.push(PaletteEntry {
                    label: "Search Tasks".into(),
                    icon: IconName::Search,
                    keywords: vec![],
                    act: PaletteAct::SearchTasks,
                });
                v.push(PaletteEntry {
                    label: "Toggle Sidebar".into(),
                    icon: IconName::Menu,
                    keywords: vec![],
                    act: PaletteAct::ToggleSidebar,
                });
                if matches!(self.route, Route::Reviews { .. }) {
                    v.push(PaletteEntry {
                        label: "Start Review".into(),
                        icon: IconName::Plus,
                        keywords: vec![],
                        act: PaletteAct::StartReview,
                    });
                    for state in self.review_detail.read(cx).available_actions() {
                        use api::types::FindingState;
                        let label = match state {
                            FindingState::Fixed => "Mark Finding as Fixed",
                            FindingState::Verified => "Verify Finding",
                            FindingState::Open => "Reopen Finding",
                            FindingState::Deferred => "Defer Finding",
                            FindingState::Rejected => "Reject Finding",
                        };
                        v.push(PaletteEntry {
                            label: label.into(),
                            icon: IconName::Check,
                            keywords: vec![],
                            act: PaletteAct::FindingAction(state),
                        });
                    }
                }
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

    fn palette_confirm(&mut self, path: IndexPath, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.palette_entries.get(path.row).cloned() else {
            self.close_palette(cx);
            return;
        };
        self.close_palette(cx);
        self.restore_focus = false;
        window.focus(&self.focus_handle, cx);
        match entry.act {
            PaletteAct::Navigate(route) => self.navigate(route, cx),
            PaletteAct::OpenTask { project, task } => {
                self.navigate(Route::TaskDetail { project, task }, cx)
            }
            PaletteAct::SwitchTenant(id) => self.switch_tenant(id, cx),
            PaletteAct::MarkAllRead => self.center.update(cx, |c, cx| c.mark_all_read(cx)),
            PaletteAct::RefreshTasks => self.task_list.update(cx, |l, cx| l.reload(cx)),
            PaletteAct::CreateTask => {
                if !matches!(
                    self.route,
                    Route::Project { .. }
                        | Route::MyTasks
                        | Route::Today
                        | Route::Upcoming
                        | Route::TaskDetail { .. }
                ) {
                    self.navigate(Route::MyTasks, cx);
                }
                self.task_list
                    .update(cx, |l, cx| l.focus_create(window, cx));
            }
            PaletteAct::SearchTasks => self.toggle_palette(PaletteKind::QuickSearch, window, cx),
            PaletteAct::StartReview => self
                .review_list
                .update(cx, |l, cx| l.start_review(window, cx)),
            PaletteAct::OpenCurrentReview => {
                if let Some((project, _, _)) = self.review_detail.read(cx).current_review() {
                    self.route = Route::Reviews { project };
                    self.review_detail
                        .update(cx, |detail, cx| detail.focus_current(window, cx));
                    cx.notify();
                }
            }
            PaletteAct::FindingAction(state) => self
                .review_detail
                .update(cx, |d, cx| d.apply_selected_action(state, cx)),
            PaletteAct::ToggleSidebar => {
                self.sidebar_visible = !self.sidebar_visible;
                cx.notify();
            }
        }
    }

    /// 選択 tenant を切り替えて全 view を再ロードする（§19 Switch Tenant）。
    fn switch_tenant(&mut self, id: uuid::Uuid, cx: &mut Context<Self>) {
        self.session_generation += 1;
        self.search_generation += 1;
        self.projects.clear();
        self.catalog_ready = false;
        self.catalog_loading = false;
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
            self.bootstrap(client, cx);
        }
        self.navigate(Route::MyTasks, cx);
    }

    /// プロジェクト配下の Tasks/Reviews タブ（§14）。
    fn project_tabs(
        &self,
        project: uuid::Uuid,
        key: String,
        tasks_active: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let shell = cx.entity().downgrade();
        TabBar::new("project-tabs")
            .selected_index(if tasks_active { 0 } else { 1 })
            .child(Tab::new().label("Tasks"))
            .child(Tab::new().label("Reviews"))
            .on_click(move |index, _, cx| {
                let route = if *index == 0 {
                    Route::Project {
                        id: project,
                        label: key.clone(),
                    }
                } else {
                    Route::Reviews { project }
                };
                let _ = shell.update(cx, |shell, cx| shell.navigate(route, cx));
            })
    }

    fn content(&self, colors: &KoyoriColors, cx: &mut Context<Self>) -> impl IntoElement {
        // feature crate の View が入る場所。
        if self.route == Route::Settings {
            return div()
                .size_full()
                .min_w_0()
                .overflow_hidden()
                .child(self.settings_view.clone());
        }
        if self.route == Route::Notifications {
            return div()
                .size_full()
                .min_w_0()
                .overflow_hidden()
                .child(self.center.clone());
        }
        // タスク系ルートは全て §15 の一覧を表示。見出しで今どの一覧かを示す。
        let personal = match self.route {
            Route::MyTasks => Some(("My Tasks", "Tasks assigned to you across all projects")),
            Route::Today => Some(("Today", "Assigned to you, due today or overdue")),
            Route::Upcoming => Some(("Upcoming", "Assigned to you, due after today")),
            _ => None,
        };
        if let Some((title, subtitle)) = personal {
            return div()
                .size_full()
                .min_w_0()
                .overflow_hidden()
                .flex()
                .flex_col()
                .child(page_header(colors, title.into(), subtitle.into()))
                .child(div().flex_1().min_h_0().child(self.task_list.clone()));
        }
        // プロジェクト配下は Tasks/Reviews のタブ切替（§14）。
        if let Some(project) = self.current_project() {
            let (key, name) = self
                .projects
                .iter()
                .find(|p| p.id == project)
                .map(|p| (p.key.clone(), p.name.clone()))
                .unwrap_or_else(|| ("Project".into(), "Project".into()));
            let reviews = matches!(self.route, Route::Reviews { .. });
            let subtitle = if name == key {
                "Project".to_string()
            } else {
                format!("Project · {key}")
            };
            return div()
                .size_full()
                .min_w_0()
                .overflow_hidden()
                .flex()
                .flex_col()
                .child(page_header(colors, name.into(), subtitle.into()))
                .child(self.project_tabs(project, key, !reviews, cx))
                .child(div().flex_1().min_h_0().child(if reviews {
                    self.review_list.clone().into_any_element()
                } else {
                    self.task_list.clone().into_any_element()
                }));
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
                .size_full()
                .min_w_0()
                .overflow_hidden()
                .border_l_1()
                .border_color(colors.border)
                .child(self.task_detail.clone());
        }
        if matches!(self.route, Route::Reviews { .. }) {
            return div()
                .size_full()
                .min_w_0()
                .overflow_hidden()
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
        let review = matches!(self.route, Route::Reviews { .. });
        self.settings
            .window_layout
            .as_ref()
            .and_then(|v| {
                v.get(if review {
                    "review_detail_width"
                } else {
                    "detail_width"
                })?
                .as_f64()
            })
            .filter(|w| w.is_finite())
            .map(|w| px((w as f32).clamp(if review { 420. } else { 340. }, 800.)))
            .unwrap_or(px(if review { 660. } else { 440. }))
    }

    fn workspace(&self, colors: &KoyoriColors, cx: &mut Context<Self>) -> AnyElement {
        if matches!(self.route, Route::Settings | Route::Notifications) {
            return self.content(colors, cx).into_any_element();
        }
        let review = matches!(self.route, Route::Reviews { .. });
        h_resizable(if review {
            "review-workspace"
        } else {
            "task-workspace"
        })
        .with_state(if review {
            &self.review_resizable
        } else {
            &self.resizable
        })
        .on_resize({
            let shell = cx.entity();
            move |state, window, cx| {
                if let Some(width) = state.read(cx).sizes().last().copied() {
                    shell.update(cx, |this, cx| {
                        let layout = this
                            .settings
                            .window_layout
                            .get_or_insert_with(|| serde_json::json!({}));
                        layout[if review {
                            "review_detail_width"
                        } else {
                            "detail_width"
                        }] = serde_json::json!(f64::from(width));
                        if let Err(error) = this.settings_store.save(&this.settings) {
                            window.push_notification(error.to_string(), cx);
                        }
                    });
                }
            }
        })
        .child(
            resizable_panel()
                .size_range(px(240.)..Pixels::MAX)
                .child(self.content(colors, cx)),
        )
        .child(
            resizable_panel()
                .size(self.detail_width())
                .flex_none()
                .size_range(px(if review { 420. } else { 340. })..px(1000.))
                .child(self.detail(colors)),
        )
        .into_any_element()
    }
}

impl Render for AppShell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.restore_focus {
            self.restore_focus = false;
            window.close_dialog(cx);
            window.focus(&self.focus_handle, cx);
        }
        let colors = theme::colors(cx);
        let weak_palette = cx.entity().downgrade();
        let weak_search = weak_palette.clone();
        div()
            .id("shell-root")
            .relative()
            .track_focus(&self.focus_handle)
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
                        .overflow_hidden()
                        .when(self.sidebar_visible, |d| d.child(self.sidebar(&colors, cx)))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .h_full()
                                .child(self.workspace(&colors, cx)),
                        ),
                )
            })
            .children(Root::render_notification_layer(window, cx))
            .children(Root::render_dialog_layer(window, cx))
            .children(Root::render_sheet_layer(window, cx))
    }
}

/// Content 上部の見出し。今どの一覧を見ているかを常に示す。
fn page_header(colors: &KoyoriColors, title: SharedString, subtitle: SharedString) -> Div {
    div()
        .flex()
        .flex_col()
        .flex_shrink_0()
        .gap_0p5()
        .px_4()
        .pt_3()
        .pb_2()
        .child(
            div()
                .text_lg()
                .font_weight(FontWeight::SEMIBOLD)
                .text_ellipsis()
                .child(title),
        )
        .child(
            div()
                .text_xs()
                .text_color(colors.text_muted)
                .text_ellipsis()
                .child(subtitle),
        )
}
