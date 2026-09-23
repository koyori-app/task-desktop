use crate::error::{Error, Result};

/// システムブラウザで URL を開く。認証の authorize ページや
/// 「ブラウザで開く」導線に使う（desktop.md §6）。
pub fn open_url(url: &str) -> Result<()> {
    opener::open(url).map_err(|e| Error::Opener(e.to_string()))
}
