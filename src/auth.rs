use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use axum::http::{HeaderMap, header};
use base64::Engine;
use serde::Serialize;

pub const SESSION_COOKIE: &str = "picsoc_session";
pub const CLEAR_SESSION_COOKIE: &str =
    "picsoc_session=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0";
const SESSION_SECONDS: u64 = 24 * 60 * 60;
const MAX_SESSIONS: usize = 512;

#[derive(Clone)]
pub struct Auth {
    password: Option<Arc<str>>,
    // Session tokens never encode the password, and are discarded when the service exits.
    sessions: Arc<Mutex<HashMap<String, Instant>>>,
}

#[derive(Debug, Serialize)]
pub struct AuthStatus {
    pub password_required: bool,
    pub authenticated: bool,
}

#[derive(Debug)]
pub enum LoginError {
    InvalidPassword,
    Unavailable,
}

impl Auth {
    pub fn new(password: Option<Arc<str>>) -> Self {
        Self {
            password,
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn password_required(&self) -> bool {
        self.password.is_some()
    }

    pub fn status(&self, headers: &HeaderMap) -> AuthStatus {
        AuthStatus {
            password_required: self.password_required(),
            authenticated: self.authenticated(headers),
        }
    }

    pub fn authenticated(&self, headers: &HeaderMap) -> bool {
        let Some(password) = &self.password else {
            return true;
        };
        // Keep command-line and existing API integrations working alongside browser sessions.
        if headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Basic "))
            .and_then(|value| base64::engine::general_purpose::STANDARD.decode(value).ok())
            .is_some_and(|value| {
                constant_time_equal(&value, format!("picsoc:{password}").as_bytes())
            })
        {
            return true;
        }
        let Some(token) = cookie_token(headers) else {
            return false;
        };
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if sessions
            .get(token)
            .is_some_and(|expires| *expires > Instant::now())
        {
            return true;
        }
        sessions.remove(token);
        false
    }

    pub fn login(&self, password: &str) -> Result<Option<String>, LoginError> {
        let Some(expected) = &self.password else {
            return Ok(None);
        };
        if !constant_time_equal(password.as_bytes(), expected.as_bytes()) {
            return Err(LoginError::InvalidPassword);
        }
        let mut random = [0u8; 32];
        getrandom::fill(&mut random).map_err(|_| LoginError::Unavailable)?;
        let mut token = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random);
        let now = Instant::now();
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        sessions.retain(|_, expires| *expires > now);
        // Random collisions are improbable, but never replace another live session's token.
        while sessions.contains_key(&token) {
            getrandom::fill(&mut random).map_err(|_| LoginError::Unavailable)?;
            token = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random);
        }
        if sessions.len() >= MAX_SESSIONS
            && let Some(oldest) = sessions
                .iter()
                .min_by_key(|(_, expires)| **expires)
                .map(|(token, _)| token.clone())
        {
            sessions.remove(&oldest);
        }
        sessions.insert(token.clone(), now + Duration::from_secs(SESSION_SECONDS));
        Ok(Some(token))
    }

    pub fn logout(&self, headers: &HeaderMap) {
        if let Some(token) = cookie_token(headers) {
            self.sessions
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .remove(token);
        }
    }
}

pub fn session_cookie(token: &str, secure: bool) -> String {
    let secure_flag = if secure { "; Secure" } else { "" };
    format!(
        "{SESSION_COOKIE}={token}; HttpOnly; SameSite=Strict; Path=/; Max-Age={SESSION_SECONDS}{secure_flag}"
    )
}

pub fn clear_session_cookie(secure: bool) -> String {
    if secure {
        format!("{CLEAR_SESSION_COOKIE}; Secure")
    } else {
        CLEAR_SESSION_COOKIE.into()
    }
}

fn cookie_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|cookie| cookie.trim().split_once('='))
        .find_map(|(name, token)| {
            let token = token.trim();
            (name == SESSION_COOKIE
                && token.len() == 43
                && token
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')))
            .then_some(token)
        })
}

fn constant_time_equal(actual: &[u8], expected: &[u8]) -> bool {
    let mut diff = actual.len() ^ expected.len();
    for (index, byte) in expected.iter().enumerate() {
        diff |= usize::from(actual.get(index).copied().unwrap_or(0) ^ byte);
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(token: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            format!("unrelated=value; {SESSION_COOKIE}={token}")
                .parse()
                .unwrap(),
        );
        headers
    }

    #[test]
    fn passwords_require_full_value_and_support_unicode() {
        let auth = Auth::new(Some(Arc::from("测试密码")));
        for wrong in ["", "测试", "测试密码extra", "other"] {
            assert!(matches!(
                auth.login(wrong),
                Err(LoginError::InvalidPassword)
            ));
        }
        assert!(auth.sessions.lock().unwrap().is_empty());
        let token = auth.login("测试密码").unwrap().unwrap();
        assert!(auth.authenticated(&headers(&token)));
        assert!(!constant_time_equal(b"short", b"shortextra"));
        assert!(!constant_time_equal(b"longextra", b"long"));
    }

    #[test]
    fn sessions_are_random_revocable_expiring_and_restart_invalidates_them() {
        let auth = Auth::new(Some(Arc::from("secret-password")));
        let first = auth.login("secret-password").unwrap().unwrap();
        let second = auth.login("secret-password").unwrap().unwrap();
        assert_ne!(first, second);
        assert_eq!(
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(&first)
                .unwrap()
                .len(),
            32
        );
        assert!(!first.contains("secret-password"));
        assert!(auth.clone().authenticated(&headers(&first)));
        assert!(!Auth::new(Some(Arc::from("secret-password"))).authenticated(&headers(&first)));
        auth.logout(&headers(&first));
        assert!(!auth.authenticated(&headers(&first)));
        assert!(auth.authenticated(&headers(&second)));
        auth.sessions
            .lock()
            .unwrap()
            .insert(second.clone(), Instant::now() - Duration::from_secs(1));
        assert!(!auth.authenticated(&headers(&second)));
        assert!(!auth.sessions.lock().unwrap().contains_key(&second));
    }

    #[test]
    fn session_storage_is_bounded_and_cookie_and_basic_are_independent() {
        let auth = Auth::new(Some(Arc::from("password")));
        let oldest = auth.login("password").unwrap().unwrap();
        auth.sessions
            .lock()
            .unwrap()
            .insert(oldest.clone(), Instant::now() + Duration::from_secs(1));
        for _ in 0..MAX_SESSIONS {
            auth.login("password").unwrap();
        }
        assert_eq!(auth.sessions.lock().unwrap().len(), MAX_SESSIONS);
        assert!(!auth.authenticated(&headers(&oldest)));
        let mut basic = HeaderMap::new();
        let token = base64::engine::general_purpose::STANDARD.encode("picsoc:password");
        basic.insert(
            header::AUTHORIZATION,
            format!("Basic {token}").parse().unwrap(),
        );
        assert!(auth.authenticated(&basic));
        assert!(!auth.authenticated(&headers("invalid")));
    }

    #[test]
    fn without_a_password_no_session_is_needed() {
        let auth = Auth::new(None);
        assert!(!auth.password_required());
        assert!(auth.authenticated(&HeaderMap::new()));
        assert!(auth.login("").unwrap().is_none());
    }
}
