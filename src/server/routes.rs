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
        .route(
            "/v1/manage",
            post(manage::handle).layer(DefaultBodyLimit::max(64 * 1024 * 1024)),
        )
        .route("/v1/database/backup", post(manage::backup))
        .route("/settings", get(settings::get_settings_overview))
        .route("/runs", get(get_runs))
        .route("/runs/:id", get(get_run_summary))
        .route("/runs/:id/definition", get(get_run_definition))
        .route("/runs/:id/panels", get(get_run_panels))
        .route("/runs/:id/tasks", get(get_run_tasks))
        .route(
            "/runs/:id/tasks/:task_id/definition",
            get(get_task_definition),
        )
        .route("/runs/:id/tasks/:task_id/output", post(get_run_task_output))
        .route("/runs/:id/tasks/:task_id/campaign", get(campaign::get))
        .route("/runs/:id/performance", get(get_run_performance))
        .route(
            "/runs/:id/performance/graphs",
            get(get_run_performance_graphs),
        )
        .route("/runs/:id/metrics", get(get_run_metrics))
        .route("/workers", get(get_workers))
        .route("/workers/:id/panels", get(get_node_panels))
        .route(
            "/node-launch-requests/claim-external",
            post(claim_external_node_launch_request),
        )
        .route(
            "/node-launch-requests/:id/progress",
            post(update_node_launch_request_progress),
        )
        .route(
            "/node-launch-requests/:id/lease",
            post(renew_external_launch_lease),
        )
        .route("/templates/:kind", get(list_templates))
        .route("/templates/:kind/:name", get(get_template))
        .route("/logs", get(get_logs))
        .route(
            "/histogram-bundle/export",
            post(export_histogram_bundle)
                .layer(DefaultBodyLimit::max(histogram_import::MAX_IMPORT_BYTES)),
        )
        .route(
            "/histogram-bundle/campaign-export",
            post(histogram_export::export_campaign),
        )
        .route(
            "/histogram-bundle/import",
            post(histogram_import::import_hwu)
                .layer(DefaultBodyLimit::max(histogram_import::MAX_IMPORT_BYTES)),
        )
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            require_admin_session,
        ));

    Router::new()
        .nest(
            "/api",
            public.merge(protected).fallback(|| async {
                (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({"error":"unknown API route"})),
                )
            }),
        )
        .layer(DefaultBodyLimit::max(2 * 1024 * 1024))
        .layer(cors_layer(state.origins.clone()))
        .layer(middleware::from_fn(request_context_middleware))
        .with_state(state)
}

fn cors_layer(origins: origins::BrowserOrigins) -> CorsLayer {
    CorsLayer::new()
        .allow_credentials(true)
        .allow_methods([
            axum::http::Method::GET,
            axum::http::Method::POST,
            axum::http::Method::OPTIONS,
        ])
        .allow_headers([axum::http::header::CONTENT_TYPE])
        .allow_origin(AllowOrigin::predicate(move |origin, parts| {
            parts
                .headers
                .get_all(axum::http::header::ORIGIN)
                .iter()
                .count()
                == 1
                && origins.allows(origin)
        }))
}
