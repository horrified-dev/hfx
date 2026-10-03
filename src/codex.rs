//! Direct OpenAI OAuth + PKCE. hfx owns its login and refresh tokens.
//! It never reads or changes the Codex CLI's credentials.
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use reqwest::{Client, RequestBuilder, Url};
use ring::{
    digest,
    rand::{SecureRandom, SystemRandom},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc::Sender},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

const AUTHORIZE: &str = "https://auth.openai.com/oauth/authorize";
const TOKEN: &str = "https://auth.openai.com/oauth/token";
const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const REDIRECT: &str = "http://localhost:1455/auth/callback";
pub const BACKEND: &str = "https://chatgpt.com/backend-api/codex";

#[derive(Clone, Debug, Default)]
pub struct Account {
    pub email: String,
    pub plan: String,
    pub signed_in: bool,
}

// Deliberately no Debug: bearer credentials must not enter logs or UI errors.
#[derive(Clone, Serialize, Deserialize)]
struct Tokens {
    access: String,
    refresh: String,
    expires: u64,
    account_id: String,
    email: String,
    plan: String,
}

#[derive(Clone)]
pub struct Auth {
    tokens: Arc<Mutex<Option<Tokens>>>,
    context_windows: Arc<Mutex<std::collections::BTreeMap<String, u64>>>,
    gate: Arc<tokio::sync::Mutex<()>>,
    path: Option<PathBuf>,
    token_url: String,
    backend: String,
}

pub enum AuthEvent {
    Snapshot {
        account: Account,
        models: Vec<String>,
        error: Option<String>,
    },
    LoginUrl(String),
    Error(String),
}
#[derive(Clone, Copy)]
pub enum AuthAction {
    Status,
    Login,
    Logout,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn random_string(bytes: usize) -> Result<String, String> {
    let mut value = vec![0; bytes];
    SystemRandom::new()
        .fill(&mut value)
        .map_err(|_| "Cannot generate secure login randomness")?;
    Ok(URL_SAFE_NO_PAD.encode(value))
}

fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(digest::digest(&digest::SHA256, verifier.as_bytes()).as_ref())
}

fn authorize_url(verifier: &str, state: &str, redirect: &str) -> String {
    let mut url = Url::parse(AUTHORIZE).expect("Constant authorization URL");
    url.query_pairs_mut().extend_pairs([
        ("response_type", "code"),
        ("client_id", CLIENT_ID),
        ("redirect_uri", redirect),
        ("scope", "openid profile email offline_access"),
        ("code_challenge", &challenge(verifier)),
        ("code_challenge_method", "S256"),
        ("state", state),
        ("id_token_add_organizations", "true"),
        ("codex_cli_simplified_flow", "true"),
        ("originator", "hfx"),
    ]);
    url.into()
}

pub fn valid_login_url(url: &str) -> bool {
    Url::parse(url).is_ok_and(|url| {
        url.scheme() == "https"
            && url.host_str() == Some("auth.openai.com")
            && url.path() == "/oauth/authorize"
            && url.username().is_empty()
            && url.password().is_none()
    })
}

// JWT payloads are routing/display/expiry metadata, not local verification.
// OpenAI authorizes each bearer-authenticated request.
fn claims(token: &str) -> Value {
    token
        .split('.')
        .nth(1)
        .and_then(|s| URL_SAFE_NO_PAD.decode(s.trim_end_matches('=')).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or(Value::Null)
}

fn account_id(claims: &Value) -> Option<&str> {
    claims["chatgpt_account_id"]
        .as_str()
        .or_else(|| {
            claims
                .pointer("/https:~1~1api.openai.com~1auth/chatgpt_account_id")
                .and_then(Value::as_str)
        })
        .or_else(|| {
            claims
                .pointer("/organizations/0/id")
                .and_then(Value::as_str)
        })
}

fn token_data(value: Value, previous: Option<&Tokens>) -> Result<Tokens, String> {
    let access = value["access_token"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or("OpenAI returned no access token")?
        .to_owned();
    let refresh = value["refresh_token"]
        .as_str()
        .filter(|s| !s.is_empty())
        .or_else(|| previous.map(|p| p.refresh.as_str()))
        .ok_or("OpenAI returned no refresh token. Try signing in again.")?
        .to_owned();
    let id = claims(value["id_token"].as_str().unwrap_or(""));
    let jwt = claims(&access);
    let account_id = account_id(&id)
        .or_else(|| account_id(&jwt))
        .or_else(|| previous.map(|p| p.account_id.as_str()))
        .filter(|s| !s.is_empty())
        .ok_or("OpenAI returned no ChatGPT account ID")?
        .to_owned();
    let email = id["email"]
        .as_str()
        .or_else(|| jwt["email"].as_str())
        .or_else(|| previous.map(|p| p.email.as_str()))
        .unwrap_or("ChatGPT account")
        .to_owned();
    let plan = id
        .pointer("/https:~1~1api.openai.com~1auth/chatgpt_plan_type")
        .and_then(Value::as_str)
        .or_else(|| {
            jwt.pointer("/https:~1~1api.openai.com~1auth/chatgpt_plan_type")
                .and_then(Value::as_str)
        })
        .or_else(|| previous.map(|p| p.plan.as_str()))
        .unwrap_or("")
        .to_owned();
    let expires = value["expires_in"]
        .as_u64()
        .map(|seconds| now().saturating_add(seconds))
        .or_else(|| jwt["exp"].as_u64())
        .unwrap_or_else(|| now() + 3600);
    Ok(Tokens {
        access,
        refresh,
        expires,
        account_id,
        email,
        plan,
    })
}

fn client() -> Result<Client, String> {
    Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .user_agent(concat!("hfx/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|_| "Cannot initialize the login HTTP client".into())
}

impl Auth {
    pub fn empty(path: Option<PathBuf>) -> Self {
        Self {
            tokens: Arc::new(Mutex::new(None)),
            context_windows: Default::default(),
            gate: Arc::new(tokio::sync::Mutex::new(())),
            path,
            token_url: TOKEN.into(),
            backend: BACKEND.into(),
        }
    }
    pub fn new(path: Option<PathBuf>) -> Result<Self, String> {
        let tokens =
            if let Some(path) = &path {
                match std::fs::read(path) {
                    Ok(bytes) => Some(serde_json::from_slice::<Tokens>(&bytes).map_err(
                        |_| "The hfx login cache is invalid. Sign out and sign in again.",
                    )?),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                    Err(_) => return Err("Cannot read the hfx login cache".into()),
                }
            } else {
                None
            };
        Ok(Self {
            tokens: Arc::new(Mutex::new(tokens)),
            context_windows: Default::default(),
            gate: Arc::new(tokio::sync::Mutex::new(())),
            path,
            token_url: TOKEN.into(),
            backend: BACKEND.into(),
        })
    }

    pub fn account(&self) -> Account {
        self.tokens
            .lock()
            .expect("Auth lock")
            .as_ref()
            .map(|t| Account {
                signed_in: true,
                email: t.email.clone(),
                plan: t.plan.clone(),
            })
            .unwrap_or_default()
    }

    fn save(&self, tokens: &Tokens) -> Result<(), String> {
        if let Some(path) = &self.path {
            save_private(path, tokens)?;
        }
        *self.tokens.lock().expect("Auth lock") = Some(tokens.clone());
        Ok(())
    }

    async fn exchange(&self, form: &[(&str, &str)]) -> Result<Value, String> {
        let response = client()?
            .post(&self.token_url)
            .form(form)
            .send()
            .await
            .map_err(|_| "Cannot connect to OpenAI authentication")?;
        if !response.status().is_success() {
            return Err(format!(
                "OpenAI authentication returned HTTP {}. Try signing in again.",
                response.status().as_u16()
            ));
        }
        response
            .json()
            .await
            .map_err(|_| "OpenAI authentication returned an invalid token response".into())
    }

    async fn tokens(&self, force_refresh: bool) -> Result<Tokens, String> {
        let _gate = self.gate.lock().await;
        let previous = self
            .tokens
            .lock()
            .expect("Auth lock")
            .clone()
            .ok_or("Sign in with OpenAI in Settings → Providers → Codex.")?;
        if !force_refresh && previous.expires > now() + 30 {
            return Ok(previous);
        }
        let value = self
            .exchange(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", &previous.refresh),
                ("client_id", CLIENT_ID),
            ])
            .await?;
        let refreshed = token_data(value, Some(&previous))?;
        self.save(&refreshed)?;
        Ok(refreshed)
    }

    pub async fn authorize(
        &self,
        request: RequestBuilder,
        force_refresh: bool,
    ) -> Result<RequestBuilder, String> {
        let tokens = self.tokens(force_refresh).await?;
        Ok(request
            .bearer_auth(tokens.access)
            .header("ChatGPT-Account-Id", tokens.account_id)
            .header("originator", "hfx"))
    }

    pub fn endpoint(&self, suffix: &str) -> String {
        format!("{}/{suffix}", self.backend)
    }

    pub fn context_windows(&self) -> std::collections::BTreeMap<String, u64> {
        self.context_windows
            .lock()
            .expect("Model metadata lock")
            .clone()
    }

    pub async fn models(&self) -> Result<Vec<String>, String> {
        let client = client()?;
        for retry in 0..2 {
            let request = client
                .get(self.endpoint("models"))
                .query(&[("client_version", env!("CARGO_PKG_VERSION"))]);
            let response = self
                .authorize(request, retry == 1)
                .await?
                .send()
                .await
                .map_err(|_| "Cannot discover Codex models")?;
            if response.status() == reqwest::StatusCode::UNAUTHORIZED && retry == 0 {
                continue;
            }
            if !response.status().is_success() {
                return Err(format!(
                    "Codex model discovery returned HTTP {}",
                    response.status().as_u16()
                ));
            }
            let value: Value = response
                .json()
                .await
                .map_err(|_| "Codex returned an invalid model list")?;
            let data = value["models"]
                .as_array()
                .or_else(|| value["data"].as_array())
                .ok_or("Codex returned no model list")?;
            for model in data {
                if let Some(id) = model["slug"].as_str().or_else(|| model["id"].as_str())
                    && let Some(limit) = crate::backend::model_context_window(model)
                {
                    self.context_windows
                        .lock()
                        .expect("Model metadata lock")
                        .insert(id.to_owned(), limit);
                }
            }
            let mut models = data
                .iter()
                .filter_map(|m| m["slug"].as_str().or_else(|| m["id"].as_str()))
                .map(str::to_owned)
                .collect::<Vec<_>>();
            models.sort();
            models.dedup();
            return Ok(models);
        }
        unreachable!()
    }

    async fn login(&self, tx: &Sender<AuthEvent>) -> Result<(), String> {
        self.login_at(tx, 1455).await
    }

    async fn login_at(&self, tx: &Sender<AuthEvent>, port: u16) -> Result<(), String> {
        let _gate = self.gate.lock().await;
        // Bind before opening the browser. IPv4 is required; IPv6 optional.
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))
            .await
            .map_err(
                |_| "Cannot open localhost:1455. Finish any other Codex sign-in, then try again.",
            )?;
        let port = listener
            .local_addr()
            .map_err(|_| "Cannot read callback listener address")?
            .port();
        let redirect = if port == 1455 {
            REDIRECT.to_owned()
        } else {
            format!("http://localhost:{port}/auth/callback")
        };
        let ipv6 = TcpListener::bind((std::net::Ipv6Addr::LOCALHOST, port))
            .await
            .ok();
        let verifier = random_string(96)?;
        let state = random_string(32)?;
        tx.send(AuthEvent::LoginUrl(authorize_url(
            &verifier, &state, &redirect,
        )))
        .map_err(|_| "Window closed")?;
        let code=tokio::time::timeout(Duration::from_secs(600),async {
            loop {
                let mut stream=tokio::select! {
                    value=listener.accept()=>value.map_err(|_|"Login callback listener failed")?.0,
                    value=async {match &ipv6 {Some(listener)=>listener.accept().await,None=>std::future::pending().await}}=>value.map_err(|_|"Login callback listener failed")?.0,
                };
                match callback(&mut stream,&state).await? {
                    Some(code)=>break Ok::<_,String>(code),
                    None=>continue,
                }
            }
        }).await.map_err(|_|"OpenAI sign-in expired. Try again.")??;
        let value = self
            .exchange(&[
                ("grant_type", "authorization_code"),
                ("code", &code),
                ("redirect_uri", &redirect),
                ("client_id", CLIENT_ID),
                ("code_verifier", &verifier),
            ])
            .await?;
        self.save(&token_data(value, None)?)
    }

    async fn logout(&self) -> Result<(), String> {
        let _gate = self.gate.lock().await;
        if let Some(path) = &self.path {
            match std::fs::remove_file(path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err("Cannot remove the hfx login cache".into()),
            }
        }
        *self.tokens.lock().expect("Auth lock") = None;
        Ok(())
    }

    pub async fn authenticate(&self, action: AuthAction, tx: Sender<AuthEvent>) {
        let result = async {
            match action {
                AuthAction::Login => self.login(&tx).await?,
                AuthAction::Logout => self.logout().await?,
                AuthAction::Status => {}
            }
            let signed_in = self.account().signed_in;
            // A model discovery failure must not discard a successful login.
            let models = if signed_in {
                self.models().await
            } else {
                Ok(Vec::new())
            };
            let _ = tx.send(AuthEvent::Snapshot {
                account: self.account(),
                models: models.as_ref().cloned().unwrap_or_default(),
                error: models.err(),
            });
            Ok::<_, String>(())
        }
        .await;
        if let Err(error) = result {
            let _ = tx.send(AuthEvent::Error(error));
        }
    }

    #[cfg(test)]
    pub(crate) fn fixture(backend: String) -> Self {
        let mut auth = Self::empty(None);
        auth.backend = backend;
        auth.save(&Tokens {
            access: "fixture-access".into(),
            refresh: "fixture-refresh".into(),
            expires: now() + 3600,
            account_id: "fixture-account".into(),
            email: "fixture@example.com".into(),
            plan: "plus".into(),
        })
        .unwrap();
        auth
    }
}

fn save_private(path: &Path, tokens: &Tokens) -> Result<(), String> {
    use std::io::Write;
    let parent = path.parent().ok_or("Invalid login cache path")?;
    std::fs::create_dir_all(parent).map_err(|_| "Cannot create the hfx login directory")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| "Cannot secure the hfx login directory")?;
    }
    let temp = parent.join(format!(".codex-auth-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&temp)
            .map_err(|_| "Cannot create the hfx login cache")?;
        file.write_all(
            &serde_json::to_vec(tokens).map_err(|_| "Cannot encode the hfx login cache")?,
        )
        .map_err(|_| "Cannot save the hfx login cache")?;
        file.sync_all()
            .map_err(|_| "Cannot sync the hfx login cache")?;
        std::fs::rename(&temp, path).map_err(|_| "Cannot replace the hfx login cache")?;
        Ok::<_, String>(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temp);
    }
    result
}

fn callback_code(target: &str, state: &str) -> Result<Option<String>, String> {
    let url =
        Url::parse(&format!("http://localhost{target}")).map_err(|_| "Invalid login callback")?;
    if url.path() != "/auth/callback" {
        return Ok(None);
    }
    let pairs = url.query_pairs().collect::<Vec<_>>();
    let values = |key: &str| {
        pairs
            .iter()
            .filter(|(k, _)| k == key)
            .map(|(_, v)| v.to_string())
            .collect::<Vec<_>>()
    };
    if values("state") != [state] {
        return Err("Login state mismatch. Restart sign-in from hfx.".into());
    }
    if !values("error").is_empty() {
        return Err("OpenAI sign-in was denied or cancelled.".into());
    }
    let code = values("code");
    if code.len() != 1 || code[0].is_empty() {
        return Err("OpenAI returned no authorization code".into());
    }
    Ok(Some(code[0].clone()))
}

async fn callback(stream: &mut TcpStream, state: &str) -> Result<Option<String>, String> {
    let request = tokio::time::timeout(Duration::from_secs(5), async {
        let mut bytes = Vec::new();
        let mut chunk = [0; 1024];
        while bytes.len() < 8192 {
            let n = stream
                .read(&mut chunk)
                .await
                .map_err(|_| "Cannot read login callback")?;
            if n == 0 {
                break;
            }
            bytes.extend_from_slice(&chunk[..n]);
            if bytes.windows(4).any(|v| v == b"\r\n\r\n") {
                break;
            }
        }
        Ok::<_, String>(bytes)
    })
    .await;
    // A malformed local request shouldn't consume the pending login.
    let Ok(Ok(bytes)) = request else {
        return Ok(None);
    };
    let request = String::from_utf8_lossy(&bytes);
    let mut line = request.lines().next().unwrap_or("").split_whitespace();
    let result = if line.next() == Some("GET") {
        callback_code(line.next().unwrap_or(""), state)
    } else {
        Ok(None)
    };
    let (status, body) = match &result {
        Ok(Some(_)) => (
            "200 OK",
            "<h1>Finishing OpenAI sign-in</h1><p>Return to hfx. It will confirm when sign-in is complete.</p>",
        ),
        Ok(None) => ("404 Not Found", "<h1>Not found</h1>"),
        Err(_) => (
            "400 Bad Request",
            "<h1>Sign-in could not be completed</h1><p>Return to hfx to try again.</p>",
        ),
    };
    let html = format!(
        "<!doctype html><meta charset=utf-8><title>hfx · OpenAI sign-in</title><body style='background:#161817;color:#e5e8e6;font:18px system-ui;padding:60px'>{body}</body>"
    );
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nContent-Security-Policy: default-src 'none'; style-src 'unsafe-inline'\r\nConnection: close\r\n\r\n{html}",
        html.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn jwt(value: Value) -> String {
        format!(
            "fixture.{}.signature",
            URL_SAFE_NO_PAD.encode(value.to_string())
        )
    }
    #[test]
    fn pkce_matches_rfc7636_vector_and_authorization_uses_openai() {
        assert_eq!(
            challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        let verifier = random_string(96).unwrap();
        assert_eq!(verifier.len(), 128);
        let url = authorize_url(&verifier, "fixture-state", REDIRECT);
        assert!(valid_login_url(&url));
        let url = Url::parse(&url).unwrap();
        let params = url
            .query_pairs()
            .collect::<std::collections::HashMap<_, _>>();
        assert_eq!(params["code_challenge_method"], "S256");
        assert_eq!(params["redirect_uri"], REDIRECT);
        assert_eq!(params["state"], "fixture-state");
        assert!(!valid_login_url(
            "https://auth.openai.com.evil.example/oauth/authorize"
        ));
    }
    #[test]
    fn callbacks_validate_state_and_reject_errors_missing_codes_and_duplicates() {
        assert_eq!(
            callback_code("/auth/callback?state=expected&code=valid", "expected").unwrap(),
            Some("valid".into())
        );
        for target in [
            "/auth/callback?state=wrong&code=valid",
            "/auth/callback?state=expected&error=access_denied",
            "/auth/callback?state=expected",
            "/auth/callback?state=expected&state=wrong&code=valid",
        ] {
            assert!(callback_code(target, "expected").is_err());
        }
        assert_eq!(callback_code("/favicon.ico", "expected").unwrap(), None);
    }
    #[test]
    fn account_claims_and_refresh_rotation_preserve_metadata_without_id_token() {
        let token=token_data(json!({"access_token":jwt(json!({"chatgpt_account_id":"account"})),"refresh_token":"r1","id_token":jwt(json!({"email":"fixture@example.com","https://api.openai.com/auth":{"chatgpt_account_id":"account","chatgpt_plan_type":"plus"}})),"expires_in":3600}),None).unwrap();
        let updated = token_data(
            json!({"access_token":"new-access","refresh_token":"r2","expires_in":3600}),
            Some(&token),
        )
        .unwrap();
        assert_eq!(updated.account_id, "account");
        assert_eq!(updated.refresh, "r2");
        assert_eq!(updated.email, "fixture@example.com");
        let retained = token_data(
            json!({"access_token":"newer-access","expires_in":3600}),
            Some(&updated),
        )
        .unwrap();
        assert_eq!(retained.refresh, "r2");
        for value in [
            json!({"chatgpt_account_id":"a"}),
            json!({"https://api.openai.com/auth":{"chatgpt_account_id":"a"}}),
            json!({"organizations":[{"id":"a"}]}),
        ] {
            assert_eq!(account_id(&value), Some("a"));
        }
    }
    #[test]
    fn cache_persists_privately_and_logout_removes_only_hfx_credentials() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("hfx/codex-auth.json");
        let auth = Auth::new(Some(path.clone())).unwrap();
        let token = Tokens {
            access: "fixture-access".into(),
            refresh: "fixture-refresh".into(),
            expires: now() + 3600,
            account_id: "account".into(),
            email: "fixture@example.com".into(),
            plan: "plus".into(),
        };
        auth.save(&token).unwrap();
        assert!(Auth::new(Some(path.clone())).unwrap().account().signed_in);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(auth.logout())
            .unwrap();
        assert!(!auth.account().signed_in);
    }

    async fn server(
        responses: Vec<(u16, Value)>,
    ) -> (String, tokio::task::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let mut requests = Vec::new();
            for (status, body) in responses {
                let (mut stream, _) =
                    tokio::time::timeout(Duration::from_secs(5), listener.accept())
                        .await
                        .unwrap()
                        .unwrap();
                let mut bytes = Vec::new();
                let mut chunk = [0; 4096];
                loop {
                    let n = stream.read(&mut chunk).await.unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&chunk[..n]);
                    if let Some(offset) = bytes.windows(4).position(|s| s == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&bytes[..offset]);
                        let length = headers
                            .lines()
                            .find_map(|s| {
                                s.to_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse::<usize>().unwrap())
                            })
                            .unwrap_or(0);
                        if bytes.len() >= offset + 4 + length {
                            break;
                        }
                    }
                }
                requests.push(String::from_utf8(bytes).unwrap());
                let body = body.to_string();
                stream.write_all(format!("HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
            }
            requests
        });
        (base, task)
    }

    fn form(request: &str) -> std::collections::HashMap<String, String> {
        Url::parse(&format!(
            "http://localhost/?{}",
            request.split("\r\n\r\n").nth(1).unwrap()
        ))
        .unwrap()
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn browser_callback_exchanges_the_exact_pkce_verifier_and_saves_login() {
        let root = tempfile::tempdir().unwrap();
        let (base,server)=server(vec![(200,json!({"access_token":jwt(json!({"chatgpt_account_id":"fixture-account","email":"fixture@example.com"})),"refresh_token":"fixture-refresh","expires_in":3600}))]).await;
        let mut auth = Auth::empty(Some(root.path().join("auth/codex-auth.json")));
        auth.token_url = format!("{base}/oauth/token");
        let (tx, rx) = std::sync::mpsc::channel();
        let login = auth.clone();
        let task = tokio::spawn(async move { login.login_at(&tx, 0).await });
        let url = match rx.recv_timeout(Duration::from_secs(5)).unwrap() {
            AuthEvent::LoginUrl(url) => Url::parse(&url).unwrap(),
            _ => panic!("Expected login URL"),
        };
        let params = url
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect::<std::collections::HashMap<_, _>>();
        let mut callback = Url::parse(&params["redirect_uri"]).unwrap();
        callback
            .query_pairs_mut()
            .append_pair("code", "fixture-code")
            .append_pair("state", &params["state"]);
        let page = client().unwrap().get(callback).send().await.unwrap();
        assert!(page.status().is_success());
        assert!(!page.text().await.unwrap().contains("fixture-code"));
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(auth.account().signed_in);
        let requests = server.await.unwrap();
        let form = form(&requests[0]);
        assert_eq!(form["grant_type"], "authorization_code");
        assert_eq!(form["code"], "fixture-code");
        assert_eq!(form["redirect_uri"], params["redirect_uri"]);
        assert_eq!(form["client_id"], CLIENT_ID);
        assert_eq!(challenge(&form["code_verifier"]), params["code_challenge"]);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn near_expiry_refresh_then_401_retry_rotate_tokens_and_discover_models() {
        let refresh1 = json!({"access_token":"fixture-access-2","refresh_token":"fixture-refresh-2","expires_in":3600});
        let refresh2 = json!({"access_token":"fixture-access-3","refresh_token":"fixture-refresh-3","expires_in":3600});
        let (base, server) = server(vec![
            (200, refresh1),
            (401, json!({"error":"expired"})),
            (200, refresh2),
            (
                200,
                json!({"models":[{"slug":"fixture-model","context_window":272000}]}),
            ),
        ])
        .await;
        let root = tempfile::tempdir().unwrap();
        let mut auth = Auth::fixture(base.clone());
        auth.path = Some(root.path().join("hfx/codex-auth.json"));
        auth.token_url = format!("{base}/oauth/token");
        auth.tokens.lock().unwrap().as_mut().unwrap().expires = now() + 10;
        assert_eq!(auth.models().await.unwrap(), ["fixture-model"]);
        assert_eq!(auth.context_windows().get("fixture-model"), Some(&272000));
        let requests = server.await.unwrap();
        assert_eq!(form(&requests[0])["refresh_token"], "fixture-refresh");
        assert_eq!(form(&requests[2])["refresh_token"], "fixture-refresh-2");
        assert!(
            requests[1]
                .to_lowercase()
                .contains("authorization: bearer fixture-access-2")
        );
        assert!(
            requests[3]
                .to_lowercase()
                .contains("authorization: bearer fixture-access-3")
        );
        assert!(
            requests[3]
                .to_lowercase()
                .contains("chatgpt-account-id: fixture-account")
        );
        assert!(requests[3].contains("client_version="));
        let reloaded = Auth::new(auth.path.clone()).unwrap();
        assert_eq!(
            reloaded.tokens.lock().unwrap().as_ref().unwrap().refresh,
            "fixture-refresh-3"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancelled_login_releases_callback_listener_without_credentials() {
        let auth = Auth::empty(None);
        let (tx, rx) = std::sync::mpsc::channel();
        let login = auth.clone();
        let task = tokio::spawn(async move { login.login_at(&tx, 0).await });
        let url = match rx.recv_timeout(Duration::from_secs(5)).unwrap() {
            AuthEvent::LoginUrl(url) => Url::parse(&url).unwrap(),
            _ => panic!("Expected login URL"),
        };
        let redirect = url
            .query_pairs()
            .find(|(k, _)| k == "redirect_uri")
            .unwrap()
            .1
            .into_owned();
        let port = Url::parse(&redirect).unwrap().port().unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(!auth.account().signed_in);
        assert!(TcpListener::bind(("127.0.0.1", port)).await.is_ok());
    }
}
