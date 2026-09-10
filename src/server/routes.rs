use super::*;
use axum::{Router, extract::DefaultBodyLimit, middleware};
use tower_http::cors::{AllowOrigin, CorsLayer};

pub(super) fn build_app(state: AppState) -> Router {
    let public = Router::new()
        .route("/health", get(health_check))
        .route(
            "/auth/session",
            get(get_session_status).post(get_session_status),
        )
        .route("/auth/login", post(login))
        .route("/auth/logout", post(logout));

    let protected = Router::new()
        .route("/settings", get(settings::get_settings_overview))
        .route("/runs", get(get_runs).post(create_run))
        .route("/runs/clone", post(clone_run))
        .route("/runs/:id", delete(delete_run))
        .route("/runs/:id/pause", post(pause_run))
        .route("/runs/:id/repro-toml", get(get_run_repro_toml))
        .route("/runs/:id/panels", get(get_run_panels))
        .route("/runs/:id/tasks", get(get_run_tasks).post(add_run_tasks))
        .route("/runs/:id/tasks/:task_id", delete(delete_run_task))
        .route("/runs/:id/tasks/:task_id/output", post(get_run_task_output))
        .route(
            "/runs/:id/tasks/:task_id/queue-tuning",
            post(update_run_task_queue_tuning),
        )
        .route("/runs/:id/performance", get(get_run_performance))
        .route("/runs/:id/auto-assign", post(auto_assign_run))
        .route("/nodes", get(get_nodes))
        .route("/nodes/:id/panels", get(get_node_panels))
        .route("/nodes/:id/assign", post(assign_node))
        .route("/nodes/:id/unassign", post(unassign_node))
        .route("/nodes/:id/stop", post(stop_node))
        .route("/nodes/unassign-all", post(unassign_all_nodes))
        .route("/nodes/stop-all", post(stop_all_nodes))
        .route("/nodes/auto-run", post(auto_run_nodes))
        .route(
            "/node-launch-requests",
            get(get_node_launch_requests).post(create_node_launch_request),
        )
        .route(
            "/node-launch-requests/claim-external",
            post(claim_external_node_launch_request),
        )
        .route(
            "/node-launch-requests/:id/progress",
            post(update_node_launch_request_progress),
        )
        .route("/templates/:kind", get(list_templates).post(save_template))
        .route(
            "/templates/:kind/:name",
            get(get_template).delete(delete_template),
        )
        .route("/logs", get(get_logs))
        .route("/histogram-bundle/export", post(export_histogram_bundle))
        .route("/admin/db/restart", post(restart_db))
        .route("/admin/control/shutdown", post(shutdown_control_process))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            require_admin_session,
        ));

    Router::new()
        .nest("/api", public.merge(protected))
        .layer(DefaultBodyLimit::max(2 * 1024 * 1024))
        .layer(cors_layer(state.allowed_origins.clone()))
        .layer(middleware::from_fn(request_context_middleware))
        .with_state(state)
}

fn cors_layer(allowed_origins: Vec<axum::http::HeaderValue>) -> CorsLayer {
    CorsLayer::new()
        .allow_credentials(true)
        .allow_methods([
            axum::http::Method::GET,
            axum::http::Method::POST,
            axum::http::Method::DELETE,
            axum::http::Method::OPTIONS,
        ])
        .allow_headers([axum::http::header::CONTENT_TYPE])
        .allow_origin(AllowOrigin::list(allowed_origins))
}
