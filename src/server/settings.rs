use super::{AppState, REPOSITORY_URL, json_response};
use crate::api::ApiError;
use axum::{Json, extract::State};
use std::path::Path;

pub(super) async fn get_settings_overview(
    State(state): State<AppState>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    let runtime_config = state.runtime.runtime_config();
    json_response(serde_json::json!({
        "repository": { "url": REPOSITORY_URL },
        "paths": {
            "runtime_config": display_absolute_path(state.runtime.runtime_config_path()),
            "server_config": display_absolute_path(&state.server_config_path),
            "resources_root": display_absolute_path(state.runtime.primary_resource_root()),
            "run_templates_dir": display_absolute_path(&state.run_templates_dir),
            "task_templates_dir": display_absolute_path(&state.task_templates_dir),
            "node_templates_dir": display_absolute_path(&state.node_templates_dir),
            "postgres_data_dir": display_absolute_path(Path::new(&runtime_config.local_postgres.data_dir)),
            "postgres_socket_dir": display_absolute_path(Path::new(&runtime_config.local_postgres.socket_dir)),
            "postgres_log_file": display_absolute_path(Path::new(&runtime_config.local_postgres.log_file)),
        },
        "runtime": {
            "database_url": redact_database_url(&runtime_config.database.url),
            "resource_roots": runtime_config.resources.roots,
            "tracing": {
                "persist_runtime_logs": runtime_config.tracing.persist_runtime_logs,
                "db_gammaboard_level": runtime_config.tracing.db_gammaboard_level,
                "db_external_level": runtime_config.tracing.db_external_level,
            },
            "local_postgres": {
                "max_connections": runtime_config.local_postgres.max_connections,
                "listen_addresses": runtime_config.local_postgres.listen_addresses,
                "host_auth_cidr": runtime_config.local_postgres.host_auth_cidr,
                "shared_buffers": runtime_config.local_postgres.shared_buffers,
                "effective_cache_size": runtime_config.local_postgres.effective_cache_size,
                "work_mem": runtime_config.local_postgres.work_mem,
                "checkpoint_timeout": runtime_config.local_postgres.checkpoint_timeout,
                "max_wal_size": runtime_config.local_postgres.max_wal_size,
                "wal_compression": runtime_config.local_postgres.wal_compression,
                "synchronous_commit": runtime_config.local_postgres.synchronous_commit,
            },
        },
        "server": {
            "name": state.server_name,
            "api_bind": state.api_bind,
            "secure_cookie": state.secure_cookie,
            "auth_enabled": state.auth.is_some(),
            "allow_local_node_spawn": state.allow_local_node_spawn,
            "allowed_origins": state.allowed_origins.iter()
                .filter_map(|origin| origin.to_str().ok()).collect::<Vec<_>>(),
        },
    }))
}

fn display_absolute_path(path: &Path) -> String {
    if path.is_absolute() {
        path.display().to_string()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path).display().to_string())
            .unwrap_or_else(|_| path.display().to_string())
    }
}

fn redact_database_url(raw: &str) -> String {
    let Ok(mut parsed) = url::Url::parse(raw) else {
        return raw.to_string();
    };
    if parsed.password().is_some() {
        let _ = parsed.set_password(Some("*****"));
    }
    parsed.to_string()
}
