#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Api(#[from] api::ApiError),
    #[error(transparent)]
    Platform(#[from] platform::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// ブラウザが state を改ざん/混線した（別の承認フローの応答）。
    #[error("oauth state mismatch")]
    StateMismatch,
    /// 5 分待っても callback が来なかった（desktop.md §6）。
    #[error("authorization timed out")]
    AuthTimeout,
    /// `?error=` 付きで戻った。
    #[error("authorization failed: {0}")]
    AuthRejected(String),
    /// callback に code が無かった。
    #[error("callback missing code")]
    MissingCode,
    /// callback リクエストが読めない形。
    #[error("malformed callback request")]
    MalformedCallback,
    /// web_base など設定値の異常。
    #[error("invalid configuration: {0}")]
    InvalidConfig(String),
    /// 設定のシリアライズ失敗。
    #[error("serialization: {0}")]
    Serialize(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
