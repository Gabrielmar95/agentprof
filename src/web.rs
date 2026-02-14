use crate::logger::HookLogEntry;
use axum::http::header;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{Html, IntoResponse};
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;
use std::collections::HashMap;
use std::convert::Infallible;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;
use tokio::sync::{watch, RwLock};
use tokio_stream::wrappers::WatchStream;
use tokio_stream::StreamExt;

static WEB_HTML: &str = include_str!("web.html");

struct AppState {
    data: RwLock<ApiResponse>,
    version_tx: watch::Sender<u64>,
}

#[derive(Serialize, Clone)]
struct ApiResponse {
    format: String,
    session: SessionInfo,
    summary: SummaryInfo,
    tool_calls: Vec<ToolCall>,
    method_stats: HashMap<String, MethodStatsJson>,
}

#[derive(Serialize, Clone)]
struct SessionInfo {
    start: String,
    end: String,
    duration_secs: f64,
}

#[derive(Serialize, Clone)]
struct SummaryInfo {
    total_events: usize,
    matched_calls: usize,
    unmatched_pre: usize,
    unmatched_post: usize,
}

#[derive(Serialize, Clone)]
struct ToolCall {
    tool_name: String,
    tool_use_id: String,
    start_time: String,
    end_time: String,
    latency_ms: f64,
    tool_input: Option<serde_json::Value>,
    tool_response: Option<serde_json::Value>,
    session_id: Option<String>,
    input_tokens: Option<usize>,
    output_tokens: Option<usize>,
}

#[derive(Serialize, Clone)]
struct MethodStatsJson {
    calls: usize,
    errors: usize,
    sum_ms: f64,
    avg_ms: f64,
    p50_ms: f64,
    p95_ms: f64,
    p99_ms: f64,
    max_ms: f64,
    sum_tokens: usize,
}

fn parse_jsonl(log_path: &Path) -> Result<ApiResponse, Box<dyn std::error::Error + Send + Sync>> {
    let file = std::fs::File::open(log_path)?;
    let reader = BufReader::new(file);

    let mut entries: Vec<HookLogEntry> = Vec::new();
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<HookLogEntry>(&line) {
            Ok(entry) => entries.push(entry),
            Err(e) => {
                eprintln!("Warning: skipping malformed log line: {}", e);
            }
        }
    }

    if entries.is_empty() {
        return Ok(ApiResponse {
            format: "hooks".to_string(),
            session: SessionInfo {
                start: String::new(),
                end: String::new(),
                duration_secs: 0.0,
            },
            summary: SummaryInfo {
                total_events: 0,
                matched_calls: 0,
                unmatched_pre: 0,
                unmatched_post: 0,
            },
            tool_calls: Vec::new(),
            method_stats: HashMap::new(),
        });
    }

    // Index PreToolUse entries by tool_use_id
    let mut pre_events: HashMap<String, &HookLogEntry> = HashMap::new();
    for entry in &entries {
        if entry.event == "PreToolUse" {
            pre_events.insert(entry.tool_use_id.clone(), entry);
        }
    }

    // Match PostToolUse with PreToolUse
    let mut tool_calls: Vec<ToolCall> = Vec::new();
    let mut matched_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut unmatched_post: usize = 0;

    for entry in &entries {
        if entry.event == "PostToolUse" {
            if let Some(pre) = pre_events.get(&entry.tool_use_id) {
                let latency_ms = (entry.timestamp - pre.timestamp).num_milliseconds() as f64;
                tool_calls.push(ToolCall {
                    tool_name: entry.tool_name.clone(),
                    tool_use_id: entry.tool_use_id.clone(),
                    start_time: pre.timestamp.to_rfc3339(),
                    end_time: entry.timestamp.to_rfc3339(),
                    latency_ms,
                    tool_input: pre.tool_input.clone(),
                    tool_response: entry.tool_response.clone(),
                    session_id: entry.session_id.clone(),
                    input_tokens: pre.input_tokens,
                    output_tokens: entry.output_tokens,
                });
                matched_ids.insert(entry.tool_use_id.clone());
            } else {
                unmatched_post += 1;
            }
        }
    }

    // Count unmatched PreToolUse
    let unmatched_pre = entries
        .iter()
        .filter(|e| e.event == "PreToolUse" && !matched_ids.contains(&e.tool_use_id))
        .count();

    // Compute method stats
    struct MethodAccum {
        latencies: Vec<f64>,
        sum_tokens: usize,
    }
    let mut accum_map: HashMap<String, MethodAccum> = HashMap::new();
    for call in &tool_calls {
        let accum = accum_map
            .entry(call.tool_name.clone())
            .or_insert_with(|| MethodAccum {
                latencies: Vec::new(),
                sum_tokens: 0,
            });
        accum.latencies.push(call.latency_ms);
        accum.sum_tokens += call.input_tokens.unwrap_or(0) + call.output_tokens.unwrap_or(0);
    }

    let mut method_stats: HashMap<String, MethodStatsJson> = HashMap::new();
    for (name, mut accum) in accum_map {
        accum
            .latencies
            .sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let calls = accum.latencies.len();
        let sum_ms = accum.latencies.iter().sum::<f64>();
        let avg_ms = sum_ms / calls as f64;
        let p50_ms = percentile(&accum.latencies, 50.0);
        let p95_ms = percentile(&accum.latencies, 95.0);
        let p99_ms = percentile(&accum.latencies, 99.0);
        let max_ms = accum.latencies.last().copied().unwrap_or(0.0);
        method_stats.insert(
            name,
            MethodStatsJson {
                calls,
                errors: 0,
                sum_ms,
                avg_ms,
                p50_ms,
                p95_ms,
                p99_ms,
                max_ms,
                sum_tokens: accum.sum_tokens,
            },
        );
    }

    // Session info
    let first_ts = entries.first().unwrap().timestamp;
    let last_ts = entries.last().unwrap().timestamp;
    let duration = last_ts - first_ts;

    Ok(ApiResponse {
        format: "hooks".to_string(),
        session: SessionInfo {
            start: first_ts.to_rfc3339(),
            end: last_ts.to_rfc3339(),
            duration_secs: duration.num_milliseconds() as f64 / 1000.0,
        },
        summary: SummaryInfo {
            total_events: entries.len(),
            matched_calls: tool_calls.len(),
            unmatched_pre,
            unmatched_post,
        },
        tool_calls,
        method_stats,
    })
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = (p / 100.0 * (sorted.len() - 1) as f64).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

async fn serve_html() -> Html<&'static str> {
    Html(WEB_HTML)
}

async fn serve_data(state: axum::extract::State<Arc<AppState>>) -> impl IntoResponse {
    let data = state.data.read().await;
    (
        [(header::CONTENT_TYPE, "application/json")],
        Json(data.clone()),
    )
}

async fn serve_events(
    state: axum::extract::State<Arc<AppState>>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let rx = state.version_tx.subscribe();
    let stream = WatchStream::new(rx)
        .map(|version| Ok(Event::default().event("refresh").data(version.to_string())));
    Sse::new(stream).keep_alive(KeepAlive::default())
}

async fn watch_file(log_path: PathBuf, state: Arc<AppState>) {
    let mut last_mtime: Option<SystemTime> = None;

    loop {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        let current_mtime = match std::fs::metadata(&log_path) {
            Ok(meta) => meta.modified().ok(),
            Err(_) => continue,
        };

        if current_mtime == last_mtime {
            continue;
        }
        last_mtime = current_mtime;

        let path = log_path.clone();
        let result = tokio::task::spawn_blocking(move || parse_jsonl(&path)).await;

        match result {
            Ok(Ok(new_data)) => {
                eprintln!(
                    "agentprof: file changed, reloaded {} events, {} matched calls",
                    new_data.summary.total_events, new_data.summary.matched_calls
                );
                let mut data = state.data.write().await;
                *data = new_data;
                drop(data);
                state.version_tx.send_modify(|v| *v += 1);
            }
            Ok(Err(e)) => {
                eprintln!("agentprof: failed to re-parse file: {}", e);
            }
            Err(e) => {
                eprintln!("agentprof: spawn_blocking error: {}", e);
            }
        }
    }
}

pub async fn run_server(log_path: &Path, port: u16) -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("agentprof: parsing {}...", log_path.display());
    let api_data = parse_jsonl(log_path).map_err(|e| -> Box<dyn std::error::Error> { e })?;
    eprintln!(
        "agentprof: loaded {} events, {} matched tool calls",
        api_data.summary.total_events, api_data.summary.matched_calls
    );

    let (version_tx, _) = watch::channel(0u64);
    let shared = Arc::new(AppState {
        data: RwLock::new(api_data),
        version_tx,
    });

    let watcher_state = Arc::clone(&shared);
    let watcher_path = log_path.to_path_buf();
    tokio::spawn(async move {
        watch_file(watcher_path, watcher_state).await;
    });

    let app = Router::new()
        .route("/", get(serve_html))
        .route("/api/data", get(serve_data))
        .route("/api/events", get(serve_events))
        .with_state(shared);

    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    eprintln!("agentprof: web UI at http://localhost:{}", port);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
