//! ユーザーアイコン（`avatar_url`）の取得。
//!
//! `avatar_url` は外部ホストのこともあるので Device Token を付けない専用の
//! クライアントで取る。巨大なファイルや画像以外は受け取らない。

use std::sync::OnceLock;

use reqwest::Url;

use crate::error::{ApiError, Result};

/// これを超えるアイコンは捨てる（一覧に何十個も並ぶため）。
const MAX_BYTES: usize = 2 * 1024 * 1024;

fn http() -> &'static reqwest::Client {
    static HTTP: OnceLock<reqwest::Client> = OnceLock::new();
    HTTP.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .expect("avatar http client")
    })
}

/// 取得したアイコン。`content_type` は `image/png` 等（無ければ None）。
#[derive(Debug, Clone)]
pub struct AvatarBytes {
    pub bytes: Vec<u8>,
    pub content_type: Option<String>,
}

pub async fn fetch_avatar(url: &str) -> Result<AvatarBytes> {
    let url = Url::parse(url).map_err(|e| ApiError::InvalidConfig(format!("{url}: {e}")))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(ApiError::InvalidConfig(format!(
            "unsupported avatar scheme: {}",
            url.scheme()
        )));
    }
    crate::client::on_runtime(async move {
        let mut response = http().get(url).send().await?.error_for_status()?;
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.split(';').next().unwrap_or(v).trim().to_ascii_lowercase());
        if content_type
            .as_deref()
            .is_some_and(|t| !t.starts_with("image/"))
        {
            return Err(ApiError::InvalidConfig(format!(
                "avatar is not an image: {}",
                content_type.unwrap_or_default()
            )));
        }
        if response
            .content_length()
            .is_some_and(|len| len as usize > MAX_BYTES)
        {
            return Err(ApiError::InvalidConfig("avatar is too large".into()));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            bytes.extend_from_slice(&chunk);
            if bytes.len() > MAX_BYTES {
                return Err(ApiError::InvalidConfig("avatar is too large".into()));
            }
        }
        Ok(AvatarBytes {
            bytes,
            content_type,
        })
    })
    .await
}
