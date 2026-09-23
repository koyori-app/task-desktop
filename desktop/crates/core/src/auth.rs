//! Device Token 認証フロー（desktop.md §6 / task.md §17）。
//! loopback + PKCE。Custom URI Scheme は使わない。

use std::{
    io::{Read, Write},
    net::TcpListener,
    time::{Duration, Instant},
};

use base64::Engine;
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};

/// `state` 用のエントロピー（uuid v4 の 128bit）。
fn random_token() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// PKCE code_verifier: 64 hex 文字（256bit、unreserved 文字のみ）。
fn code_verifier() -> String {
    format!("{}{}", random_token(), random_token())
}

fn code_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

/// `start` で bind した loopback listener と PKCE 一式。
/// ブラウザ承認待ちの間だけ生きる一時オブジェクト。
pub struct PendingAuth {
    listener: TcpListener,
    authorize_url: String,
    state: String,
    code_verifier: String,
}

/// `wait_for_grant` が返す交換材料。
pub struct AuthGrant {
    pub code: String,
    pub code_verifier: String,
}

impl PendingAuth {
    /// 127.0.0.1 の空きポートに bind し、authorize URL を組み立てる。
    /// `web_base` は `https://task.koyori.app` のような Web 側オリジン。
    /// `device_name` は端末管理に表示される名前（§17）。
    pub fn start(web_base: &str, device_name: &str) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        listener.set_nonblocking(true)?;

        let code_verifier = code_verifier();
        let challenge = code_challenge(&code_verifier);
        let state = random_token();

        let mut url = url::Url::parse(web_base)
            .and_then(|u| u.join("desktop/authorize"))
            .map_err(|e| Error::InvalidConfig(format!("web_base {web_base:?}: {e}")))?;
        url.query_pairs_mut()
            .append_pair("port", &port.to_string())
            .append_pair("code_challenge", &challenge)
            .append_pair("state", &state)
            .append_pair("name", device_name);

        Ok(Self {
            listener,
            authorize_url: url.into(),
            state,
            code_verifier,
        })
    }

    /// システムブラウザで開く URL（`platform::open_url` に渡す）。
    pub fn authorize_url(&self) -> &str {
        &self.authorize_url
    }

    /// callback をブロッキングで待つ。UI からは別スレッドで呼ぶ。
    /// `timeout` は desktop.md §6 の 5 分を呼び出し側が渡す想定。
    pub fn wait_for_grant(self, timeout: Duration) -> Result<AuthGrant> {
        let deadline = Instant::now() + timeout;
        loop {
            match self.listener.accept() {
                Ok((mut stream, _)) => {
                    if let Some(result) = handle_connection(&mut stream, &self.state) {
                        return result.map(|code| AuthGrant {
                            code,
                            code_verifier: self.code_verifier,
                        });
                    }
                    // 無関係なリクエスト（favicon 等）は捨てて待ち続ける
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return Err(Error::AuthTimeout);
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(e) => return Err(e.into()),
            }
        }
    }
}

/// 1 接続を処理。`/callback` への GET なら `Some(Ok(code))`。
/// ブラウザには成否を問わず応答を返す。
fn handle_connection(
    stream: &mut std::net::TcpStream,
    expected_state: &str,
) -> Option<Result<String>> {
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .ok()?;

    let mut buf = vec![0u8; 8192];
    let n = stream.read(&mut buf).ok()?;
    let request = String::from_utf8_lossy(&buf[..n]);
    let request_line = request.lines().next()?;
    // "GET /callback?code=..&state=.. HTTP/1.1"
    let path = request_line.split_whitespace().nth(1)?;

    let parsed = url::Url::parse(&format!("http://localhost{path}")).ok()?;
    if parsed.path() != "/callback" {
        return None;
    }

    let mut code = None;
    let mut state = None;
    let mut error = None;
    for (k, v) in parsed.query_pairs() {
        match k.as_ref() {
            "code" => code = Some(v.into_owned()),
            "state" => state = Some(v.into_owned()),
            "error" => error = Some(v.into_owned()),
            _ => {}
        }
    }

    let (body, result) = if let Some(e) = error {
        (
            "Authorization failed.".to_string(),
            Err(Error::AuthRejected(e)),
        )
    } else if state.as_deref() != Some(expected_state) {
        ("State mismatch.".to_string(), Err(Error::StateMismatch))
    } else if let Some(code) = code {
        ("Signed in. You can return to Koyori.".to_string(), Ok(code))
    } else {
        ("Missing code.".to_string(), Err(Error::MissingCode))
    };

    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();

    Some(result)
}

/// code を Device Token へ交換し、OS credential store へ保存する。
/// 戻り値は保存した token。呼び出し側はログに出さないこと（§6）。
pub async fn redeem(api_base: &str, grant: &AuthGrant) -> Result<String> {
    // 交換口は未認証だが Client は Bearer を付ける（code 自体が資格、実害なし）
    let client = api::Client::new(api_base, "")?;
    let token = client
        .exchange_desktop_code(&grant.code, &grant.code_verifier)
        .await?;
    platform::CredentialStore::new(platform::CREDENTIAL_SERVICE)
        .set(platform::CREDENTIAL_ACCOUNT_TOKEN, &token.token)?;
    Ok(token.token)
}

/// 保存済み Device Token を読む。未ログインは `Ok(None)`。
pub fn load_token() -> Result<Option<String>> {
    Ok(platform::CredentialStore::new(platform::CREDENTIAL_SERVICE)
        .get(platform::CREDENTIAL_ACCOUNT_TOKEN)?)
}

/// Logout = 自分の device を DELETE + credential store から削除（§6）。
/// `device_id` は `list_devices` で現在端末を突き合わせて渡す。
/// サーバ側削除に失敗してもローカルの token は消す（401 後の復旧を妨げない）。
pub async fn logout(api_base: &str, device_id: uuid::Uuid) -> Result<()> {
    let Some(token) = load_token()? else {
        return Ok(());
    };
    let client = api::Client::new(api_base, &token)?;
    // DELETE が失敗しても続行してローカルだけは確実に消す
    let _ = client.delete_device(device_id).await;
    platform::CredentialStore::new(platform::CREDENTIAL_SERVICE)
        .delete(platform::CREDENTIAL_ACCOUNT_TOKEN)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 接続済みソケットペアを作って GET を流し、handle_connection の結果を見る。
    fn run_callback(request: &str) -> Result<String> {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let mut client = std::net::TcpStream::connect(addr).unwrap();
        client.write_all(request.as_bytes()).unwrap();
        let (mut server, _) = listener.accept().unwrap();
        handle_connection(&mut server, "test-state").expect("no callback parsed")
    }

    #[test]
    fn callback_returns_code_on_state_match() {
        let code =
            run_callback("GET /callback?code=abc123&state=test-state HTTP/1.1\r\nHost: x\r\n\r\n")
                .unwrap();
        assert_eq!(code, "abc123");
    }

    #[test]
    fn callback_rejects_state_mismatch() {
        let err = run_callback("GET /callback?code=abc&state=wrong HTTP/1.1\r\n\r\n").unwrap_err();
        assert!(matches!(err, Error::StateMismatch));
    }

    #[test]
    fn callback_surfaces_error_param() {
        let err =
            run_callback("GET /callback?error=access_denied&state=test-state HTTP/1.1\r\n\r\n")
                .unwrap_err();
        assert!(matches!(err, Error::AuthRejected(ref e) if e == "access_denied"));
    }

    #[test]
    fn non_callback_path_is_ignored() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let mut client = std::net::TcpStream::connect(addr).unwrap();
        client
            .write_all(b"GET /favicon.ico HTTP/1.1\r\n\r\n")
            .unwrap();
        let (mut server, _) = listener.accept().unwrap();
        assert!(handle_connection(&mut server, "test-state").is_none());
    }

    #[test]
    fn pkce_verifier_is_unreserved_and_challenge_is_s256() {
        let v = code_verifier();
        assert_eq!(v.len(), 64);
        assert!(
            v.chars()
                .all(|c| c.is_ascii_alphanumeric() || "-._~".contains(c))
        );
        // challenge = BASE64URL(SHA256(verifier))、パディング無し
        let c = code_challenge(&v);
        assert_eq!(c.len(), 43);
        assert!(
            c.chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_".contains(c))
        );
    }

    #[test]
    fn authorize_url_carries_port_state_challenge_name() {
        let pending = PendingAuth::start("https://task.koyori.app/", "my-pc").unwrap();
        let url = url::Url::parse(pending.authorize_url()).unwrap();
        assert_eq!(url.path(), "/desktop/authorize");
        let params: std::collections::HashMap<_, _> = url.query_pairs().collect();
        assert_eq!(params["name"], "my-pc");
        assert!(params["port"].parse::<u16>().is_ok());
        assert_eq!(params["code_challenge"].len(), 43);
        assert_eq!(params["state"].len(), 32);
    }
}
