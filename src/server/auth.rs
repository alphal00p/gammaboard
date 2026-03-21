use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordVerifier},
};
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
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use std::{
    env,
    time::{SystemTime, UNIX_EPOCH},
};

use super::{ApiError, AppState};

const COOKIE_NAME: &str = "gammaboard_admin_session";
const SESSION_TTL_SECS: u64 = 12 * 60 * 60;

#[derive(Clone)]
pub struct AuthConfig {
    password_hash: String,
    encoding_key: EncodingKey,
    decoding_key: DecodingKey,
}

#[derive(Debug, Serialize)]
pub struct SessionStatus {
    pub authenticated: bool,
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SessionClaims {
    exp: u64,
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
        encoding_key: EncodingKey::from_secret(session_secret.as_bytes()),
        decoding_key: DecodingKey::from_secret(session_secret.as_bytes()),
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

    let Some(token) = cookie_value(request.headers(), COOKIE_NAME) else {
        return ApiError::Unauthorized("admin login required".to_string()).into_response();
    };
    if verify_session_token(auth, &token).is_none() {
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

    let token = sign_session_token(auth)?;
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
            cookie_value(headers, COOKIE_NAME).and_then(|value| verify_session_token(auth, &value))
        })
        .is_some();
    SessionStatus { authenticated }
}

fn verify_password_hash(encoded: &str, password: &str) -> bool {
    let Ok(hash) = PasswordHash::new(encoded) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &hash)
        .is_ok()
}

fn sign_session_token(auth: &AuthConfig) -> Result<String, ApiError> {
    encode(
        &Header::new(Algorithm::HS256),
        &SessionClaims {
            exp: now_unix_secs() + SESSION_TTL_SECS,
        },
        &auth.encoding_key,
    )
    .map_err(|err| ApiError::Internal(err.to_string()))
}

fn verify_session_token(auth: &AuthConfig, token: &str) -> Option<SessionClaims> {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_exp = true;
    decode::<SessionClaims>(token, &auth.decoding_key, &validation)
        .ok()
        .map(|value| value.claims)
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

fn session_cookie(token: &str, max_age_secs: u64) -> String {
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

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
