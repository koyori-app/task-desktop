use crate::error::Result;

/// OS credential store への薄いラッパー（keyring）。
/// Windows Credential Manager / macOS Keychain / Linux Secret Service。
/// Token を平文ファイルやログに出さないための唯一の出入り口（desktop.md §6）。
pub struct CredentialStore {
    service: &'static str,
}

impl CredentialStore {
    pub const fn new(service: &'static str) -> Self {
        Self { service }
    }

    /// 未登録は `Ok(None)`。ストア自体の障害は `Err`。
    pub fn get(&self, account: &str) -> Result<Option<String>> {
        let entry = keyring::Entry::new(self.service, account)?;
        match entry.get_password() {
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn set(&self, account: &str, secret: &str) -> Result<()> {
        keyring::Entry::new(self.service, account)?
            .set_password(secret)
            .map_err(Into::into)
    }

    /// 未登録でも成功扱い（Logout の冪等性）。
    pub fn delete(&self, account: &str) -> Result<()> {
        match keyring::Entry::new(self.service, account)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}
