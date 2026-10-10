//! Independent MCP credentials and a small, single-owner OAuth authorization server.
//!
//! OAuth registrations and hashed tokens survive restarts. Short-lived authorization
//! requests and one-use codes remain in memory. No plaintext token is saved on disk.

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use axum::{
    Form, Json, Router,
    extract::{DefaultBodyLimit, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use url::Url;

const READ: &str = "picsoc:read";
const WRITE: &str = "picsoc:write";
const ACCESS_SECONDS: u64 = 3600;
const REFRESH_SECONDS: u64 = 7 * 24 * 3600;
const MAX_CLIENTS: usize = 128;
const MAX_PENDING: usize = 128;
const MAX_CODES: usize = 128;
const MAX_TOKENS: usize = 512;

#[derive(Clone)]
pub struct McpAuth {
    inner: Arc<Inner>,
}

struct Inner {
    enabled: bool,
    password: Option<Arc<str>>,
    token: Option<Arc<str>>,
    base: Option<Url>,
    redirects: Vec<String>,
    client_path: PathBuf,
    store: Mutex<Store>,
}

#[derive(Default)]
struct Store {
    clients: HashMap<String, Client>,
    pending: HashMap<String, Pending>,
    codes: HashMap<String, Grant>,
    access: HashMap<String, Token>,
    refresh: HashMap<String, Token>,
    used_refresh: HashMap<String, (String, String, Instant)>,
    attempts: Vec<Instant>,
    registrations: Vec<Instant>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Client {
    name: String,
    redirect_uris: Vec<String>,
    auth_method: String,
    secret_hash: Option<String>,
}

#[derive(Clone)]
struct Pending {
    grant: Grant,
    state: Option<String>,
    expires: Instant,
    failures: u8,
}

#[derive(Clone)]
struct Grant {
    client_id: String,
    redirect_uri: String,
    challenge: String,
    scope: String,
    resource: String,
    expires: Instant,
}

#[derive(Clone)]
struct Token {
    client_id: String,
    scope: String,
    resource: String,
    family: String,
    expires: Instant,
}

#[derive(Serialize, Deserialize)]
struct PersistedStore {
    version: u8,
    clients: HashMap<String, Client>,
    access: HashMap<String, PersistedToken>,
    refresh: HashMap<String, PersistedToken>,
    used_refresh: HashMap<String, (String, String, u64)>,
}

#[derive(Serialize, Deserialize)]
struct PersistedToken {
    client_id: String,
    scope: String,
    resource: String,
    family: String,
    expires_unix: u64,
}

impl PersistedToken {
    fn from_token(token: &Token) -> Self {
        Self {
            client_id: token.client_id.clone(),
            scope: token.scope.clone(),
            resource: token.resource.clone(),
            family: token.family.clone(),
            expires_unix: unix_now()
                + token
                    .expires
                    .saturating_duration_since(Instant::now())
                    .as_secs(),
        }
    }
    fn into_token(self) -> Option<Token> {
        let remaining = self.expires_unix.checked_sub(unix_now())?;
        if remaining == 0 || remaining > REFRESH_SECONDS {
            return None;
        }
        Some(Token {
            client_id: self.client_id,
            scope: self.scope,
            resource: self.resource,
            family: self.family,
            expires: Instant::now() + Duration::from_secs(remaining),
        })
    }
}

#[derive(Clone, Copy)]
pub struct Access {
    pub write: bool,
}

impl McpAuth {
    pub fn new(
        enabled: bool,
        password: Option<Arc<str>>,
        token: Option<Arc<str>>,
        public_url: Option<&str>,
        redirect_allowlist: &[String],
        client_path: PathBuf,
    ) -> Result<Self> {
        if token.as_ref().is_some_and(|value| {
            value.len() < 32
                || value.len() > 1024
                || !value.bytes().all(|byte| byte.is_ascii_graphic())
        }) {
            bail!("config.toml: mcp.token must be 32–1024 printable ASCII characters");
        }
        let base = public_url.map(|value| -> Result<Url> {
            let mut url = Url::parse(value).context("config.toml: mcp.public_url is not a valid URL")?;
            let loopback = matches!(url.host_str(),Some("localhost"|"127.0.0.1"|"[::1]"));
            if !(url.scheme() == "https" || (url.scheme() == "http" && loopback)) || url.host_str().is_none() || !url.username().is_empty() || url.password().is_some() || url.query().is_some() || url.fragment().is_some() || url.path() != "/" {
                bail!("config.toml: mcp.public_url must be an HTTPS origin without credentials, query or path (loopback HTTP is allowed for development)");
            }
            url.set_path("");
            Ok(url)
        }).transpose()?;
        if base.is_some() && password.is_none() {
            bail!("config.toml: OAuth requires a nonempty password");
        }
        if base.is_some()
            && password
                .as_ref()
                .is_some_and(|value| value.is_empty() || value.len() > 1024)
        {
            bail!("config.toml: OAuth requires a nonempty password of at most 1024 bytes");
        }
        if enabled && token.is_none() && base.is_none() {
            bail!("config.toml: enable MCP with mcp.token, or mcp.public_url and password");
        }
        let mut redirects =
            vec!["https://chatgpt.com/connector_platform_oauth_redirect".to_string()];
        for value in redirect_allowlist {
            validate_redirect(value)?;
            if !redirects.contains(value) {
                redirects.push(value.clone());
            }
        }
        let mut store = Store::default();
        if base.is_some() && client_path.exists() {
            let metadata = std::fs::symlink_metadata(&client_path)?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                bail!("OAuth credential store must be a regular file");
            }
            if metadata.len() > 1024 * 1024 {
                bail!("OAuth client store is too large");
            }
            let saved: PersistedStore = serde_json::from_slice(&std::fs::read(&client_path)?)
                .context("Invalid OAuth credential store")?;
            if saved.version != 1
                || saved.access.len() > MAX_TOKENS
                || saved.refresh.len() > MAX_TOKENS
                || saved.used_refresh.len() > MAX_TOKENS
            {
                bail!("Invalid OAuth credential store limits/version");
            }
            store.clients = saved.clients;
            store.access = saved
                .access
                .into_iter()
                .filter_map(|(key, value)| value.into_token().map(|value| (key, value)))
                .collect();
            store.refresh = saved
                .refresh
                .into_iter()
                .filter_map(|(key, value)| value.into_token().map(|value| (key, value)))
                .collect();
            store.used_refresh = saved
                .used_refresh
                .into_iter()
                .filter_map(|(key, (family, client, expiry))| {
                    expiry
                        .checked_sub(unix_now())
                        .filter(|remaining| *remaining > 0 && *remaining <= REFRESH_SECONDS)
                        .map(|remaining| {
                            (
                                key,
                                (
                                    family,
                                    client,
                                    Instant::now() + Duration::from_secs(remaining),
                                ),
                            )
                        })
                })
                .collect();
            if store.clients.len() > MAX_CLIENTS {
                bail!("OAuth client store contains too many registrations");
            }
            for client in store.clients.values() {
                if client.name.len() > 120
                    || client.redirect_uris.len() > 8
                    || !matches!(
                        client.auth_method.as_str(),
                        "none" | "client_secret_post" | "client_secret_basic"
                    )
                {
                    bail!("Invalid stored OAuth client");
                }
                for redirect in &client.redirect_uris {
                    validate_redirect(redirect)?;
                }
            }
        }
        Ok(Self {
            inner: Arc::new(Inner {
                enabled,
                password,
                token,
                base,
                redirects,
                client_path,
                store: Mutex::new(store),
            }),
        })
    }

    pub fn enabled(&self) -> bool {
        self.inner.enabled
    }
    pub fn oauth_enabled(&self) -> bool {
        self.inner.enabled && self.inner.base.is_some()
    }
    pub fn endpoint(&self) -> Option<String> {
        self.inner
            .base
            .as_ref()
            .map(|base| format!("{}mcp", base.as_str()))
    }
    pub fn origin_allowed(&self, headers: &HeaderMap) -> bool {
        let Some(origin) = headers.get(header::ORIGIN) else {
            return true;
        };
        let Some(base) = &self.inner.base else {
            return false;
        };
        origin
            .to_str()
            .ok()
            .is_some_and(|origin| origin == base.origin().ascii_serialization())
    }

    // Keep the Axum rejection response intact so callers preserve OAuth challenge headers.
    #[allow(clippy::result_large_err)]
    pub fn authorize(&self, headers: &HeaderMap) -> std::result::Result<Access, Response> {
        if !self.enabled() {
            return Err((StatusCode::NOT_FOUND, "MCP is disabled").into_response());
        }
        if !self.origin_allowed(headers) {
            return Err((StatusCode::FORBIDDEN, "Invalid Origin").into_response());
        }
        let bearer = headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "));
        if let (Some(bearer), Some(expected)) = (bearer, &self.inner.token)
            && constant_time_equal(bearer.as_bytes(), expected.as_bytes())
        {
            return Ok(Access { write: true });
        }
        if let Some(bearer) = bearer.filter(|value| value.len() <= 1024) {
            let mut store = self
                .inner
                .store
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            store.prune();
            if let Some(token) = store.access.get(&hash(bearer))
                && Some(&token.resource) == self.endpoint().as_ref()
                && has_scope(&token.scope, READ)
            {
                return Ok(Access {
                    write: has_scope(&token.scope, WRITE),
                });
            }
        }
        Err(self.unauthorized())
    }

    pub fn unauthorized(&self) -> Response {
        let mut response = oauth_error(
            StatusCode::UNAUTHORIZED,
            "invalid_token",
            "MCP authorization required",
        );
        let challenge = self.inner.base.as_ref().map_or_else(|| "Bearer realm=\"Picsoc MCP\"".to_string(), |base| format!("Bearer resource_metadata=\"{}.well-known/oauth-protected-resource/mcp\", scope=\"{READ} {WRITE}\"", base.as_str()));
        if let Ok(value) = challenge.parse() {
            response
                .headers_mut()
                .insert(header::WWW_AUTHENTICATE, value);
        }
        response
    }

    pub fn router(&self) -> Router {
        Router::new()
            .route(
                "/.well-known/oauth-protected-resource",
                get(resource_metadata),
            )
            .route(
                "/.well-known/oauth-protected-resource/mcp",
                get(resource_metadata),
            )
            .route(
                "/.well-known/oauth-authorization-server",
                get(server_metadata),
            )
            .route("/oauth/register", post(register))
            .route(
                "/oauth/authorize",
                get(authorize_page).post(authorize_submit),
            )
            .route("/oauth/token", post(token))
            .route("/oauth/revoke", post(revoke))
            .layer(DefaultBodyLimit::max(16 * 1024))
            .with_state(self.clone())
    }

    fn persist_store(&self, store: &Store) -> Result<()> {
        let saved = PersistedStore {
            version: 1,
            clients: store.clients.clone(),
            access: store
                .access
                .iter()
                .map(|(key, value)| (key.clone(), PersistedToken::from_token(value)))
                .collect(),
            refresh: store
                .refresh
                .iter()
                .map(|(key, value)| (key.clone(), PersistedToken::from_token(value)))
                .collect(),
            used_refresh: store
                .used_refresh
                .iter()
                .map(|(key, (family, client, expires))| {
                    (
                        key.clone(),
                        (
                            family.clone(),
                            client.clone(),
                            unix_now()
                                + expires.saturating_duration_since(Instant::now()).as_secs(),
                        ),
                    )
                })
                .collect(),
        };
        let bytes = serde_json::to_vec(&saved)?;
        let parent = self
            .inner
            .client_path
            .parent()
            .context("OAuth client store has no parent directory")?;
        std::fs::create_dir_all(parent)?;
        let temp = tempfile_path(&self.inner.client_path)?;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let result = (|| -> Result<()> {
            use std::io::Write;
            let mut file = options.open(&temp)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            std::fs::rename(&temp, &self.inner.client_path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(temp);
        }
        result
    }
}

impl Store {
    fn prune(&mut self) {
        let now = Instant::now();
        self.pending.retain(|_, value| value.expires > now);
        self.codes.retain(|_, value| value.expires > now);
        self.access.retain(|_, value| value.expires > now);
        self.refresh.retain(|_, value| value.expires > now);
        self.used_refresh
            .retain(|_, (_, _, expires)| *expires > now);
        self.attempts
            .retain(|value| now.duration_since(*value) < Duration::from_secs(60));
        self.registrations
            .retain(|value| now.duration_since(*value) < Duration::from_secs(60));
    }

    fn invalidate_family(&mut self, family: &str) {
        self.access.retain(|_, token| token.family != family);
        self.refresh.retain(|_, token| token.family != family);
    }
}

async fn resource_metadata(State(auth): State<McpAuth>) -> Response {
    if !auth.oauth_enabled() {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(base) = &auth.inner.base else {
        return StatusCode::NOT_FOUND.into_response();
    };
    no_store(Json(json!({"resource":auth.endpoint(),"authorization_servers":[base.as_str().trim_end_matches('/')],"scopes_supported":[READ,WRITE],"bearer_methods_supported":["header"],"resource_name":"Picsoc material library and PNG designs"})).into_response())
}

async fn server_metadata(State(auth): State<McpAuth>) -> Response {
    if !auth.oauth_enabled() {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(base) = &auth.inner.base else {
        return StatusCode::NOT_FOUND.into_response();
    };
    no_store(Json(json!({"issuer":base.as_str().trim_end_matches('/'),"authorization_endpoint":format!("{base}oauth/authorize"),"token_endpoint":format!("{base}oauth/token"),"registration_endpoint":format!("{base}oauth/register"),"revocation_endpoint":format!("{base}oauth/revoke"),"response_types_supported":["code"],"grant_types_supported":["authorization_code","refresh_token"],"token_endpoint_auth_methods_supported":["none","client_secret_basic","client_secret_post"],"code_challenge_methods_supported":["S256"],"authorization_response_iss_parameter_supported":true,"scopes_supported":[READ,WRITE]})).into_response())
}

#[derive(Deserialize)]
struct Registration {
    redirect_uris: Vec<String>,
    #[serde(default)]
    client_name: Option<String>,
    #[serde(default)]
    token_endpoint_auth_method: Option<String>,
    #[serde(default)]
    grant_types: Option<Vec<String>>,
    #[serde(default)]
    response_types: Option<Vec<String>>,
}

async fn register(
    State(auth): State<McpAuth>,
    headers: HeaderMap,
    Json(input): Json<Registration>,
) -> Response {
    if !auth.oauth_enabled() {
        return StatusCode::NOT_FOUND.into_response();
    }
    if !auth.origin_allowed(&headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let method = input
        .token_endpoint_auth_method
        .as_deref()
        .unwrap_or("none");
    if input.redirect_uris.is_empty()
        || input.redirect_uris.len() > 8
        || input
            .redirect_uris
            .iter()
            .any(|uri| validate_redirect(uri).is_err() || !auth.inner.redirects.contains(uri))
    {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_redirect_uri",
            "Use a configured, exact redirect URI",
        );
    }
    if !matches!(
        method,
        "none" | "client_secret_basic" | "client_secret_post"
    ) || input.grant_types.as_ref().is_some_and(|values| {
        values
            .iter()
            .any(|value| !matches!(value.as_str(), "authorization_code" | "refresh_token"))
    }) || input
        .response_types
        .as_ref()
        .is_some_and(|values| values.as_slice() != ["code"])
    {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_client_metadata",
            "Unsupported OAuth client metadata",
        );
    }
    let name = input.client_name.unwrap_or_else(|| "MCP client".into());
    if name.len() > 120 || name.chars().any(char::is_control) {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_client_metadata",
            "Invalid client name",
        );
    }
    let Ok(id) = random_token() else {
        return internal_error();
    };
    let secret = if method == "none" {
        None
    } else {
        match random_token() {
            Ok(value) => Some(value),
            Err(_) => return internal_error(),
        }
    };
    let mut store = auth
        .inner
        .store
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    store.prune();
    if store.registrations.len() >= 5 {
        return oauth_error(
            StatusCode::TOO_MANY_REQUESTS,
            "temporarily_unavailable",
            "Please wait before registering another OAuth client",
        );
    }
    store.registrations.push(Instant::now());
    if store.clients.len() >= MAX_CLIENTS {
        return oauth_error(
            StatusCode::TOO_MANY_REQUESTS,
            "temporarily_unavailable",
            "OAuth client registration limit reached",
        );
    }
    store.clients.insert(
        id.clone(),
        Client {
            name: name.clone(),
            redirect_uris: input.redirect_uris.clone(),
            auth_method: method.into(),
            secret_hash: secret.as_deref().map(hash),
        },
    );
    if let Err(error) = auth.persist_store(&store) {
        store.clients.remove(&id);
        tracing::error!(%error,"Unable to save OAuth client registration");
        return internal_error();
    }
    let mut result = json!({"client_id":id,"client_name":name,"redirect_uris":input.redirect_uris,"token_endpoint_auth_method":method,"grant_types":["authorization_code","refresh_token"],"response_types":["code"]});
    if let Some(secret) = secret {
        result["client_secret"] = Value::String(secret);
        result["client_secret_expires_at"] = json!(0);
    }
    no_store((StatusCode::CREATED, Json(result)).into_response())
}

#[derive(Deserialize)]
struct Authorization {
    response_type: String,
    client_id: String,
    redirect_uri: String,
    code_challenge: String,
    code_challenge_method: String,
    resource: String,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    state: Option<String>,
}

async fn authorize_page(
    State(auth): State<McpAuth>,
    Query(input): Query<Authorization>,
) -> Response {
    if !auth.oauth_enabled() {
        return StatusCode::NOT_FOUND.into_response();
    }
    let mut store = auth
        .inner
        .store
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    store.prune();
    let Some(client) = store.clients.get(&input.client_id) else {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_client",
            "Unknown OAuth client",
        );
    };
    if !client.redirect_uris.contains(&input.redirect_uri)
        || !auth.inner.redirects.contains(&input.redirect_uri)
    {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "Unregistered redirect URI",
        );
    }
    let name = client.name.clone();
    // Do not echo unbounded state into an HTTP redirect header.
    if input.state.as_ref().is_some_and(|value| value.len() > 1024) {
        return authorization_error(
            &auth,
            &input.redirect_uri,
            None,
            "invalid_request",
            "Authorization state is too long",
        );
    }
    if input.response_type != "code"
        || input.code_challenge_method != "S256"
        || input.code_challenge.len() != 43
        || !input
            .code_challenge
            .bytes()
            .all(|value| value.is_ascii_alphanumeric() || matches!(value, b'-' | b'_'))
        || Some(&input.resource) != auth.endpoint().as_ref()
    {
        return authorization_error(
            &auth,
            &input.redirect_uri,
            input.state.as_deref(),
            "invalid_request",
            "A valid S256 PKCE challenge and exact MCP resource are required",
        );
    }
    let Some(scope) = normalize_scope(input.scope.as_deref().unwrap_or("picsoc:read picsoc:write"))
    else {
        return authorization_error(
            &auth,
            &input.redirect_uri,
            input.state.as_deref(),
            "invalid_scope",
            "Unknown scope",
        );
    };
    if store.pending.len() >= MAX_PENDING {
        return authorization_error(
            &auth,
            &input.redirect_uri,
            input.state.as_deref(),
            "temporarily_unavailable",
            "Too many authorization requests",
        );
    }
    let Ok(request_id) = random_token() else {
        return authorization_error(
            &auth,
            &input.redirect_uri,
            input.state.as_deref(),
            "server_error",
            "Unable to start authorization",
        );
    };
    let expires = Instant::now() + Duration::from_secs(300);
    store.pending.insert(
        hash(&request_id),
        Pending {
            grant: Grant {
                client_id: input.client_id,
                redirect_uri: input.redirect_uri,
                challenge: input.code_challenge,
                scope: scope.clone(),
                resource: input.resource,
                expires,
            },
            state: input.state,
            expires,
            failures: 0,
        },
    );
    authorization_form(&request_id, &name, &scope, None)
}

/// Only called after the exact client callback has been validated, or from a
/// pending request created by that validation. Login failures stay in the local
/// form; terminal authorization responses return to the client with RFC 9207 iss.
fn authorization_error(
    auth: &McpAuth,
    redirect_uri: &str,
    state: Option<&str>,
    error: &str,
    description: &str,
) -> Response {
    if !auth.inner.redirects.iter().any(|uri| uri == redirect_uri)
        || validate_redirect(redirect_uri).is_err()
    {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "Invalid callback",
        );
    }
    let Ok(mut redirect) = Url::parse(redirect_uri) else {
        return internal_error();
    };
    let Some(base) = &auth.inner.base else {
        return internal_error();
    };
    redirect
        .query_pairs_mut()
        .append_pair("error", error)
        .append_pair("error_description", description)
        .append_pair("iss", base.as_str().trim_end_matches('/'));
    if let Some(state) = state {
        redirect.query_pairs_mut().append_pair("state", state);
    }
    let Ok(location) = redirect.as_str().parse() else {
        return internal_error();
    };
    let mut response = StatusCode::SEE_OTHER.into_response();
    response.headers_mut().insert(header::LOCATION, location);
    no_store(response)
}

fn authorization_form(
    request_id: &str,
    name: &str,
    scope: &str,
    message: Option<&str>,
) -> Response {
    let permission = if has_scope(scope, WRITE) {
        "搜索和预览已导入素材，创建 PNG 设计，并保存和修改布局。"
    } else {
        "搜索和预览已导入素材，以及查看已保存设计。"
    };
    let warning = message.map_or_else(String::new, |message| {
        format!(
            "<p role=\"alert\" style=\"color:#c0392b\">{}</p>",
            html_escape(message)
        )
    });
    let request_id = html_escape(request_id);
    let body = format!(
        r#"<!doctype html><html lang="zh-CN"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>连接 Picsoc</title><style>body{{margin:0;min-height:100vh;display:grid;place-items:center;background:#f5f5f7;color:#1d1d1f;font:16px system-ui,sans-serif}}main{{box-sizing:border-box;width:min(440px,92vw);background:white;padding:36px;border-radius:24px;box-shadow:0 20px 60px #0001}}h1{{font-size:26px;margin:0 0 20px}}p{{line-height:1.7;color:#666}}label{{display:block;font-size:14px}}input,button{{box-sizing:border-box;width:100%;font:inherit;padding:12px 14px;border-radius:10px;margin-top:10px}}input{{border:1px solid #ddd}}button{{border:0;background:#007aff;color:white;cursor:pointer}}button.secondary{{background:#f0f0f3;color:#555}}small{{display:block;margin-top:20px;color:#777}}</style></head><body><main><h1>连接 Picsoc</h1><p><strong>{}</strong> 请求访问你的素材库。</p><p>{permission}</p>{warning}<form action="/oauth/authorize" method="post"><input type="hidden" name="request_id" value="{request_id}"><label for="password">输入 Picsoc 登录密码</label><input id="password" type="password" name="password" autocomplete="current-password" required maxlength="1024"><button type="submit" name="decision" value="allow">授权连接</button><button class="secondary" type="submit" name="decision" value="deny" formnovalidate>拒绝连接</button></form><small>原始文件保持只读。连接授权可在 ChatGPT 中断开。</small></main></body></html>"#,
        html_escape(name)
    );
    let status = if message.is_some() {
        StatusCode::UNAUTHORIZED
    } else {
        StatusCode::OK
    };
    let mut response = no_store((status, Html(body)).into_response());
    for (key, value) in [
        (
            "content-security-policy",
            "default-src 'none'; style-src 'unsafe-inline'; form-action 'self'; frame-ancestors 'none'; base-uri 'none'",
        ),
        ("referrer-policy", "no-referrer"),
        ("x-frame-options", "DENY"),
    ] {
        response.headers_mut().insert(key, value.parse().unwrap());
    }
    response
}

#[derive(Deserialize)]
struct Consent {
    request_id: String,
    #[serde(default)]
    password: String,
    #[serde(default)]
    decision: Option<String>,
}

async fn authorize_submit(
    State(auth): State<McpAuth>,
    headers: HeaderMap,
    Form(input): Form<Consent>,
) -> Response {
    if !auth.oauth_enabled() {
        return StatusCode::NOT_FOUND.into_response();
    }
    if headers.get(header::ORIGIN).is_none() || !auth.origin_allowed(&headers) {
        return oauth_error(
            StatusCode::FORBIDDEN,
            "invalid_request",
            "Authorization requires the configured public origin",
        );
    }
    let mut store = auth
        .inner
        .store
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    store.prune();
    let key = hash(&input.request_id);
    let Some(pending) = store.pending.get(&key).cloned() else {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "Authorization request expired; start again",
        );
    };
    if input.decision.as_deref() == Some("deny") {
        store.pending.remove(&key);
        return authorization_error(
            &auth,
            &pending.grant.redirect_uri,
            pending.state.as_deref(),
            "access_denied",
            "The user declined authorization",
        );
    }
    if input
        .decision
        .as_deref()
        .is_some_and(|decision| decision != "allow")
    {
        store.pending.remove(&key);
        return authorization_error(
            &auth,
            &pending.grant.redirect_uri,
            pending.state.as_deref(),
            "invalid_request",
            "Unknown authorization decision",
        );
    }
    if pending.failures >= 5 {
        store.pending.remove(&key);
        return authorization_error(
            &auth,
            &pending.grant.redirect_uri,
            pending.state.as_deref(),
            "access_denied",
            "Restart authorization after too many attempts",
        );
    }
    if store.attempts.len() >= 40 {
        store.pending.remove(&key);
        return authorization_error(
            &auth,
            &pending.grant.redirect_uri,
            pending.state.as_deref(),
            "temporarily_unavailable",
            "Please wait before trying again",
        );
    }
    store.attempts.push(Instant::now());
    if input.password.len() > 1024
        || !auth.inner.password.as_ref().is_some_and(|expected| {
            constant_time_equal(input.password.as_bytes(), expected.as_bytes())
        })
    {
        if pending.failures + 1 >= 5 {
            store.pending.remove(&key);
            return authorization_error(
                &auth,
                &pending.grant.redirect_uri,
                pending.state.as_deref(),
                "access_denied",
                "Restart authorization after too many attempts",
            );
        }
        if let Some(saved) = store.pending.get_mut(&key) {
            saved.failures += 1;
        }
        let name = store
            .clients
            .get(&pending.grant.client_id)
            .map_or("MCP client", |client| client.name.as_str());
        return authorization_form(
            &input.request_id,
            name,
            &pending.grant.scope,
            Some("密码不正确，请重试或拒绝连接。"),
        );
    }
    if store.codes.len() >= MAX_CODES {
        store.pending.remove(&key);
        return authorization_error(
            &auth,
            &pending.grant.redirect_uri,
            pending.state.as_deref(),
            "temporarily_unavailable",
            "Too many pending authorization codes",
        );
    }
    let Ok(code) = random_token() else {
        store.pending.remove(&key);
        return authorization_error(
            &auth,
            &pending.grant.redirect_uri,
            pending.state.as_deref(),
            "server_error",
            "Unable to finish authorization",
        );
    };
    let Some(mut pending) = store.pending.remove(&key) else {
        return internal_error();
    };
    pending.grant.expires = Instant::now() + Duration::from_secs(120);
    let Ok(mut redirect) = Url::parse(&pending.grant.redirect_uri) else {
        return internal_error();
    };
    redirect.query_pairs_mut().append_pair("code", &code);
    if let Some(base) = &auth.inner.base {
        redirect
            .query_pairs_mut()
            .append_pair("iss", base.as_str().trim_end_matches('/'));
    }
    if let Some(state) = pending.state {
        redirect.query_pairs_mut().append_pair("state", &state);
    }
    store.codes.insert(hash(&code), pending.grant);
    let mut response = StatusCode::SEE_OTHER.into_response();
    if let Ok(value) = redirect.as_str().parse() {
        response.headers_mut().insert(header::LOCATION, value);
    } else {
        return internal_error();
    }
    no_store(response)
}

#[derive(Deserialize)]
struct TokenRequest {
    grant_type: String,
    #[serde(default)]
    client_id: Option<String>,
    #[serde(default)]
    client_secret: Option<String>,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    redirect_uri: Option<String>,
    #[serde(default)]
    code_verifier: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
    resource: String,
    #[serde(default)]
    scope: Option<String>,
}

async fn token(
    State(auth): State<McpAuth>,
    headers: HeaderMap,
    Form(input): Form<TokenRequest>,
) -> Response {
    if !auth.oauth_enabled() {
        return StatusCode::NOT_FOUND.into_response();
    }
    if !auth.origin_allowed(&headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let mut store = auth
        .inner
        .store
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    store.prune();
    let original_access = store.access.clone();
    let original_refresh = store.refresh.clone();
    let original_used_refresh = store.used_refresh.clone();
    let Some(client_id) = authenticate_client(
        &store,
        &headers,
        input.client_id.as_deref(),
        input.client_secret.as_deref(),
    ) else {
        return oauth_error(
            StatusCode::UNAUTHORIZED,
            "invalid_client",
            "Invalid client credentials",
        );
    };
    if Some(&input.resource) != auth.endpoint().as_ref() {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_target",
            "Token resource must match the MCP endpoint",
        );
    }
    if store.access.len() >= MAX_TOKENS
        || store.refresh.len() >= MAX_TOKENS
        || store.used_refresh.len() >= MAX_TOKENS
    {
        return oauth_error(
            StatusCode::TOO_MANY_REQUESTS,
            "temporarily_unavailable",
            "Active OAuth token limit reached",
        );
    }
    let (scope, family, refresh_expiry) = match input.grant_type.as_str() {
        "authorization_code" => {
            let Some(code) = input.code.filter(|value| value.len() <= 1024) else {
                return invalid_grant();
            };
            let Some(grant) = store.codes.remove(&hash(&code)) else {
                return invalid_grant();
            };
            let Some(verifier) = input.code_verifier.filter(|value| {
                value.len() >= 43
                    && value.len() <= 128
                    && value.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~')
                    })
            }) else {
                return invalid_grant();
            };
            let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(Sha256::digest(verifier.as_bytes()));
            if grant.client_id != client_id
                || input.redirect_uri.as_deref() != Some(&grant.redirect_uri)
                || grant.resource != input.resource
                || !constant_time_equal(challenge.as_bytes(), grant.challenge.as_bytes())
            {
                return invalid_grant();
            }
            let Ok(family) = random_token() else {
                return internal_error();
            };
            (
                grant.scope,
                family,
                Instant::now() + Duration::from_secs(REFRESH_SECONDS),
            )
        }
        "refresh_token" => {
            let Some(refresh) = input.refresh_token.filter(|value| value.len() <= 1024) else {
                return invalid_grant();
            };
            let key = hash(&refresh);
            if let Some((family, owner, _)) = store.used_refresh.get(&key).cloned() {
                if owner == client_id {
                    store.invalidate_family(&family);
                    if let Err(error) = auth.persist_store(&store) {
                        tracing::error!(%error,"Unable to persist replay revocation");
                        return internal_error();
                    }
                }
                return invalid_grant();
            }
            let Some(previous) = store.refresh.get(&key).cloned() else {
                return invalid_grant();
            };
            if previous.client_id != client_id || previous.resource != input.resource {
                return invalid_grant();
            }
            let scope = match input.scope.as_deref() {
                None => previous.scope.clone(),
                Some(value) => match normalize_scope(value) {
                    Some(scope)
                        if scope
                            .split_whitespace()
                            .all(|value| has_scope(&previous.scope, value)) =>
                    {
                        scope
                    }
                    _ => {
                        return oauth_error(
                            StatusCode::BAD_REQUEST,
                            "invalid_scope",
                            "Refresh cannot increase scopes",
                        );
                    }
                },
            };
            store.refresh.remove(&key);
            store.used_refresh.insert(
                key,
                (
                    previous.family.clone(),
                    previous.client_id.clone(),
                    previous.expires,
                ),
            );
            (scope, previous.family, previous.expires)
        }
        _ => {
            return oauth_error(
                StatusCode::BAD_REQUEST,
                "unsupported_grant_type",
                "Only authorization_code and refresh_token are supported",
            );
        }
    };
    let (Ok(access), Ok(refresh)) = (random_token(), random_token()) else {
        return internal_error();
    };
    let resource = input.resource;
    let now = Instant::now();
    store.access.insert(
        hash(&access),
        Token {
            client_id: client_id.clone(),
            scope: scope.clone(),
            resource: resource.clone(),
            family: family.clone(),
            expires: now + Duration::from_secs(ACCESS_SECONDS),
        },
    );
    store.refresh.insert(
        hash(&refresh),
        Token {
            client_id,
            scope: scope.clone(),
            resource,
            family,
            expires: refresh_expiry,
        },
    );
    if let Err(error) = auth.persist_store(&store) {
        store.access = original_access;
        store.refresh = original_refresh;
        store.used_refresh = original_used_refresh;
        tracing::error!(%error,"Unable to persist OAuth token rotation");
        return internal_error();
    }
    no_store(Json(json!({"access_token":access,"token_type":"Bearer","expires_in":ACCESS_SECONDS,"refresh_token":refresh,"scope":scope})).into_response())
}

#[derive(Deserialize)]
struct RevokeRequest {
    token: String,
    #[serde(default)]
    client_id: Option<String>,
    #[serde(default)]
    client_secret: Option<String>,
}

async fn revoke(
    State(auth): State<McpAuth>,
    headers: HeaderMap,
    Form(input): Form<RevokeRequest>,
) -> Response {
    if !auth.oauth_enabled() {
        return StatusCode::NOT_FOUND.into_response();
    }
    if !auth.origin_allowed(&headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let mut store = auth
        .inner
        .store
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    store.prune();
    let Some(client_id) = authenticate_client(
        &store,
        &headers,
        input.client_id.as_deref(),
        input.client_secret.as_deref(),
    ) else {
        return oauth_error(
            StatusCode::UNAUTHORIZED,
            "invalid_client",
            "Invalid client credentials",
        );
    };
    let key = hash(&input.token);
    let candidate = store
        .access
        .get(&key)
        .or_else(|| store.refresh.get(&key))
        .cloned();
    if let Some(token) = candidate
        && token.client_id == client_id
    {
        store.invalidate_family(&token.family);
        if let Err(error) = auth.persist_store(&store) {
            tracing::error!(%error,"Unable to persist token revocation");
            return internal_error();
        }
    }
    no_store(StatusCode::OK.into_response())
}

fn authenticate_client(
    store: &Store,
    headers: &HeaderMap,
    id: Option<&str>,
    secret: Option<&str>,
) -> Option<String> {
    let basic = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Basic "))
        .and_then(|value| base64::engine::general_purpose::STANDARD.decode(value).ok())
        .and_then(|value| String::from_utf8(value).ok())
        .and_then(|value| {
            value
                .split_once(':')
                .map(|(id, secret)| (id.to_owned(), secret.to_owned()))
        });
    if basic.is_some() && secret.is_some() {
        return None;
    }
    let candidate = basic.as_ref().map(|(id, _)| id.as_str()).or(id)?;
    if basic.is_some() && id.is_some_and(|id| id != candidate) {
        return None;
    }
    let client = store.clients.get(candidate)?;
    let accepted = match client.auth_method.as_str() {
        "none" => basic.is_none() && secret.is_none(),
        "client_secret_basic" => basic.as_ref().is_some_and(|(_, secret)| {
            client.secret_hash.as_ref().is_some_and(|expected| {
                constant_time_equal(hash(secret).as_bytes(), expected.as_bytes())
            })
        }),
        "client_secret_post" => {
            basic.is_none()
                && secret.is_some_and(|secret| {
                    client.secret_hash.as_ref().is_some_and(|expected| {
                        constant_time_equal(hash(secret).as_bytes(), expected.as_bytes())
                    })
                })
        }
        _ => false,
    };
    accepted.then(|| candidate.to_owned())
}

fn normalize_scope(value: &str) -> Option<String> {
    let values: Vec<_> = value.split_whitespace().collect();
    if !values.contains(&READ) || values.iter().any(|value| !matches!(*value, READ | WRITE)) {
        return None;
    }
    Some(if values.contains(&WRITE) {
        format!("{READ} {WRITE}")
    } else {
        READ.into()
    })
}

fn has_scope(scope: &str, value: &str) -> bool {
    scope.split_whitespace().any(|candidate| candidate == value)
}
fn hash(value: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(value.as_bytes()))
}
fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn random_token() -> Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).context("Unable to generate secure credentials")?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
}
fn tempfile_path(path: &std::path::Path) -> Result<PathBuf> {
    Ok(path.with_extension(format!("{}.tmp", random_token()?)))
}
fn constant_time_equal(actual: &[u8], expected: &[u8]) -> bool {
    let mut diff = actual.len() ^ expected.len();
    for (i, value) in expected.iter().enumerate() {
        diff |= usize::from(actual.get(i).copied().unwrap_or(0) ^ value);
    }
    diff == 0
}
fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
fn validate_redirect(value: &str) -> Result<()> {
    if value.len() > 2048 {
        bail!("OAuth redirect URI is too long");
    }
    let url = Url::parse(value)?;
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.host_str().is_none()
        || !(url.scheme() == "https" || (url.scheme() == "http" && loopback))
    {
        bail!("OAuth redirects require HTTPS (or loopback HTTP) without credentials or fragment");
    }
    Ok(())
}
fn no_store(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    response
        .headers_mut()
        .insert(header::PRAGMA, "no-cache".parse().unwrap());
    response
}
fn oauth_error(status: StatusCode, error: &str, description: &str) -> Response {
    no_store(
        (
            status,
            Json(json!({"error":error,"error_description":description})),
        )
            .into_response(),
    )
}
fn invalid_grant() -> Response {
    oauth_error(
        StatusCode::BAD_REQUEST,
        "invalid_grant",
        "Invalid, expired or already used grant",
    )
}
fn internal_error() -> Response {
    oauth_error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "server_error",
        "Authorization service unavailable",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };
    use tower::ServiceExt;

    const CALLBACK: &str = "https://chatgpt.com/connector_platform_oauth_redirect";
    const PUBLIC: &str = "https://picsoc.example";
    const RESOURCE: &str = "https://picsoc.example/mcp";

    fn fixture(path: PathBuf) -> McpAuth {
        McpAuth::new(
            true,
            Some(Arc::from("test-secret")),
            None,
            Some(PUBLIC),
            &[],
            path,
        )
        .unwrap()
    }

    async fn request(
        auth: &McpAuth,
        method: &str,
        uri: &str,
        body: String,
        content_type: &str,
        origin: Option<&str>,
    ) -> Response {
        let mut request = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::CONTENT_TYPE, content_type);
        if let Some(origin) = origin {
            request = request.header(header::ORIGIN, origin);
        }
        auth.router()
            .oneshot(request.body(Body::from(body)).unwrap())
            .await
            .unwrap()
    }

    async fn json_body(response: Response) -> Value {
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap()).unwrap()
    }
    fn form(values: &[(&str, &str)]) -> String {
        url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(values.iter().copied())
            .finish()
    }

    async fn register_client(auth: &McpAuth, method: &str) -> Value {
        let response=request(auth,"POST","/oauth/register",json!({"redirect_uris":[CALLBACK],"client_name":"ChatGPT","token_endpoint_auth_method":method}).to_string(),"application/json",None).await;
        assert_eq!(response.status(), StatusCode::CREATED);
        json_body(response).await
    }

    async fn authorize_code(
        auth: &McpAuth,
        client_id: &str,
        verifier: &str,
        scope: &str,
    ) -> String {
        let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(Sha256::digest(verifier.as_bytes()));
        let query = form(&[
            ("response_type", "code"),
            ("client_id", client_id),
            ("redirect_uri", CALLBACK),
            ("code_challenge", &challenge),
            ("code_challenge_method", "S256"),
            ("resource", RESOURCE),
            ("scope", scope),
            ("state", "state-with-空格"),
        ]);
        let response = request(
            auth,
            "GET",
            &format!("/oauth/authorize?{query}"),
            String::new(),
            "application/json",
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        let body = String::from_utf8(
            to_bytes(response.into_body(), 65536)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap();
        let request_id = body
            .split("name=\"request_id\" value=\"")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap();
        let response = request(
            auth,
            "POST",
            "/oauth/authorize",
            form(&[("request_id", request_id), ("password", "test-secret")]),
            "application/x-www-form-urlencoded",
            Some(PUBLIC),
        )
        .await;
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        let redirect = Url::parse(response.headers()[header::LOCATION].to_str().unwrap()).unwrap();
        let values: HashMap<_, _> = redirect.query_pairs().into_owned().collect();
        assert_eq!(values.get("iss").unwrap(), PUBLIC);
        assert_eq!(values.get("state").unwrap(), "state-with-空格");
        values.get("code").unwrap().clone()
    }

    async fn exchange(
        auth: &McpAuth,
        client: &Value,
        code: &str,
        verifier: &str,
        resource: &str,
    ) -> Response {
        let mut values = vec![
            ("grant_type", "authorization_code"),
            ("client_id", client["client_id"].as_str().unwrap()),
            ("code", code),
            ("redirect_uri", CALLBACK),
            ("code_verifier", verifier),
            ("resource", resource),
        ];
        if let Some(secret) = client["client_secret"].as_str() {
            values.push(("client_secret", secret));
        }
        request(
            auth,
            "POST",
            "/oauth/token",
            form(&values),
            "application/x-www-form-urlencoded",
            None,
        )
        .await
    }

    fn bearer(token: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {token}").parse().unwrap(),
        );
        headers
    }

    #[test]
    fn disabled_and_local_credentials_do_not_reuse_browser_password() {
        let dir = tempfile::tempdir().unwrap();
        let disabled = McpAuth::new(
            false,
            Some(Arc::from("password")),
            None,
            None,
            &[],
            dir.path().join("oauth.json"),
        )
        .unwrap();
        assert!(!disabled.enabled());
        assert_eq!(
            disabled
                .authorize(&HeaderMap::new())
                .err()
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );
        assert!(
            McpAuth::new(
                true,
                Some(Arc::from("password")),
                None,
                None,
                &[],
                dir.path().join("oauth.json")
            )
            .is_err()
        );
        let token = "a-secure-token-with-at-least-32-characters";
        let auth = McpAuth::new(
            true,
            None,
            Some(Arc::from(token)),
            None,
            &[],
            dir.path().join("oauth.json"),
        )
        .unwrap();
        assert!(auth.authorize(&bearer(token)).is_ok());
        assert!(auth.authorize(&bearer("password")).is_err());
        let mut headers = bearer(token);
        headers.insert(header::ORIGIN, "https://malicious.example".parse().unwrap());
        assert_eq!(
            auth.authorize(&headers).err().unwrap().status(),
            StatusCode::FORBIDDEN
        );
        for public in [
            "http://192.168.2.101:3210",
            "https://user:pass@example.com",
            "https://example.com/path",
            "https://example.com?query",
        ] {
            assert!(
                McpAuth::new(
                    true,
                    Some(Arc::from("password")),
                    None,
                    Some(public),
                    &[],
                    dir.path().join("oauth.json")
                )
                .is_err()
            );
        }
    }

    #[tokio::test]
    async fn discover_pkce_single_use_and_persistent_hashed_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("oauth.json");
        let auth = fixture(path.clone());
        let discovery = json_body(
            request(
                &auth,
                "GET",
                "/.well-known/oauth-authorization-server",
                String::new(),
                "application/json",
                None,
            )
            .await,
        )
        .await;
        assert_eq!(discovery["issuer"], PUBLIC);
        assert_eq!(
            discovery["authorization_response_iss_parameter_supported"],
            true
        );
        let client = register_client(&auth, "client_secret_post").await;
        let verifier = "abcdefghijklmnopqrstuvwxyz0123456789-._~abcdefghijkl";
        let code = authorize_code(
            &auth,
            client["client_id"].as_str().unwrap(),
            verifier,
            "picsoc:read",
        )
        .await;
        assert_eq!(
            exchange(&auth, &client, &code, verifier, "https://other.example/mcp")
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
        let response = exchange(&auth, &client, &code, verifier, RESOURCE).await;
        assert_eq!(response.status(), StatusCode::OK);
        let tokens = json_body(response).await;
        let access = tokens["access_token"].as_str().unwrap();
        assert!(!auth.authorize(&bearer(access)).unwrap().write);
        assert_eq!(
            exchange(&auth, &client, &code, verifier, RESOURCE)
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
        let disk = std::fs::read_to_string(&path).unwrap();
        for secret in [
            access,
            tokens["refresh_token"].as_str().unwrap(),
            client["client_secret"].as_str().unwrap(),
            "test-secret",
        ] {
            assert!(!disk.contains(secret));
        }
        let restarted = fixture(path);
        assert!(!restarted.authorize(&bearer(access)).unwrap().write);
        let wrong_code = authorize_code(
            &auth,
            client["client_id"].as_str().unwrap(),
            verifier,
            "picsoc:read",
        )
        .await;
        assert_eq!(
            exchange(
                &auth,
                &client,
                &wrong_code,
                "wrong-but-valid-length-abcdefghijklmnopqrstuvwxyz012345",
                RESOURCE
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            exchange(&auth, &client, &wrong_code, verifier, RESOURCE)
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
    }

    #[tokio::test]
    async fn refresh_rotation_and_replay_revoke_the_entire_family_across_restarts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("oauth.json");
        let auth = fixture(path.clone());
        let client = register_client(&auth, "none").await;
        let id = client["client_id"].as_str().unwrap();
        let verifier = "abcdefghijklmnopqrstuvwxyz0123456789-._~abcdefghijkl";
        let code = authorize_code(&auth, id, verifier, "picsoc:read picsoc:write").await;
        let first = json_body(exchange(&auth, &client, &code, verifier, RESOURCE).await).await;
        let refresh = first["refresh_token"].as_str().unwrap();
        let make = |scope: &str| {
            form(&[
                ("grant_type", "refresh_token"),
                ("client_id", id),
                ("refresh_token", refresh),
                ("resource", RESOURCE),
                ("scope", scope),
            ])
        };
        let second_response = request(
            &auth,
            "POST",
            "/oauth/token",
            make("picsoc:read"),
            "application/x-www-form-urlencoded",
            None,
        )
        .await;
        assert_eq!(second_response.status(), StatusCode::OK);
        let second = json_body(second_response).await;
        assert_ne!(second["refresh_token"], first["refresh_token"]);
        let restarted = fixture(path.clone());
        assert!(
            restarted
                .authorize(&bearer(second["access_token"].as_str().unwrap()))
                .is_ok()
        );
        let replay = request(
            &restarted,
            "POST",
            "/oauth/token",
            make("picsoc:read"),
            "application/x-www-form-urlencoded",
            None,
        )
        .await;
        assert_eq!(replay.status(), StatusCode::BAD_REQUEST);
        assert!(
            restarted
                .authorize(&bearer(first["access_token"].as_str().unwrap()))
                .is_err()
        );
        assert!(
            restarted
                .authorize(&bearer(second["access_token"].as_str().unwrap()))
                .is_err()
        );
        assert!(
            fixture(path)
                .authorize(&bearer(second["access_token"].as_str().unwrap()))
                .is_err()
        );
    }

    #[tokio::test]
    async fn unregistered_redirects_csrf_and_wrong_password_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let auth = fixture(dir.path().join("oauth.json"));
        let bad = request(
            &auth,
            "POST",
            "/oauth/register",
            json!({"redirect_uris":["https://evil.example/callback"]}).to_string(),
            "application/json",
            None,
        )
        .await;
        assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
        assert!(auth.inner.store.lock().unwrap().clients.is_empty());
        let client = register_client(&auth, "none").await;
        let id = client["client_id"].as_str().unwrap();
        let verifier = "abcdefghijklmnopqrstuvwxyz0123456789-._~abcdefghijkl";
        let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(Sha256::digest(verifier.as_bytes()));
        let query = form(&[
            ("response_type", "code"),
            ("client_id", id),
            ("redirect_uri", CALLBACK),
            ("code_challenge", &challenge),
            ("code_challenge_method", "S256"),
            ("resource", RESOURCE),
        ]);
        let response = request(
            &auth,
            "GET",
            &format!("/oauth/authorize?{query}"),
            String::new(),
            "application/json",
            None,
        )
        .await;
        let html = String::from_utf8(
            to_bytes(response.into_body(), 65536)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap();
        let nonce = html
            .split("name=\"request_id\" value=\"")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap();
        let consent = form(&[("request_id", nonce), ("password", "test-secret")]);
        for origin in [None, Some("https://evil.example")] {
            assert_eq!(
                request(
                    &auth,
                    "POST",
                    "/oauth/authorize",
                    consent.clone(),
                    "application/x-www-form-urlencoded",
                    origin
                )
                .await
                .status(),
                StatusCode::FORBIDDEN
            );
        }
        assert_eq!(
            request(
                &auth,
                "POST",
                "/oauth/authorize",
                form(&[("request_id", nonce), ("password", "wrong")]),
                "application/x-www-form-urlencoded",
                Some(PUBLIC)
            )
            .await
            .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            request(
                &auth,
                "POST",
                "/oauth/authorize",
                consent,
                "application/x-www-form-urlencoded",
                Some(PUBLIC)
            )
            .await
            .status(),
            StatusCode::SEE_OTHER
        );
    }

    fn authorization_query(client_id: &str, state: &str) -> Vec<(String, String)> {
        vec![
            ("response_type".into(), "code".into()),
            ("client_id".into(), client_id.into()),
            ("redirect_uri".into(), CALLBACK.into()),
            ("code_challenge".into(), "a".repeat(43)),
            ("code_challenge_method".into(), "S256".into()),
            ("resource".into(), RESOURCE.into()),
            ("scope".into(), "picsoc:read picsoc:write".into()),
            ("state".into(), state.into()),
        ]
    }

    async fn authorization_get(auth: &McpAuth, values: &[(String, String)]) -> Response {
        let query = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(
                values
                    .iter()
                    .map(|(key, value)| (key.as_str(), value.as_str())),
            )
            .finish();
        request(
            auth,
            "GET",
            &format!("/oauth/authorize?{query}"),
            String::new(),
            "application/json",
            None,
        )
        .await
    }

    fn error_callback(response: &Response, error: &str, state: &str) {
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        let redirect = Url::parse(response.headers()[header::LOCATION].to_str().unwrap()).unwrap();
        assert_eq!(
            redirect.origin().ascii_serialization(),
            "https://chatgpt.com"
        );
        assert_eq!(redirect.path(), "/connector_platform_oauth_redirect");
        let values: HashMap<_, _> = redirect.query_pairs().into_owned().collect();
        assert_eq!(values.get("iss").unwrap(), PUBLIC);
        assert_eq!(values.get("state").unwrap(), state);
        assert_eq!(values.get("error").unwrap(), error);
        assert!(!values.contains_key("code"));
    }

    async fn authorization_nonce(auth: &McpAuth, client_id: &str, state: &str) -> String {
        let response = authorization_get(auth, &authorization_query(client_id, state)).await;
        assert_eq!(response.status(), StatusCode::OK);
        let html = String::from_utf8(
            to_bytes(response.into_body(), 65536)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap();
        assert!(html.contains("value=\"deny\" formnovalidate"));
        html.split("name=\"request_id\" value=\"")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap()
            .to_owned()
    }

    #[tokio::test]
    async fn authorization_errors_echo_issuer_and_state_only_after_callback_validation() {
        let dir = tempfile::tempdir().unwrap();
        let auth = fixture(dir.path().join("oauth.json"));
        let client = register_client(&auth, "none").await;
        let id = client["client_id"].as_str().unwrap();
        let state = "state-中文 & +";
        for (field, value, error) in [
            ("scope", "picsoc:admin", "invalid_scope"),
            ("resource", "https://other.example/mcp", "invalid_request"),
            ("code_challenge_method", "plain", "invalid_request"),
        ] {
            let mut values = authorization_query(id, state);
            values.iter_mut().find(|(key, _)| key == field).unwrap().1 = value.into();
            let response = authorization_get(&auth, &values).await;
            error_callback(&response, error, state);
        }
        for (field, value) in [
            ("client_id", "unknown-client"),
            ("redirect_uri", "https://evil.example/callback"),
        ] {
            let mut values = authorization_query(id, state);
            values.iter_mut().find(|(key, _)| key == field).unwrap().1 = value.into();
            values.iter_mut().find(|(key, _)| key == "scope").unwrap().1 = "picsoc:admin".into();
            let response = authorization_get(&auth, &values).await;
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
            assert!(!response.headers().contains_key(header::LOCATION));
        }
        assert!(auth.inner.store.lock().unwrap().pending.is_empty());
    }

    #[tokio::test]
    async fn incorrect_password_keeps_retry_form_and_denial_ends_the_grant_once() {
        let dir = tempfile::tempdir().unwrap();
        let auth = fixture(dir.path().join("oauth.json"));
        let client = register_client(&auth, "none").await;
        let state = "denial-状态";
        let nonce = authorization_nonce(&auth, client["client_id"].as_str().unwrap(), state).await;
        let retry = request(
            &auth,
            "POST",
            "/oauth/authorize",
            form(&[("request_id", &nonce), ("password", "wrong")]),
            "application/x-www-form-urlencoded",
            Some(PUBLIC),
        )
        .await;
        assert_eq!(retry.status(), StatusCode::UNAUTHORIZED);
        assert!(!retry.headers().contains_key(header::LOCATION));
        assert!(
            retry.headers()[header::CONTENT_TYPE]
                .to_str()
                .unwrap()
                .starts_with("text/html")
        );
        let html =
            String::from_utf8(to_bytes(retry.into_body(), 65536).await.unwrap().to_vec()).unwrap();
        assert!(html.contains("密码不正确"));
        assert!(html.contains(&format!("name=\"request_id\" value=\"{nonce}\"")));
        assert!(!html.contains("value=\"wrong\""));
        let denied = request(
            &auth,
            "POST",
            "/oauth/authorize",
            form(&[("request_id", &nonce), ("decision", "deny")]),
            "application/x-www-form-urlencoded",
            Some(PUBLIC),
        )
        .await;
        error_callback(&denied, "access_denied", state);
        let replay = request(
            &auth,
            "POST",
            "/oauth/authorize",
            form(&[("request_id", &nonce), ("password", "test-secret")]),
            "application/x-www-form-urlencoded",
            Some(PUBLIC),
        )
        .await;
        assert_eq!(replay.status(), StatusCode::BAD_REQUEST);
        assert!(!replay.headers().contains_key(header::LOCATION));
        let store = auth.inner.store.lock().unwrap();
        assert!(store.pending.is_empty());
        assert!(store.codes.is_empty());
    }

    #[tokio::test]
    async fn exhausting_password_retries_returns_an_issuer_bound_authorization_error() {
        let dir = tempfile::tempdir().unwrap();
        let auth = fixture(dir.path().join("oauth.json"));
        let client = register_client(&auth, "none").await;
        let state = "retry-limit";
        let nonce = authorization_nonce(&auth, client["client_id"].as_str().unwrap(), state).await;
        for attempt in 1..=5 {
            let response = request(
                &auth,
                "POST",
                "/oauth/authorize",
                form(&[("request_id", &nonce), ("password", "wrong")]),
                "application/x-www-form-urlencoded",
                Some(PUBLIC),
            )
            .await;
            if attempt < 5 {
                assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
                assert!(!response.headers().contains_key(header::LOCATION));
            } else {
                error_callback(&response, "access_denied", state);
            }
        }
        let store = auth.inner.store.lock().unwrap();
        assert!(store.pending.is_empty());
        assert!(store.codes.is_empty());
    }

    #[tokio::test]
    async fn read_scope_can_search_but_cannot_render_through_mcp() {
        use crate::mcp::{McpState, ToolDispatcher, ToolFuture};
        struct Tools;
        impl ToolDispatcher for Tools {
            fn tools(&self) -> Vec<Value> {
                vec![
                    json!({"name":"search_assets","inputSchema":{"type":"object"},"annotations":{"readOnlyHint":true}}),
                    json!({"name":"render_design","inputSchema":{"type":"object"},"annotations":{"readOnlyHint":false}}),
                ]
            }
            fn call(&self, _name: String, _arguments: Value) -> ToolFuture {
                Box::pin(async { Ok(json!({"content":[{"type":"text","text":"called"}]})) })
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let auth = fixture(dir.path().join("oauth.json"));
        let client = register_client(&auth, "none").await;
        let verifier = "abcdefghijklmnopqrstuvwxyz0123456789-._~abcdefghijkl";
        let code = authorize_code(
            &auth,
            client["client_id"].as_str().unwrap(),
            verifier,
            "picsoc:read",
        )
        .await;
        let tokens = json_body(exchange(&auth, &client, &code, verifier, RESOURCE).await).await;
        let router = crate::mcp::router(McpState {
            auth: Arc::new(auth),
            tools: Arc::new(Tools),
        });
        for (name, status) in [
            ("search_assets", StatusCode::OK),
            ("render_design", StatusCode::FORBIDDEN),
        ] {
            let request=Request::builder().method("POST").uri("/mcp").header(header::AUTHORIZATION,format!("Bearer {}",tokens["access_token"].as_str().unwrap())).header(header::ACCEPT,"application/json, text/event-stream").header(header::CONTENT_TYPE,"application/json").body(Body::from(json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":{}}}).to_string())).unwrap();
            assert_eq!(
                router.clone().oneshot(request).await.unwrap().status(),
                status
            );
        }
    }
}
