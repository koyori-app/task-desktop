#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("credential store: {0}")]
    Credential(#[from] keyring::Error),
    #[error("os notification: {0}")]
    Notification(String),
    #[error("auto-launch: {0}")]
    AutoLaunch(#[from] auto_launch::Error),
    #[error("global hotkey: {0}")]
    HotKey(#[from] global_hotkey::Error),
    #[error("tray: {0}")]
    Tray(#[from] tray_icon::Error),
    #[error("tray icon: {0}")]
    BadIcon(#[from] tray_icon::BadIcon),
    #[error("tray menu: {0}")]
    Menu(#[from] muda::Error),
    #[error("icon decode: {0}")]
    Image(#[from] image::ImageError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("open url: {0}")]
    Opener(String),
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;
