//! Shared operator observability: tracing init and Prometheus-style metrics.

use std::sync::LazyLock;
use std::time::Instant;

use prometheus::{
    Encoder, HistogramOpts, HistogramVec, IntCounter, IntCounterVec, IntGaugeVec, Opts, Registry,
    TextEncoder,
};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::format::FmtSpan;

/// Env: `HIVEMIND_LOG_FORMAT=json` (or `json=1`/`true`) enables JSON logs.
/// Filter: standard `RUST_LOG` (default `info`).
pub fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let json = log_format_json();
    let result = if json {
        tracing_subscriber::fmt()
            .json()
            .with_env_filter(filter)
            .with_span_events(FmtSpan::NONE)
            .with_writer(std::io::stderr)
            .with_current_span(true)
            .with_span_list(false)
            .try_init()
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .try_init()
    };
    let _ = result;
}

fn log_format_json() -> bool {
    match std::env::var("HIVEMIND_LOG_FORMAT") {
        Ok(v) => {
            let v = v.trim().to_ascii_lowercase();
            matches!(v.as_str(), "json" | "1" | "true" | "yes")
        }
        Err(_) => false,
    }
}

static REGISTRY: LazyLock<Registry> = LazyLock::new(Registry::new);

static HTTP_REQUESTS: LazyLock<IntCounterVec> = LazyLock::new(|| {
    let c = IntCounterVec::new(
        Opts::new(
            "hivemind_http_requests_total",
            "HTTP requests handled by the shared service",
        ),
        &["method", "path", "status"],
    )
    .expect("metric");
    REGISTRY.register(Box::new(c.clone())).expect("register");
    c
});

static HTTP_DURATION: LazyLock<HistogramVec> = LazyLock::new(|| {
    let h = HistogramVec::new(
        HistogramOpts::new(
            "hivemind_http_request_duration_seconds",
            "HTTP request latency in seconds",
        )
        .buckets(vec![
            0.001, 0.0025, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0,
        ]),
        &["method", "path"],
    )
    .expect("metric");
    REGISTRY.register(Box::new(h.clone())).expect("register");
    h
});

static REMEMBER: LazyLock<IntCounter> = LazyLock::new(|| {
    let c = IntCounter::new(
        "hivemind_remember_total",
        "Successful remember / append operations",
    )
    .expect("metric");
    REGISTRY.register(Box::new(c.clone())).expect("register");
    c
});

static RECALL: LazyLock<IntCounter> = LazyLock::new(|| {
    let c = IntCounter::new("hivemind_recall_total", "Recall / search operations").expect("metric");
    REGISTRY.register(Box::new(c.clone())).expect("register");
    c
});

static LOG_OFFSET: LazyLock<IntGaugeVec> = LazyLock::new(|| {
    let g = IntGaugeVec::new(
        Opts::new(
            "hivemind_log_offset",
            "Next append offset for an open tenant store",
        ),
        &["tenant"],
    )
    .expect("metric");
    REGISTRY.register(Box::new(g.clone())).expect("register");
    g
});

/// Force registration of all metrics so `/metrics` is never empty after first call.
pub fn ensure_registered() {
    LazyLock::force(&HTTP_REQUESTS);
    LazyLock::force(&HTTP_DURATION);
    LazyLock::force(&REMEMBER);
    LazyLock::force(&RECALL);
    LazyLock::force(&LOG_OFFSET);
}

pub fn record_http(method: &str, path: &str, status: u16, started: Instant) {
    ensure_registered();
    let status = status.to_string();
    HTTP_REQUESTS
        .with_label_values(&[method, path, &status])
        .inc();
    HTTP_DURATION
        .with_label_values(&[method, path])
        .observe(started.elapsed().as_secs_f64());
}

pub fn record_remember() {
    ensure_registered();
    REMEMBER.inc();
}

pub fn record_recall() {
    ensure_registered();
    RECALL.inc();
}

pub fn set_log_offset(tenant: &str, offset: u64) {
    ensure_registered();
    LOG_OFFSET
        .with_label_values(&[tenant])
        .set(i64::try_from(offset).unwrap_or(i64::MAX));
}

/// Prometheus text exposition of registered metrics.
pub fn render_prometheus() -> String {
    ensure_registered();
    let metric_families = REGISTRY.gather();
    let mut buffer = Vec::new();
    let encoder = TextEncoder::new();
    encoder
        .encode(&metric_families, &mut buffer)
        .expect("encode prometheus");
    String::from_utf8(buffer).unwrap_or_default()
}

/// Normalize a request path for metric cardinality (collapse tenant ids).
pub fn normalize_path(path: &str) -> String {
    let mut parts: Vec<&str> = path.split('/').collect();
    // /v1/tenants/{tenant}/...
    if parts.len() >= 4 && parts[1] == "v1" && parts[2] == "tenants" {
        parts[3] = ":tenant";
    }
    if parts.is_empty() || (parts.len() == 1 && parts[0].is_empty()) {
        return "/".to_owned();
    }
    let joined = parts.join("/");
    if joined.is_empty() {
        "/".to_owned()
    } else if joined.starts_with('/') {
        joined
    } else {
        format!("/{joined}")
    }
}
