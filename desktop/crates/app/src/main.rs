mod shell;
mod theme;

use gpui_kit::component::Root;
use gpui_kit::*;

use shell::AppShell;

fn main() {
    gpui_kit::application().run(|cx| {
        gpui_kit::init(cx);

        // 設定と Credential（§6, §22）。Token は OS credential store からだけ読む。
        let settings_store = core::SettingsStore::default_location().unwrap_or_else(|_| {
            core::SettingsStore::at(std::env::temp_dir().join("koyori-settings.json"))
        });
        let settings = settings_store.load();
        theme::apply(settings.appearance, None, cx);

        // Device Token があればクライアントと同期エンジンを用意する。
        // 無ければ未ログイン（認証画面の結線は別タスク）。
        let (client, engine) = match core::auth::load_token() {
            Ok(Some(token)) => {
                let client = api::Client::new(&settings.api_base, &token).ok();
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
                        ..Default::default()
                    },
                    |window, cx| {
                        // OS 外観の変化に追従（Appearance::System のときだけ意味を持つ）。
                        window
                            .observe_window_appearance(move |_, cx| {
                                if settings.appearance == core::settings::Appearance::System {
                                    theme::apply(settings.appearance, None, cx);
                                }
                            })
                            .detach();
                        let shell =
                            cx.new(|cx| AppShell::new(settings, settings_store, client, engine, cx));
                        cx.new(|cx| Root::new(shell, window, cx))
                    },
                )
            })
            .expect("failed to open window");
        })
        .detach();
    });
}
