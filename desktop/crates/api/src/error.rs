/// 終了コードではなく UI の分岐（desktop.md §23）に対応するエラー分類。
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    /// 401 — 未ログインへ戻し、再承認を促す。
    #[error("unauthorized")]
    Unauthorized,
    /// 403 — backend の message を Toast に出す。
    #[error("forbidden: {message}")]
    Forbidden { message: String },
    /// 404 — 対象が存在しない（通知の飛び先は "no longer exists"）。
    #[error("not found")]
    NotFound,
    /// 409 — backend の message を Toast に出す。
    #[error("conflict: {message}")]
    Conflict { message: String },
    /// その他の 4xx — リクエストの問題。
    #[error("bad request: {message}")]
    BadRequest { message: String },
    /// 通信障害（オフライン・DNS・TLS 等）— Connection Status と自動再試行。
    #[error("transport error: {0}")]
    Transport(#[from] reqwest::Error),
    /// 5xx など予期しないステータス。
    #[error("unexpected status {status}: {message}")]
    Status { status: u16, message: String },
    /// レスポンスのデコード失敗。
    #[error("invalid response body: {0}")]
    Decode(#[from] serde_json::Error),
    /// クライアント設定の問題。
    #[error("invalid client configuration: {0}")]
    InvalidConfig(String),
}

pub type Result<T> = std::result::Result<T, ApiError>;
