use axum::{
    body::Body,
    extract::State,
    http::{
        HeaderMap, HeaderValue, Request,
        header::{COOKIE, ORIGIN, SET_COOKIE},
    },
    middleware::Next,
    response::{IntoResponse, Json, Response},
};
use base64::{Engine as _, engine::general_purpose::STANDARD_NO_PAD};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::{
    env,
    time::{SystemTime, UNIX_EPOCH},
};
use subtle::ConstantTimeEq;

use super::{ApiError, AppState};

type HmacSha256 = Hmac<Sha256>;

const COOKIE_NAME: &str = "gammaboard_admin_session";
const SESSION_TTL_SECS: i64 = 12 * 60 * 60;
const PBKDF2_KEY_LEN: usize = 32;

#[derive(Clone)]
pub struct AuthConfig {
    password_hash: String,
    session_secret: Vec<u8>,
}

#[derive(Debug, Serialize)]
pub struct SessionStatus {
    pub authenticated: bool,
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub password: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct SessionClaims {
    exp: i64,
}

pub fn load_auth_config() -> anyhow::Result<Option<AuthConfig>> {
    let Some(password_hash) = env::var("GAMMABOARD_ADMIN_PASSWORD_HASH")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };

    let session_secret = env::var("GAMMABOARD_SESSION_SECRET").map_err(|_| {
        anyhow::anyhow!("missing GAMMABOARD_SESSION_SECRET while dashboard auth is enabled")
    })?;

    Ok(Some(AuthConfig {
        password_hash,
        session_secret: session_secret.into_bytes(),
    }))
}

pub fn parse_allowed_origin() -> Option<HeaderValue> {
    let value = env::var("GAMMABOARD_ALLOWED_ORIGIN")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "http://localhost:3000".to_string());
    HeaderValue::from_str(value.trim()).ok()
}

pub async fn require_admin_session(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let Some(auth) = &state.auth else {
        return ApiError::Unavailable(
            "dashboard auth is not configured (set GAMMABOARD_ADMIN_PASSWORD_HASH and GAMMABOARD_SESSION_SECRET)"
                .to_string(),
        )
        .into_response();
    };

    if !origin_allowed(request.headers(), state.allowed_origin.as_ref()) {
        return ApiError::Unauthorized("invalid origin".to_string()).into_response();
    }

    let token = cookie_value(request.headers(), COOKIE_NAME);
    if !token
        .as_deref()
        .and_then(|value| verify_session_token(&auth.session_secret, value))
        .is_some()
    {
        return ApiError::Unauthorized("admin login required".to_string()).into_response();
    }

    next.run(request).await
}

pub async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Json(payload): axum::extract::Json<LoginRequest>,
) -> Result<Response, ApiError> {
    let Some(auth) = &state.auth else {
        return Err(ApiError::Unavailable(
            "dashboard auth is not configured".to_string(),
        ));
    };
    if !origin_allowed(&headers, state.allowed_origin.as_ref()) {
        return Err(ApiError::Unauthorized("invalid origin".to_string()));
    }
    if !verify_password_hash(&auth.password_hash, &payload.password) {
        return Err(ApiError::Unauthorized("invalid password".to_string()));
    }

    let token = sign_session_token(&auth.session_secret)?;
    Ok(response_with_cookie(
        session_cookie(&token, SESSION_TTL_SECS),
        SessionStatus {
            authenticated: true,
        },
    ))
}

pub async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    if !origin_allowed(&headers, state.allowed_origin.as_ref()) {
        return Err(ApiError::Unauthorized("invalid origin".to_string()));
    }
    Ok(response_with_cookie(
        session_cookie("", 0),
        SessionStatus {
            authenticated: false,
        },
    ))
}

pub fn auth_status_from_headers(state: &AppState, headers: &HeaderMap) -> SessionStatus {
    let authenticated = state
        .auth
        .as_ref()
        .and_then(|auth| {
            cookie_value(headers, COOKIE_NAME)
                .as_deref()
                .and_then(|value| verify_session_token(&auth.session_secret, value))
        })
        .is_some();
    SessionStatus { authenticated }
}

fn response_with_cookie<T: Serialize>(cookie: String, payload: T) -> Response {
    let mut response =
        Json(serde_json::to_value(payload).unwrap_or_else(|_| serde_json::json!({})))
            .into_response();
    if let Ok(value) = HeaderValue::from_str(&cookie) {
        response.headers_mut().append(SET_COOKIE, value);
    }
    response
}

fn session_cookie(token: &str, max_age_secs: i64) -> String {
    let secure = env_true("GAMMABOARD_SECURE_COOKIE");
    let mut parts = vec![
        format!("{COOKIE_NAME}={token}"),
        "Path=/".to_string(),
        "HttpOnly".to_string(),
        "SameSite=Lax".to_string(),
        format!("Max-Age={max_age_secs}"),
    ];
    if secure {
        parts.push("Secure".to_string());
    }
    parts.join("; ")
}

fn cookie_value(headers: &HeaderMap, key: &str) -> Option<String> {
    let raw = headers.get(COOKIE)?.to_str().ok()?;
    raw.split(';').find_map(|part| {
        let mut pieces = part.trim().splitn(2, '=');
        let name = pieces.next()?.trim();
        let value = pieces.next()?.trim();
        (name == key).then(|| value.to_string())
    })
}

fn env_true(name: &str) -> bool {
    env::var(name)
        .map(|value| matches!(value.trim(), "1" | "true" | "TRUE" | "yes" | "on"))
        .unwrap_or(false)
}

fn origin_allowed(headers: &HeaderMap, allowed_origin: Option<&HeaderValue>) -> bool {
    let Some(origin) = headers.get(ORIGIN) else {
        return true;
    };
    allowed_origin.is_none_or(|allowed| allowed == origin)
}

fn sign_session_token(secret: &[u8]) -> Result<String, ApiError> {
    let claims = SessionClaims {
        exp: now_unix_secs() + SESSION_TTL_SECS,
    };
    let payload = serde_json::to_vec(&claims).map_err(|err| ApiError::Internal(err.to_string()))?;
    let payload = STANDARD_NO_PAD.encode(payload);
    let signature = sign(secret, payload.as_bytes())?;
    Ok(format!("{payload}.{signature}"))
}

fn verify_session_token(secret: &[u8], token: &str) -> Option<SessionClaims> {
    let (payload, signature) = token.split_once('.')?;
    let expected = sign(secret, payload.as_bytes()).ok()?;
    if expected.as_bytes().ct_eq(signature.as_bytes()).unwrap_u8() != 1 {
        return None;
    }
    let claims: SessionClaims =
        serde_json::from_slice(&STANDARD_NO_PAD.decode(payload).ok()?).ok()?;
    (claims.exp > now_unix_secs()).then_some(claims)
}

fn sign(secret: &[u8], payload: &[u8]) -> Result<String, ApiError> {
    let mut mac =
        HmacSha256::new_from_slice(secret).map_err(|err| ApiError::Internal(err.to_string()))?;
    mac.update(payload);
    Ok(STANDARD_NO_PAD.encode(mac.finalize().into_bytes()))
}

fn verify_password_hash(encoded: &str, password: &str) -> bool {
    let Some((prefix, rest)) = encoded.split_once('$') else {
        return false;
    };
    if prefix != "pbkdf2_sha256" {
        return false;
    }
    let mut pieces = rest.split('$');
    let Some(iterations) = pieces.next().and_then(|value| value.parse::<u32>().ok()) else {
        return false;
    };
    let Some(salt) = pieces
        .next()
        .and_then(|value| STANDARD_NO_PAD.decode(value).ok())
    else {
        return false;
    };
    let Some(expected) = pieces
        .next()
        .and_then(|value| STANDARD_NO_PAD.decode(value).ok())
    else {
        return false;
    };
    if pieces.next().is_some() || expected.len() != PBKDF2_KEY_LEN {
        return false;
    }

    let actual = pbkdf2_sha256(password.as_bytes(), &salt, iterations);
    actual.ct_eq(&expected).unwrap_u8() == 1
}

fn pbkdf2_sha256(password: &[u8], salt: &[u8], iterations: u32) -> Vec<u8> {
    let mut block_input = Vec::with_capacity(salt.len() + 4);
    block_input.extend_from_slice(salt);
    block_input.extend_from_slice(&1u32.to_be_bytes());

    let mut u = hmac_sha256(password, &block_input);
    let mut out = u.clone();
    for _ in 1..iterations.max(1) {
        u = hmac_sha256(password, &u);
        for (lhs, rhs) in out.iter_mut().zip(&u) {
            *lhs ^= rhs;
        }
    }
    out
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("hmac key");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

fn now_unix_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
