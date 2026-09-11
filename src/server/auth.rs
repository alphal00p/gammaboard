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
use std::time::{SystemTime, UNIX_EPOCH};

use super::{ApiError, AppState, ServerAuthConfig};

const COOKIE_NAME: &str = "gammaboard_admin_session";
const SESSION_TTL_SECS: u64 = 12 * 60 * 60;
const SESSION_AUDIENCE: &str = "gammaboard-dashboard";

#[derive(Clone)]
pub struct AuthConfig {
    password_hash: String,
    encoding_key: EncodingKey,
    decoding_key: DecodingKey,
    issuer: String,
    session_version: String,
}

#[derive(Debug, Serialize)]
pub struct SessionStatus {
    pub authenticated: bool,
    pub allow_local_node_spawn: bool,
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SessionClaims {
    exp: u64,
    iat: u64,
    iss: String,
    aud: String,
    session_version: String,
}

impl AuthConfig {
    pub fn from_server_config(config: &ServerAuthConfig, server_name: &str) -> Self {
        Self {
            password_hash: config.admin_password_hash.trim().to_string(),
            encoding_key: EncodingKey::from_secret(config.session_secret.as_bytes()),
            decoding_key: DecodingKey::from_secret(config.session_secret.as_bytes()),
            issuer: server_name.to_string(),
            session_version: config.session_version.clone(),
        }
    }
}

pub async fn require_admin_session(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if let Err(err) = validate_origin(request.headers(), &state.allowed_origins) {
        return err.into_response();
    }
    let Some(auth) = &state.auth else {
        return next.run(request).await;
    };

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
    validate_origin(&headers, &state.allowed_origins)?;
    let Some(auth) = &state.auth else {
        return Ok(Json(SessionStatus {
            authenticated: true,
            allow_local_node_spawn: state.allow_local_node_spawn,
        })
        .into_response());
    };
    if !verify_password_hash(&auth.password_hash, &payload.password) {
        return Err(ApiError::Unauthorized("invalid password".to_string()));
    }

    let token = sign_session_token(auth)?;
    Ok(response_with_cookie(
        session_cookie(&token, SESSION_TTL_SECS, state.secure_cookie),
        SessionStatus {
            authenticated: true,
            allow_local_node_spawn: state.allow_local_node_spawn,
        },
    ))
}

pub async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    validate_origin(&headers, &state.allowed_origins)?;
    let authenticated = state.auth.is_none();
    Ok(response_with_cookie(
        session_cookie("", 0, state.secure_cookie),
        SessionStatus {
            authenticated,
            allow_local_node_spawn: state.allow_local_node_spawn,
        },
    ))
}

pub fn auth_status_from_headers(state: &AppState, headers: &HeaderMap) -> SessionStatus {
    let authenticated = match &state.auth {
        Some(auth) => cookie_value(headers, COOKIE_NAME)
            .and_then(|value| verify_session_token(auth, &value))
            .is_some(),
        None => true,
    };
    SessionStatus {
        authenticated,
        allow_local_node_spawn: state.allow_local_node_spawn,
    }
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
    let now = now_unix_secs();
    encode(
        &Header::new(Algorithm::HS256),
        &SessionClaims {
            exp: now + SESSION_TTL_SECS,
            iat: now,
            iss: auth.issuer.clone(),
            aud: SESSION_AUDIENCE.to_string(),
            session_version: auth.session_version.clone(),
        },
        &auth.encoding_key,
    )
    .map_err(|err| ApiError::Internal(err.to_string()))
}

fn verify_session_token(auth: &AuthConfig, token: &str) -> Option<SessionClaims> {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_exp = true;
    validation.set_issuer(&[auth.issuer.as_str()]);
    validation.set_audience(&[SESSION_AUDIENCE]);
    decode::<SessionClaims>(token, &auth.decoding_key, &validation)
        .ok()
        .map(|value| value.claims)
        .filter(|claims| claims.session_version == auth.session_version)
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

fn session_cookie(token: &str, max_age_secs: u64, secure: bool) -> String {
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

pub(super) fn validate_origin(
    headers: &HeaderMap,
    allowed_origins: &[HeaderValue],
) -> Result<(), ApiError> {
    let Some(origin) = headers.get(ORIGIN) else {
        // Non-browser CLI and worker requests need not carry an Origin header.
        return Ok(());
    };
    if allowed_origins.iter().any(|allowed| allowed == origin) {
        return Ok(());
    }
    let origin = origin.to_str().unwrap_or("<invalid Origin header>");
    let quoted_origin = format!("'{}'", origin.replace('\'', "'\\''"));
    let message = format!(
        "Browser origin {origin:?} is not allowed. Restart gammaboard deploy with --allowed-origin {quoted_origin}, or add this origin to server.allowed_origins in your server configuration."
    );
    tracing::warn!(
        origin,
        "browser origin rejected; configure --allowed-origin or server.allowed_origins"
    );
    Err(ApiError::Forbidden(message))
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_origins_require_an_exact_allowlist_match() {
        let allowed = vec![HeaderValue::from_static("http://localhost:39491")];
        let mut headers = HeaderMap::new();
        assert!(validate_origin(&headers, &allowed).is_ok());
        headers.insert(ORIGIN, allowed[0].clone());
        assert!(validate_origin(&headers, &allowed).is_ok());
        for origin in [
            "http://localhost:8081",
            "http://127.0.0.1:39491",
            "https://example.org",
            "null",
        ] {
            headers.insert(ORIGIN, HeaderValue::from_str(origin).unwrap());
            let err = validate_origin(&headers, &allowed).unwrap_err();
            assert!(err.to_string().contains(origin));
            assert!(err.to_string().contains("--allowed-origin"));
            assert_eq!(
                err.into_response().status(),
                axum::http::StatusCode::FORBIDDEN
            );
        }
    }

    fn auth_config(session_version: &str) -> AuthConfig {
        AuthConfig::from_server_config(
            &ServerAuthConfig {
                admin_password_hash: "unused".to_string(),
                session_secret: "test-session-secret-with-enough-entropy".to_string(),
                session_version: session_version.to_string(),
            },
            "test-server",
        )
    }

    #[test]
    fn session_tokens_require_matching_issuer_audience_and_session_version() {
        let auth = auth_config("1");
        let token = sign_session_token(&auth).expect("sign token");
        assert!(verify_session_token(&auth, &token).is_some());
        assert!(verify_session_token(&auth_config("2"), &token).is_none());
        assert!(
            verify_session_token(
                &AuthConfig::from_server_config(
                    &ServerAuthConfig {
                        admin_password_hash: "unused".to_string(),
                        session_secret: "test-session-secret-with-enough-entropy".to_string(),
                        session_version: "1".to_string(),
                    },
                    "other-server",
                ),
                &token
            )
            .is_none()
        );
    }
}
