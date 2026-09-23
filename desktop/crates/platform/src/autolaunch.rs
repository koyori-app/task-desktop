use crate::error::Result;

/// ログイン時起動（auto-launch）。設定画面のトグルから呼ぶ。
pub struct AutoLaunchHandle {
    inner: auto_launch::AutoLaunch,
}

impl AutoLaunchHandle {
    /// `app_name` は Windows のレジストリ Run キー名 / macOS LaunchAgent 名 /
    /// Linux の .desktop 名として使われる。
    pub fn new(app_name: &str) -> Result<Self> {
        let exe = std::env::current_exe()?;
        let exe = exe.to_string_lossy();
        let inner = auto_launch::AutoLaunchBuilder::new()
            .set_app_name(app_name)
            .set_app_path(&exe)
            .set_windows_enable_mode(auto_launch::WindowsEnableMode::CurrentUser)
            .build()?;
        Ok(Self { inner })
    }

    pub fn enable(&self) -> Result<()> {
        self.inner.enable().map_err(Into::into)
    }

    pub fn disable(&self) -> Result<()> {
        self.inner.disable().map_err(Into::into)
    }

    pub fn is_enabled(&self) -> Result<bool> {
        self.inner.is_enabled().map_err(Into::into)
    }
}
