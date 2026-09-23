use crate::error::Result;

pub use global_hotkey::GlobalHotKeyEvent;
pub use global_hotkey::hotkey::HotKey as GlobalHotKey;

/// グローバルショートカット（バックグラウンド時に前面へ呼び出す等）。
/// イベントはプロセス全体で 1 つのレシーバを共有する。
pub struct HotKeyManager {
    inner: global_hotkey::GlobalHotKeyManager,
}

/// 登録済みホットキーのハンドル。unregister に使う。
pub type HotKeyHandle = u32;

impl HotKeyManager {
    pub fn new() -> Result<Self> {
        Ok(Self {
            inner: global_hotkey::GlobalHotKeyManager::new()?,
        })
    }

    pub fn register(&self, key: GlobalHotKey) -> Result<HotKeyHandle> {
        let id = key.id();
        self.inner.register(key)?;
        Ok(id)
    }

    pub fn unregister(&self, key: GlobalHotKey) -> Result<()> {
        self.inner.unregister(key).map_err(Into::into)
    }

    /// 新しいイベントを 1 件だけ取り出す。無ければ `None`。
    /// app 層が定期ポーリングして `HotKeyState::Pressed` を処理する。
    pub fn poll_event() -> Option<GlobalHotKeyEvent> {
        global_hotkey::GlobalHotKeyEvent::receiver().try_recv().ok()
    }
}
