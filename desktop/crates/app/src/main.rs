mod palette;
mod shell;
mod theme;

use gpui_kit::component::Root;
use gpui_kit::*;

use shell::AppShell;

fn main() {
    // §8: 「Keep Running in Background」対応のため、最後の Window を
    // 閉じても終了しない（終了は Tray の Quit Koyori からのみ）。
    gpui_kit::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .with_quit_mode(QuitMode::Explicit)
        .run(|cx| {
            gpui_kit::init(cx);

            // 設定と Credential（§6, §22）。Token は OS credential store からだけ読む。
            let settings_store = core::SettingsStore::default_location().unwrap_or_else(|_| {
                core::SettingsStore::at(std::env::temp_dir().join("koyori-settings.json"))
            });
            let settings = settings_store.load();
            // 画面・Tray・OS 通知の文言は起動時の言語で組み立てる。
            i18n::set_language(settings.language);
            theme::apply(settings.appearance, None, cx);

            // §20/§22 キーバインド（settings.keybindings で上書き可）。
            let palette_key = settings
                .keybindings
                .get("command_palette")
                .cloned()
                .unwrap_or_else(|| {
                    if cfg!(target_os = "macos") {
                        "cmd-k"
                    } else {
                        "ctrl-k"
                    }
                    .into()
                });
            let search_key = settings
                .keybindings
                .get("quick_search")
                .cloned()
                .unwrap_or_else(|| {
                    if cfg!(target_os = "macos") {
                        "cmd-p"
                    } else {
                        "ctrl-p"
                    }
                    .into()
                });
            cx.bind_keys([
                KeyBinding::new(&palette_key, shell::OpenPalette, None),
                KeyBinding::new(&search_key, shell::OpenQuickSearch, None),
                KeyBinding::new(
                    if cfg!(target_os = "macos") {
                        "cmd-,"
                    } else {
                        "ctrl-,"
                    },
                    shell::OpenSettings,
                    None,
                ),
            ]);

            // Device Token があればクライアントと同期エンジンを用意する。
            // `KOYORI_API_BASE` / `KOYORI_DEV_TOKEN` があればそちらを優先する
            // dev 経路（mock-api 等でログイン無しに動作確認する用途）。
            let api_base =
                std::env::var("KOYORI_API_BASE").unwrap_or_else(|_| settings.api_base.clone());
            let dev_token = std::env::var("KOYORI_DEV_TOKEN").ok();
            let (client, engine) =
                match dev_token.or_else(|| core::auth::load_token().ok().flatten()) {
                    Some(token) => {
                        let client = api::Client::new(&api_base, &token).ok();
                        let engine = client.clone().map(|c| {
                            core::NotificationEngine::new(
                                c,
                                ::platform::Notifier::new(::platform::WINDOWS_AUMID),
                                settings.notification_cursor.clone(),
                            )
                        });
                        (client, engine)
                    }
                    _ => (None, None),
                };

            cx.spawn(async move |cx| {
                cx.update(|cx| {
                    cx.open_window(
                        WindowOptions {
                            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                                None,
                                size(px(1280.), px(800.)),
                                cx,
                            ))),
                            app_id: Some("app.koyori.desktop".into()),
                            window_min_size: Some(size(px(960.), px(640.))),
                            ..Default::default()
                        },
                        |window, cx| {
                            // §9 System Tray（非対応環境では None → 閉じたら終了）。
                            let tray = ::platform::AppTray::new(
                                include_bytes!("../assets/icon.png"),
                                "Koyori",
                            )
                            .ok();
                            let tray_available = tray.is_some();
                            // §8: Tray があり「Keep Running in Background」なら
                            // Close を Hide に変換して常駐する。
                            let keep_running = std::rc::Rc::new(std::cell::Cell::new(
                                settings.keep_running_in_background && tray_available,
                            ));
                            {
                                let keep_running = keep_running.clone();
                                window.on_window_should_close(cx, move |window, cx| {
                                    if keep_running.get() {
                                        if !::platform::set_window_visible(window, false)
                                            .unwrap_or(false)
                                        {
                                            #[cfg(target_os = "macos")]
                                            cx.hide();
                                            // GPUI's Linux application hide is a no-op. Keep
                                            // the tray session reachable via the taskbar too.
                                            #[cfg(not(target_os = "macos"))]
                                            window.minimize_window();
                                        }
                                        false
                                    } else {
                                        // QuitMode::Explicit では Window を閉じても
                                        // 終了しないため、常駐しない場合は明示的に quit。
                                        cx.quit();
                                        true
                                    }
                                });
                            }
                            let shell = cx.new(|cx| {
                                AppShell::new(
                                    settings,
                                    settings_store,
                                    client,
                                    engine,
                                    tray,
                                    keep_running,
                                    window,
                                    cx,
                                )
                            });
                            let appearance_shell = shell.downgrade();
                            window
                                .observe_window_appearance(move |window, cx| {
                                    let _ = appearance_shell.update(cx, |shell, cx| {
                                        if shell.settings.appearance
                                            == core::settings::Appearance::System
                                        {
                                            theme::apply(
                                                shell.settings.appearance,
                                                Some(window),
                                                cx,
                                            );
                                        }
                                    });
                                })
                                .detach();
                            cx.new(|cx| Root::new(shell, window, cx))
                        },
                    )
                })
                .expect("failed to open window");
            })
            .detach();
        });
}
