use axum::response::sse::{Event, KeepAlive, Sse};
use axum::{
    Router,
    extract::{Path as AxumPath, Query, State},
    http::Request,
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Json, Response},
    routing::get,
};
use clap::Args;
use futures_core::Stream;
use gammaboard::stores::RunReadStore;
use gammaboard::{BinResult, PgStore, init_pg_store};
use serde::Deserialize;
use std::{
    collections::HashMap,
    convert::Infallible,
    env,
    fmt::Display,
    net::SocketAddr,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};
use tokio::sync::{Mutex, broadcast, mpsc};
use tower_http::cors::CorsLayer;
use tracing::Instrument;

use super::shared::init_cli_tracing;

#[derive(Debug, Args)]
pub struct ServerArgs {
    #[arg(long)]
    bind: Option<SocketAddr>,
    #[arg(long, default_value_t = 10)]
    db_pool_size: u32,
}

pub async fn run_server(args: ServerArgs) -> BinResult {
    let store = init_pg_store(args.db_pool_size).await?;
    init_cli_tracing(&store)?;
    let bind = match args.bind {
        Some(bind) => bind,
        None => {
            let value = env::var("GAMMABOOARD_BACKEND_PORT").map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "missing GAMMABOOARD_BACKEND_PORT (set it in environment or pass --bind)",
                )
            })?;
            let port = value.parse::<u16>().map_err(|err| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("invalid GAMMABOOARD_BACKEND_PORT={value:?}: {err}"),
                )
            })?;
            SocketAddr::from(([0, 0, 0, 0], port))
        }
    };

    let state = AppState {
        store,
        run_stream_hub: Arc::new(Mutex::new(HashMap::new())),
    };

    let app = build_app(state);

    println!("server listening on http://{}", bind);
    println!("api available at http://{}/api", bind);

    let listener = tokio::net::TcpListener::bind(bind).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

#[derive(Clone)]
struct AppState {
    store: PgStore,
    run_stream_hub: RunStreamHub,
}

type RunStreamHub = Arc<Mutex<HashMap<i32, broadcast::Sender<StreamMessage>>>>;

#[derive(Clone)]
enum StreamMessage {
    Stats(String),
    Error(String),
}

#[derive(Deserialize)]
struct LimitQuery {
    #[serde(default = "default_limit")]
    limit: i64,
}

fn default_limit() -> i64 {
    1000
}

#[derive(Deserialize)]
struct LogQuery {
    #[serde(default = "default_log_limit")]
    limit: i64,
    worker_id: Option<String>,
    level: Option<String>,
    after_id: Option<i64>,
}

fn default_log_limit() -> i64 {
    500
}

#[derive(Deserialize)]
struct PerformanceHistoryQuery {
    #[serde(default = "default_perf_history_limit")]
    limit: i64,
    worker_id: Option<String>,
}

fn default_perf_history_limit() -> i64 {
    500
}

fn default_stream_interval_ms() -> u64 {
    1000
}

fn sanitize_stream_interval_ms(interval_ms: u64) -> u64 {
    const MIN_INTERVAL_MS: u64 = 100;
    const MAX_INTERVAL_MS: u64 = 60_000;
    interval_ms.clamp(MIN_INTERVAL_MS, MAX_INTERVAL_MS)
}

fn json_error(status: StatusCode, message: &str) -> Response {
    (status, Json(serde_json::json!({ "error": message }))).into_response()
}

fn internal_error(context: impl Display, err: impl Display, message: &str) -> Response {
    eprintln!("Error {}: {}", context, err);
    json_error(StatusCode::INTERNAL_SERVER_ERROR, message)
}

#[derive(Deserialize)]
struct StreamQuery {
    #[serde(default = "default_stream_interval_ms")]
    interval_ms: u64,
}

#[derive(Deserialize)]
struct WorkersQuery {
    run_id: Option<i32>,
}

struct MpscStream {
    receiver: mpsc::Receiver<Result<Event, Infallible>>,
}

impl Stream for MpscStream {
    type Item = Result<Event, Infallible>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        Pin::new(&mut this.receiver).poll_recv(cx)
    }
}

fn build_app(state: AppState) -> Router {
    let api_routes = Router::new()
        .route("/health", get(health_check))
        .route("/runs", get(get_runs))
        .route("/workers", get(get_workers))
        .route("/runs/:id", get(get_run))
        .route("/runs/:id/stats", get(get_run_stats))
        .route("/runs/:id/logs", get(get_run_logs))
        .route("/runs/:id/aggregated", get(get_run_aggregated_results))
        .route(
            "/runs/:id/aggregated/latest",
            get(get_run_aggregated_latest),
        )
        .route(
            "/runs/:id/performance/evaluator",
            get(get_run_evaluator_performance_history),
        )
        .route(
            "/runs/:id/performance/sampler-aggregator",
            get(get_run_sampler_performance_history),
        )
        .route("/runs/:id/stream", get(stream_run_stats))
        .layer(middleware::from_fn(request_context_middleware))
        .with_state(state);

    Router::new()
        .nest("/api", api_routes)
        .layer(CorsLayer::permissive())
}

async fn request_context_middleware(request: Request<axum::body::Body>, next: Next) -> Response {
    let method = request.method().to_string();
    let path = request.uri().path().to_string();
    let span = tracing::span!(
        tracing::Level::TRACE,
        "api_request",
        source = "server",
        method = %method,
        path = %path
    );
    next.run(request).instrument(span).await
}

async fn subscribe_run_stream(
    state: &AppState,
    run_id: i32,
    interval_ms: u64,
) -> broadcast::Receiver<StreamMessage> {
    let mut streams = state.run_stream_hub.lock().await;
    if let Some(sender) = streams.get(&run_id) {
        return sender.subscribe();
    }

    let (sender, receiver) = broadcast::channel(64);
    streams.insert(run_id, sender.clone());
    drop(streams);

    let store = state.store.clone();
    let hub = state.run_stream_hub.clone();
    let span = tracing::span!(
        tracing::Level::TRACE,
        "api_run_stream_publisher",
        source = "server",
        run_id = run_id
    );
    tokio::spawn(run_stream_publisher(store, hub, run_id, interval_ms, sender).instrument(span));

    receiver
}

async fn run_stream_publisher(
    store: PgStore,
    hub: RunStreamHub,
    run_id: i32,
    interval_ms: u64,
    sender: broadcast::Sender<StreamMessage>,
) {
    let mut ticker = tokio::time::interval(Duration::from_millis(interval_ms));
    loop {
        ticker.tick().await;

        if sender.receiver_count() == 0 {
            break;
        }

        let progress = match store.get_run_progress(run_id).await {
            Ok(run) => run,
            Err(err) => {
                let payload = serde_json::json!({
                    "error": "Failed to fetch run progress",
                    "details": err.to_string()
                })
                .to_string();
                let _ = sender.send(StreamMessage::Error(payload));
                continue;
            }
        };

        let aggregated = match store.get_latest_aggregated_result(run_id).await {
            Ok(result) => result,
            Err(err) => {
                let payload = serde_json::json!({
                    "error": "Failed to fetch aggregated results",
                    "details": err.to_string()
                })
                .to_string();
                let _ = sender.send(StreamMessage::Error(payload));
                continue;
            }
        };

        let payload = serde_json::json!({
            "run": progress,
            "aggregated": aggregated
        })
        .to_string();
        let _ = sender.send(StreamMessage::Stats(payload));
    }

    let mut streams = hub.lock().await;
    if streams
        .get(&run_id)
        .is_some_and(|existing| existing.same_channel(&sender))
    {
        streams.remove(&run_id);
    }
}

async fn health_check(State(state): State<AppState>) -> impl IntoResponse {
    match state.store.health_check().await {
        Ok(_) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "ok",
                "database": "connected"
            })),
        )
            .into_response(),
        Err(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "status": "error",
                "database": "disconnected"
            })),
        )
            .into_response(),
    }
}

async fn get_runs(State(state): State<AppState>) -> impl IntoResponse {
    match state.store.get_all_runs().await {
        Ok(runs) => (StatusCode::OK, Json(runs)).into_response(),
        Err(e) => internal_error("fetching runs", e, "Failed to fetch runs"),
    }
}

async fn get_workers(
    State(state): State<AppState>,
    Query(params): Query<WorkersQuery>,
) -> impl IntoResponse {
    match state.store.get_registered_workers(params.run_id).await {
        Ok(workers) => (StatusCode::OK, Json(workers)).into_response(),
        Err(e) => internal_error("fetching workers", e, "Failed to fetch workers"),
    }
}

async fn get_run(State(state): State<AppState>, AxumPath(id): AxumPath<i32>) -> impl IntoResponse {
    match state.store.get_run_progress(id).await {
        Ok(Some(run)) => (StatusCode::OK, Json(run)).into_response(),
        Ok(None) => json_error(StatusCode::NOT_FOUND, "Run not found"),
        Err(e) => internal_error(format!("fetching run {id}"), e, "Failed to fetch run"),
    }
}

async fn get_run_stats(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<i32>,
) -> impl IntoResponse {
    match state.store.get_work_queue_stats(id).await {
        Ok(stats) => (StatusCode::OK, Json(stats)).into_response(),
        Err(e) => internal_error(
            format!("fetching stats for run {id}"),
            e,
            "Failed to fetch stats",
        ),
    }
}

async fn get_run_logs(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<i32>,
    Query(params): Query<LogQuery>,
) -> impl IntoResponse {
    let limit = params.limit.clamp(1, 10_000);
    match state
        .store
        .get_worker_logs(
            id,
            limit,
            params.worker_id.as_deref(),
            params.level.as_deref(),
            params.after_id,
        )
        .await
    {
        Ok(logs) => (StatusCode::OK, Json(logs)).into_response(),
        Err(e) => internal_error(
            format!("fetching logs for run {id}"),
            e,
            "Failed to fetch logs",
        ),
    }
}

async fn get_run_aggregated_results(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<i32>,
    Query(params): Query<LimitQuery>,
) -> impl IntoResponse {
    let limit = params.limit.clamp(1, 10_000);
    match state.store.get_aggregated_results(id, limit).await {
        Ok(results) => (StatusCode::OK, Json(results)).into_response(),
        Err(e) => internal_error(
            format!("fetching aggregated results for run {id}"),
            e,
            "Failed to fetch aggregated results",
        ),
    }
}

async fn get_run_aggregated_latest(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<i32>,
) -> impl IntoResponse {
    match state.store.get_latest_aggregated_result(id).await {
        Ok(Some(result)) => (StatusCode::OK, Json(result)).into_response(),
        Ok(None) => json_error(StatusCode::NOT_FOUND, "No aggregated results"),
        Err(e) => internal_error(
            format!("fetching latest aggregated result for run {id}"),
            e,
            "Failed to fetch aggregated results",
        ),
    }
}

async fn get_run_evaluator_performance_history(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<i32>,
    Query(params): Query<PerformanceHistoryQuery>,
) -> impl IntoResponse {
    let limit = params.limit.clamp(1, 10_000);
    match state
        .store
        .get_evaluator_performance_history(id, limit, params.worker_id.as_deref())
        .await
    {
        Ok(rows) => (StatusCode::OK, Json(rows)).into_response(),
        Err(e) => internal_error(
            format!("fetching evaluator performance history for run {id}"),
            e,
            "Failed to fetch evaluator performance history",
        ),
    }
}

async fn get_run_sampler_performance_history(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<i32>,
    Query(params): Query<PerformanceHistoryQuery>,
) -> impl IntoResponse {
    let limit = params.limit.clamp(1, 10_000);
    match state
        .store
        .get_sampler_performance_history(id, limit, params.worker_id.as_deref())
        .await
    {
        Ok(rows) => (StatusCode::OK, Json(rows)).into_response(),
        Err(e) => internal_error(
            format!("fetching sampler performance history for run {id}"),
            e,
            "Failed to fetch sampler performance history",
        ),
    }
}

async fn stream_run_stats(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<i32>,
    Query(params): Query<StreamQuery>,
) -> impl IntoResponse {
    match state.store.get_run_progress(id).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            return json_error(StatusCode::NOT_FOUND, "Run not found");
        }
        Err(e) => {
            return internal_error(
                format!("validating run {id} for stream"),
                e,
                "Failed to initialize stream",
            );
        }
    }

    let interval_ms = sanitize_stream_interval_ms(params.interval_ms);
    let mut run_rx = subscribe_run_stream(&state, id, interval_ms).await;

    let (tx, rx) = mpsc::channel(16);
    tokio::spawn(async move {
        loop {
            let event = match run_rx.recv().await {
                Ok(StreamMessage::Stats(payload)) => Event::default().event("stats").data(payload),
                Ok(StreamMessage::Error(payload)) => Event::default().event("error").data(payload),
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    Event::default().event("error").data(
                        serde_json::json!({
                            "error": "Stream lagged",
                            "details": format!("skipped {skipped} updates")
                        })
                        .to_string(),
                    )
                }
                Err(broadcast::error::RecvError::Closed) => break,
            };

            if tx.send(Ok(event)).await.is_err() {
                break;
            }
        }
    });

    Sse::new(MpscStream { receiver: rx })
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(10))
                .text("keep-alive"),
        )
        .into_response()
}
