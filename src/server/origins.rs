//! One reloadable policy shared by CORS, request validation and service status.
use super::{ServerConfig, normalize_browser_origin};
use anyhow::Result;
use axum::http::HeaderValue;
use serde::Serialize;
use std::sync::{Arc, RwLock};

#[derive(Clone, Debug, Serialize)]
pub struct OriginPolicy {
    pub allowed_origins: Vec<String>,
    pub allow_loopback_origins: bool,
}

impl OriginPolicy {
    pub fn from_config(config: &ServerConfig) -> Result<Self> {
        let allowed_origins = config
            .allowed_origins
            .iter()
            .map(|origin| normalize_browser_origin(origin))
            .collect::<Result<Vec<_>>>()?;
        anyhow::ensure!(
            config.allow_loopback_origins || !allowed_origins.is_empty(),
            "server.allowed_origins must not be empty when allow_loopback_origins is false"
        );
        Ok(Self {
            allowed_origins,
            allow_loopback_origins: config.allow_loopback_origins,
        })
    }

    pub fn allows(&self, origin: &HeaderValue) -> bool {
        let Ok(origin) = origin.to_str() else {
            return false;
        };
        if self.allowed_origins.iter().any(|allowed| allowed == origin) {
            return true;
        }
        if !self.allow_loopback_origins {
            return false;
        }
        let Ok(parsed) = url::Url::parse(origin) else {
            return false;
        };
        // Browser origins are serialized scheme/host/port tuples. Reject URL paths,
        // credentials, opaque origins and alternative spellings such as 127.1.
        matches!(parsed.scheme(), "http" | "https")
            && origin == parsed.origin().ascii_serialization()
            && matches!(parsed.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))
    }

    pub fn apply(&self, config: &mut ServerConfig) {
        config.allowed_origins.clone_from(&self.allowed_origins);
        config.allow_loopback_origins = self.allow_loopback_origins;
    }
}

#[derive(Clone)]
pub struct BrowserOrigins(Arc<RwLock<OriginPolicy>>);

impl BrowserOrigins {
    pub fn new(config: &ServerConfig) -> Result<Self> {
        Ok(Self(Arc::new(RwLock::new(OriginPolicy::from_config(
            config,
        )?))))
    }
    pub fn snapshot(&self) -> OriginPolicy {
        self.0.read().expect("origin policy lock poisoned").clone()
    }
    pub fn allows(&self, origin: &HeaderValue) -> bool {
        self.0
            .read()
            .expect("origin policy lock poisoned")
            .allows(origin)
    }
    pub fn replace(&self, config: &ServerConfig) -> Result<OriginPolicy> {
        let policy = OriginPolicy::from_config(config)?;
        *self.0.write().expect("origin policy lock poisoned") = policy.clone();
        Ok(policy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn loopback_is_literal_http_origin_only_and_can_be_disabled() {
        let config: ServerConfig = toml::from_str("").unwrap();
        assert!(config.allow_loopback_origins);
        let policy = OriginPolicy::from_config(&config).unwrap();
        for origin in [
            "http://localhost:39491",
            "https://localhost",
            "http://127.0.0.1:1",
            "https://[::1]:65535",
            "http://[::1]",
        ] {
            assert!(
                policy.allows(&HeaderValue::from_str(origin).unwrap()),
                "{origin}"
            );
        }
        for origin in [
            "null",
            "file://localhost",
            "ws://localhost",
            "http://localhost.evil.test:4000",
            "http://sub.localhost:4000",
            "http://127.0.0.2",
            "http://127.1",
            "http://2130706433",
            "http://[::ffff:127.0.0.1]",
            "http://localhost/",
            "http://localhost/path",
            "http://user@localhost",
            "http://localhost?x=1",
            "http://localhost#x",
            "http://localhost:65536",
            "http://localhost:1 http://localhost:2",
            "http://example.org",
        ] {
            assert!(
                !policy.allows(&HeaderValue::from_str(origin).unwrap()),
                "{origin}"
            );
        }
        let shared = BrowserOrigins::new(&config).unwrap();
        let mut strict = config.clone();
        strict.allow_loopback_origins = false;
        strict.allowed_origins = vec!["https://board.example.org".into()];
        shared.replace(&strict).unwrap();
        assert!(!shared.allows(&HeaderValue::from_static("http://localhost:39491")));
        assert!(shared.allows(&HeaderValue::from_static("https://board.example.org")));
        strict.allowed_origins = vec!["*".into()];
        assert!(shared.replace(&strict).is_err());
        assert!(shared.allows(&HeaderValue::from_static("https://board.example.org")));
    }
}
