//! In-process fake OTLP collector (task T015 / research item R5).
//!
//! Listens on `127.0.0.1:0` (ephemeral port) and accepts OTLP/gRPC
//! exports for traces, metrics, and logs. Each received payload is
//! stored in a shared `Mutex<Vec<...>>` so tests can assert on what was
//! exported. Failure injection is supported (refuse N requests, drop
//! the connection) so the fail-open semantics of the plugin (FR-014)
//! can be tested deterministically.
//!
//! HTTP transport (`http/protobuf`, `http/json`) is supported via an
//! `axum` handler on a separate ephemeral port. Both endpoints share the
//! same `Sink`.

use axum::{
    Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::post,
};
use opentelemetry_proto::tonic::collector::logs::v1::logs_service_server::{
    LogsService, LogsServiceServer,
};
use opentelemetry_proto::tonic::collector::logs::v1::{
    ExportLogsServiceRequest, ExportLogsServiceResponse,
};
use opentelemetry_proto::tonic::collector::metrics::v1::metrics_service_server::{
    MetricsService, MetricsServiceServer,
};
use opentelemetry_proto::tonic::collector::metrics::v1::{
    ExportMetricsServiceRequest, ExportMetricsServiceResponse,
};
use opentelemetry_proto::tonic::collector::trace::v1::trace_service_server::{
    TraceService, TraceServiceServer,
};
use opentelemetry_proto::tonic::collector::trace::v1::{
    ExportTraceServiceRequest, ExportTraceServiceResponse,
};
use prost::Message;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tonic::{Request, Response, Status, transport::Server};

/// Shared sink — what each export call appends to. Tests `lock()` it and
/// assert on its contents.
#[derive(Default)]
pub struct Sink {
    pub trace_exports: Vec<ExportTraceServiceRequest>,
    pub metric_exports: Vec<ExportMetricsServiceRequest>,
    pub log_exports: Vec<ExportLogsServiceRequest>,
}

#[derive(Default)]
pub struct FailureInjector {
    /// If non-zero, the next N requests of any kind return `Internal`.
    pub fail_next_n: AtomicU32,
}

#[derive(Clone)]
pub struct CollectorHandle {
    pub sink: Arc<Mutex<Sink>>,
    pub failures: Arc<FailureInjector>,
    pub grpc_addr: SocketAddr,
    pub http_addr: SocketAddr,
    shutdown: Arc<tokio::sync::Notify>,
}

impl CollectorHandle {
    /// gRPC URL (e.g. `http://127.0.0.1:42313`) for use as
    /// `[plugins.params.tracer_pipelines.exporter] endpoint = "..."`.
    pub fn grpc_url(&self) -> String {
        format!("http://{}", self.grpc_addr)
    }

    /// HTTP URL (e.g. `http://127.0.0.1:42314`) for OTLP/HTTP exporters.
    /// Append `/v1/traces`, `/v1/metrics`, `/v1/logs` per signal.
    pub fn http_url(&self) -> String {
        format!("http://{}", self.http_addr)
    }

    /// Inject N consecutive failures.
    pub fn inject_failures(&self, n: u32) {
        self.failures.fail_next_n.store(n, Ordering::Relaxed);
    }

    /// Total spans seen across all batches.
    pub async fn total_spans(&self) -> usize {
        let s = self.sink.lock().await;
        s.trace_exports
            .iter()
            .flat_map(|e| &e.resource_spans)
            .flat_map(|rs| &rs.scope_spans)
            .map(|ss| ss.spans.len())
            .sum()
    }

    /// Total metric data points seen.
    pub async fn total_metrics(&self) -> usize {
        let s = self.sink.lock().await;
        s.metric_exports
            .iter()
            .flat_map(|e| &e.resource_metrics)
            .flat_map(|rm| &rm.scope_metrics)
            .map(|sm| sm.metrics.len())
            .sum()
    }

    /// Total log records seen.
    pub async fn total_logs(&self) -> usize {
        let s = self.sink.lock().await;
        s.log_exports
            .iter()
            .flat_map(|e| &e.resource_logs)
            .flat_map(|rl| &rl.scope_logs)
            .map(|sl| sl.log_records.len())
            .sum()
    }

    /// Stop the collector. The underlying tonic server completes any
    /// in-flight RPC then shuts down.
    pub fn shutdown(&self) {
        self.shutdown.notify_waiters();
    }
}

/// Spin up the fake collector and return a handle. Both gRPC and HTTP
/// receivers run on ephemeral ports; the same `Sink` records exports
/// from either transport.
pub async fn start() -> CollectorHandle {
    let sink = Arc::new(Mutex::new(Sink::default()));
    let failures = Arc::new(FailureInjector::default());
    let shutdown = Arc::new(tokio::sync::Notify::new());

    // ---- gRPC ----
    let grpc_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let grpc_addr = grpc_listener.local_addr().unwrap();
    let trace = TraceImpl {
        sink: sink.clone(),
        failures: failures.clone(),
    };
    let metrics = MetricsImpl {
        sink: sink.clone(),
        failures: failures.clone(),
    };
    let logs = LogsImpl {
        sink: sink.clone(),
        failures: failures.clone(),
    };
    let grpc_shutdown = shutdown.clone();
    tokio::spawn(async move {
        let incoming = tokio_stream::wrappers::TcpListenerStream::new(grpc_listener);
        Server::builder()
            .add_service(TraceServiceServer::new(trace))
            .add_service(MetricsServiceServer::new(metrics))
            .add_service(LogsServiceServer::new(logs))
            .serve_with_incoming_shutdown(incoming, async move {
                grpc_shutdown.notified().await;
            })
            .await
            .ok();
    });

    // ---- HTTP (axum) ----
    let http_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let http_addr = http_listener.local_addr().unwrap();
    let http_state = HttpState {
        sink: sink.clone(),
        failures: failures.clone(),
    };
    let app = Router::new()
        .route("/v1/traces", post(http_traces))
        .route("/v1/metrics", post(http_metrics))
        .route("/v1/logs", post(http_logs))
        .with_state(http_state);
    let http_shutdown = shutdown.clone();
    tokio::spawn(async move {
        axum::serve(http_listener, app)
            .with_graceful_shutdown(async move {
                http_shutdown.notified().await;
            })
            .await
            .ok();
    });

    // Let the listeners bind before tests connect.
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;

    CollectorHandle {
        sink,
        failures,
        grpc_addr,
        http_addr,
        shutdown,
    }
}

#[derive(Clone)]
struct HttpState {
    sink: Arc<Mutex<Sink>>,
    failures: Arc<FailureInjector>,
}

fn maybe_fail_status(failures: &FailureInjector) -> Option<StatusCode> {
    let prev = failures.fail_next_n.load(Ordering::Relaxed);
    if prev == 0 {
        return None;
    }
    failures
        .fail_next_n
        .store(prev.saturating_sub(1), Ordering::Relaxed);
    Some(StatusCode::INTERNAL_SERVER_ERROR)
}

fn is_json(headers: &HeaderMap) -> bool {
    headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|s| s.contains("application/json"))
}

async fn http_traces(
    State(state): State<HttpState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> StatusCode {
    if let Some(code) = maybe_fail_status(&state.failures) {
        return code;
    }
    let req: ExportTraceServiceRequest = if is_json(&headers) {
        match serde_json::from_slice(&body) {
            Ok(r) => r,
            Err(_) => return StatusCode::BAD_REQUEST,
        }
    } else {
        match ExportTraceServiceRequest::decode(body.as_ref()) {
            Ok(r) => r,
            Err(_) => return StatusCode::BAD_REQUEST,
        }
    };
    state.sink.lock().await.trace_exports.push(req);
    StatusCode::OK
}

async fn http_metrics(
    State(state): State<HttpState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> StatusCode {
    if let Some(code) = maybe_fail_status(&state.failures) {
        return code;
    }
    let req: ExportMetricsServiceRequest = if is_json(&headers) {
        match serde_json::from_slice(&body) {
            Ok(r) => r,
            Err(_) => return StatusCode::BAD_REQUEST,
        }
    } else {
        match ExportMetricsServiceRequest::decode(body.as_ref()) {
            Ok(r) => r,
            Err(_) => return StatusCode::BAD_REQUEST,
        }
    };
    state.sink.lock().await.metric_exports.push(req);
    StatusCode::OK
}

async fn http_logs(
    State(state): State<HttpState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> StatusCode {
    if let Some(code) = maybe_fail_status(&state.failures) {
        return code;
    }
    let req: ExportLogsServiceRequest = if is_json(&headers) {
        match serde_json::from_slice(&body) {
            Ok(r) => r,
            Err(_) => return StatusCode::BAD_REQUEST,
        }
    } else {
        match ExportLogsServiceRequest::decode(body.as_ref()) {
            Ok(r) => r,
            Err(_) => return StatusCode::BAD_REQUEST,
        }
    };
    state.sink.lock().await.log_exports.push(req);
    StatusCode::OK
}

fn maybe_fail<T>(failures: &FailureInjector) -> Option<Result<T, Status>> {
    let prev = failures.fail_next_n.load(Ordering::Relaxed);
    if prev == 0 {
        return None;
    }
    failures
        .fail_next_n
        .store(prev.saturating_sub(1), Ordering::Relaxed);
    Some(Err(Status::internal("injected failure")))
}

struct TraceImpl {
    sink: Arc<Mutex<Sink>>,
    failures: Arc<FailureInjector>,
}

#[tonic::async_trait]
impl TraceService for TraceImpl {
    async fn export(
        &self,
        request: Request<ExportTraceServiceRequest>,
    ) -> Result<Response<ExportTraceServiceResponse>, Status> {
        if let Some(r) = maybe_fail(&self.failures) {
            return r.map(Response::new);
        }
        self.sink
            .lock()
            .await
            .trace_exports
            .push(request.into_inner());
        Ok(Response::new(ExportTraceServiceResponse::default()))
    }
}

struct MetricsImpl {
    sink: Arc<Mutex<Sink>>,
    failures: Arc<FailureInjector>,
}

#[tonic::async_trait]
impl MetricsService for MetricsImpl {
    async fn export(
        &self,
        request: Request<ExportMetricsServiceRequest>,
    ) -> Result<Response<ExportMetricsServiceResponse>, Status> {
        if let Some(r) = maybe_fail(&self.failures) {
            return r.map(Response::new);
        }
        self.sink
            .lock()
            .await
            .metric_exports
            .push(request.into_inner());
        Ok(Response::new(ExportMetricsServiceResponse::default()))
    }
}

struct LogsImpl {
    sink: Arc<Mutex<Sink>>,
    failures: Arc<FailureInjector>,
}

#[tonic::async_trait]
impl LogsService for LogsImpl {
    async fn export(
        &self,
        request: Request<ExportLogsServiceRequest>,
    ) -> Result<Response<ExportLogsServiceResponse>, Status> {
        if let Some(r) = maybe_fail(&self.failures) {
            return r.map(Response::new);
        }
        self.sink
            .lock()
            .await
            .log_exports
            .push(request.into_inner());
        Ok(Response::new(ExportLogsServiceResponse::default()))
    }
}
