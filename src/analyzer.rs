use crate::logger::HookLogEntry;
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::Path;

struct MethodStats {
    calls: usize,
    errors: usize,
    latencies: Vec<f64>,
    input_tokens: usize,
    output_tokens: usize,
    /// tool_name → latencies for tools/call sub-breakdown
    tool_stats: HashMap<String, ToolStats>,
}

struct ToolStats {
    calls: usize,
    errors: usize,
    latencies: Vec<f64>,
    input_tokens: usize,
    output_tokens: usize,
}

struct SlowestCall {
    latency_ms: f64,
    method: String,
    tool_name: Option<String>,
    start_time: String,
    end_time: String,
}

#[cfg(test)]
#[derive(Debug)]
pub struct AnalysisResult {
    pub total_events: usize,
    pub pre_count: usize,
    pub post_count: usize,
    pub matched_count: usize,
    pub unmatched_pre: usize,
    pub unmatched_post: usize,
    pub duration_secs: i64,
    pub tools: HashMap<String, ToolResult>,
}

#[cfg(test)]
#[derive(Debug)]
pub struct ToolResult {
    pub calls: usize,
    pub latencies: Vec<f64>,
    pub input_tokens: usize,
    pub output_tokens: usize,
}

pub fn analyze(log_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let file = std::fs::File::open(log_path)?;
    let reader = BufReader::new(file);

    let mut lines: Vec<String> = Vec::new();
    for line in reader.lines() {
        let line = line?;
        if !line.trim().is_empty() {
            lines.push(line);
        }
    }

    if lines.is_empty() {
        println!("No log entries found.");
        return Ok(());
    }

    analyze_hooks(&lines)
}

fn parse_entries(lines: &[String]) -> Vec<HookLogEntry> {
    let mut entries = Vec::new();
    for line in lines {
        match serde_json::from_str::<HookLogEntry>(line) {
            Ok(entry) => entries.push(entry),
            Err(e) => {
                eprintln!("Warning: skipping malformed log line: {}", e);
            }
        }
    }
    entries
}

fn analyze_hooks(lines: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let entries = parse_entries(lines);

    if entries.is_empty() {
        println!("No log entries found.");
        return Ok(());
    }

    // Index PreToolUse entries by tool_use_id
    let mut pre_events: HashMap<String, &HookLogEntry> = HashMap::new();
    for entry in &entries {
        if entry.event == "PreToolUse" {
            pre_events.insert(entry.tool_use_id.clone(), entry);
        }
    }

    // Match PostToolUse with PreToolUse, compute latencies
    let mut method_stats: HashMap<String, MethodStats> = HashMap::new();
    let mut slowest: Vec<SlowestCall> = Vec::new();
    let mut matched_count: usize = 0;
    let mut unmatched_pre: usize = 0;
    let mut unmatched_post: usize = 0;
    let mut matched_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

    for entry in &entries {
        if entry.event == "PostToolUse" {
            if let Some(pre) = pre_events.get(&entry.tool_use_id) {
                let latency_ms = (entry.timestamp - pre.timestamp).num_milliseconds() as f64;
                let tool_name = entry.tool_name.clone();

                let input_tok = pre.input_tokens.unwrap_or(0);
                let output_tok = entry.output_tokens.unwrap_or(0);

                // All hook calls are tool calls, so we use tool_name directly as the method key
                let stats = method_stats
                    .entry(tool_name.clone())
                    .or_insert_with(|| MethodStats {
                        calls: 0,
                        errors: 0,
                        latencies: Vec::new(),
                        input_tokens: 0,
                        output_tokens: 0,
                        tool_stats: HashMap::new(),
                    });
                stats.calls += 1;
                stats.latencies.push(latency_ms);
                stats.input_tokens += input_tok;
                stats.output_tokens += output_tok;

                slowest.push(SlowestCall {
                    latency_ms,
                    method: tool_name.clone(),
                    tool_name: Some(tool_name),
                    start_time: format_time_short(pre.timestamp),
                    end_time: format_time_short(entry.timestamp),
                });

                matched_count += 1;
                matched_ids.insert(entry.tool_use_id.clone());
            } else {
                unmatched_post += 1;
            }
        }
    }

    // Count unmatched PreToolUse
    for entry in &entries {
        if entry.event == "PreToolUse" && !matched_ids.contains(&entry.tool_use_id) {
            unmatched_pre += 1;
        }
    }

    // Session time range
    let first_ts = entries.first().unwrap().timestamp;
    let last_ts = entries.last().unwrap().timestamp;
    let duration = last_ts - first_ts;
    let duration_secs = duration.num_seconds();
    let dur_min = duration_secs / 60;
    let dur_sec = duration_secs % 60;

    // Count events
    let pre_count = entries.iter().filter(|e| e.event == "PreToolUse").count();
    let post_count = entries.iter().filter(|e| e.event == "PostToolUse").count();

    // Sort slowest
    slowest.sort_by(|a, b| {
        b.latency_ms
            .partial_cmp(&a.latency_ms)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Print report
    println!();
    println!("\u{2550}\u{2550}\u{2550} Agent Profiler Report \u{2550}\u{2550}\u{2550}");
    println!();
    println!(
        "Session: {} \u{2192} {} ({}m {}s)",
        first_ts.format("%Y-%m-%d %H:%M:%S"),
        last_ts.format("%H:%M:%S"),
        dur_min,
        dur_sec
    );
    println!(
        "Total events: {} ({} PreToolUse, {} PostToolUse)",
        entries.len(),
        pre_count,
        post_count
    );
    println!("Matched tool calls: {}", matched_count);
    if unmatched_pre > 0 || unmatched_post > 0 {
        println!(
            "Unmatched events: {} PreToolUse, {} PostToolUse",
            unmatched_pre, unmatched_post
        );
    }

    print_method_breakdown(&method_stats);
    print_slowest(&slowest);
    print_token_usage(&method_stats);

    Ok(())
}

fn print_method_breakdown(method_stats: &HashMap<String, MethodStats>) {
    println!();
    println!("\u{2500}\u{2500} Method Breakdown \u{2500}\u{2500}");
    println!(
        "{:<20} {:>5}  {:>6}  {:>8}  {:>8}  {:>8}  {:>8}  {:>8}",
        "Method", "Calls", "Errors", "Avg(ms)", "P50(ms)", "P95(ms)", "P99(ms)", "Max(ms)"
    );

    let mut methods: Vec<_> = method_stats.iter().collect();
    methods.sort_by(|a, b| a.0.cmp(b.0));

    for (method, stats) in &methods {
        let mut lats = stats.latencies.clone();
        lats.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        println!(
            "{:<20} {:>5}  {:>6}  {:>8.1}  {:>8.1}  {:>8.1}  {:>8.1}  {:>8.1}",
            method,
            stats.calls,
            stats.errors,
            avg(&lats),
            percentile(&lats, 50.0),
            percentile(&lats, 95.0),
            percentile(&lats, 99.0),
            lats.last().copied().unwrap_or(0.0),
        );

        // Sub-breakdown for tools
        if !stats.tool_stats.is_empty() {
            let mut tools: Vec<_> = stats.tool_stats.iter().collect();
            tools.sort_by(|a, b| a.0.cmp(b.0));
            for (i, (tool, ts)) in tools.iter().enumerate() {
                let prefix = if i == tools.len() - 1 {
                    "\u{2514}\u{2500}"
                } else {
                    "\u{251c}\u{2500}"
                };
                let mut tlats = ts.latencies.clone();
                tlats.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                println!(
                    "  {} {:<16} {:>5}  {:>6}  {:>8.1}  {:>8.1}  {:>8.1}  {:>8.1}  {:>8.1}",
                    prefix,
                    tool,
                    ts.calls,
                    ts.errors,
                    avg(&tlats),
                    percentile(&tlats, 50.0),
                    percentile(&tlats, 95.0),
                    percentile(&tlats, 99.0),
                    tlats.last().copied().unwrap_or(0.0),
                );
            }
        }
    }
}

fn print_slowest(slowest: &[SlowestCall]) {
    println!();
    println!("\u{2500}\u{2500} Timeline (top 5 slowest) \u{2500}\u{2500}");
    for (i, call) in slowest.iter().take(5).enumerate() {
        let tool_suffix = if let Some(ref t) = call.tool_name {
            format!("({})", t)
        } else {
            String::new()
        };
        println!(
            "#{:<2} {:>7.0}ms  {}{:<20} [{} \u{2192} {}]",
            i + 1,
            call.latency_ms,
            call.method,
            tool_suffix,
            call.start_time,
            call.end_time,
        );
    }
    println!();
}

fn print_token_usage(method_stats: &HashMap<String, MethodStats>) {
    let total_input: usize = method_stats.values().map(|s| s.input_tokens).sum();
    let total_output: usize = method_stats.values().map(|s| s.output_tokens).sum();

    if total_input == 0 && total_output == 0 {
        return;
    }

    println!();
    println!("\u{2500}\u{2500} Token Usage (estimated) \u{2500}\u{2500}");
    println!(
        "{:<20} {:>5}  {:>9}  {:>9}  {:>9}",
        "Method", "Calls", "Input", "Output", "Total"
    );

    let mut methods: Vec<_> = method_stats.iter().collect();
    methods.sort_by(|a, b| {
        let total_b = b.1.input_tokens + b.1.output_tokens;
        let total_a = a.1.input_tokens + a.1.output_tokens;
        total_b.cmp(&total_a)
    });

    for (method, stats) in &methods {
        let total = stats.input_tokens + stats.output_tokens;
        if total == 0 {
            continue;
        }
        println!(
            "{:<20} {:>5}  {:>9}  {:>9}  {:>9}",
            method,
            stats.calls,
            format_tokens(stats.input_tokens),
            format_tokens(stats.output_tokens),
            format_tokens(total),
        );

        // Sub-breakdown for tools
        if !stats.tool_stats.is_empty() {
            let mut tools: Vec<_> = stats.tool_stats.iter().collect();
            tools.sort_by(|a, b| {
                let total_b = b.1.input_tokens + b.1.output_tokens;
                let total_a = a.1.input_tokens + a.1.output_tokens;
                total_b.cmp(&total_a)
            });
            for (i, (tool, ts)) in tools.iter().enumerate() {
                let prefix = if i == tools.len() - 1 {
                    "\u{2514}\u{2500}"
                } else {
                    "\u{251c}\u{2500}"
                };
                let tool_total = ts.input_tokens + ts.output_tokens;
                if tool_total == 0 {
                    continue;
                }
                println!(
                    "  {} {:<16} {:>5}  {:>9}  {:>9}  {:>9}",
                    prefix,
                    tool,
                    ts.calls,
                    format_tokens(ts.input_tokens),
                    format_tokens(ts.output_tokens),
                    format_tokens(tool_total),
                );
            }
        }
    }

    println!(
        "{:<20} {:>5}  {:>9}  {:>9}  {:>9}",
        "TOTAL",
        "",
        format_tokens(total_input),
        format_tokens(total_output),
        format_tokens(total_input + total_output),
    );
}

fn format_tokens(tokens: usize) -> String {
    if tokens < 1000 {
        format!("{}", tokens)
    } else if tokens < 1_000_000 {
        format!("{:.1}K", tokens as f64 / 1000.0)
    } else {
        format!("{:.1}M", tokens as f64 / 1_000_000.0)
    }
}

fn avg(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.iter().sum::<f64>() / values.len() as f64
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = (p / 100.0 * (sorted.len() - 1) as f64).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn format_time_short(ts: chrono::DateTime<chrono::Utc>) -> String {
    ts.format("%H:%M:%S").to_string()
}

#[cfg(test)]
fn compute_analysis(entries: &[HookLogEntry]) -> AnalysisResult {
    let mut pre_events: HashMap<String, &HookLogEntry> = HashMap::new();
    for entry in entries {
        if entry.event == "PreToolUse" {
            pre_events.insert(entry.tool_use_id.clone(), entry);
        }
    }

    let mut tools: HashMap<String, ToolResult> = HashMap::new();
    let mut matched_count: usize = 0;
    let mut unmatched_post: usize = 0;
    let mut matched_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

    for entry in entries {
        if entry.event == "PostToolUse" {
            if let Some(pre) = pre_events.get(&entry.tool_use_id) {
                let latency_ms = (entry.timestamp - pre.timestamp).num_milliseconds() as f64;
                let input_tok = pre.input_tokens.unwrap_or(0);
                let output_tok = entry.output_tokens.unwrap_or(0);

                let tool = tools
                    .entry(entry.tool_name.clone())
                    .or_insert_with(|| ToolResult {
                        calls: 0,
                        latencies: Vec::new(),
                        input_tokens: 0,
                        output_tokens: 0,
                    });
                tool.calls += 1;
                tool.latencies.push(latency_ms);
                tool.input_tokens += input_tok;
                tool.output_tokens += output_tok;

                matched_count += 1;
                matched_ids.insert(entry.tool_use_id.clone());
            } else {
                unmatched_post += 1;
            }
        }
    }

    let mut unmatched_pre: usize = 0;
    for entry in entries {
        if entry.event == "PreToolUse" && !matched_ids.contains(&entry.tool_use_id) {
            unmatched_pre += 1;
        }
    }

    let first_ts = entries.first().unwrap().timestamp;
    let last_ts = entries.last().unwrap().timestamp;
    let duration_secs = (last_ts - first_ts).num_seconds();

    let pre_count = entries.iter().filter(|e| e.event == "PreToolUse").count();
    let post_count = entries.iter().filter(|e| e.event == "PostToolUse").count();

    AnalysisResult {
        total_events: entries.len(),
        pre_count,
        post_count,
        matched_count,
        unmatched_pre,
        unmatched_post,
        duration_secs,
        tools,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real log entries from claude-profiler-session-log.jsonl

    const GLOB_PRE: &str = r#"{"timestamp":"2026-02-14T09:08:17.466100Z","event":"PreToolUse","tool_name":"Glob","tool_use_id":"toolu_01RMuqyFzjzrdSFERRdZErYP","tool_input":{"pattern":".mcp.json"},"session_id":"a152e784-b187-4064-a76e-8190e7e22ae8","input_tokens":8}"#;
    const GLOB_POST: &str = r#"{"timestamp":"2026-02-14T09:08:17.796900Z","event":"PostToolUse","tool_name":"Glob","tool_use_id":"toolu_01RMuqyFzjzrdSFERRdZErYP","tool_input":{"pattern":".mcp.json"},"tool_response":{"durationMs":287,"filenames":[],"numFiles":0,"truncated":false},"session_id":"a152e784-b187-4064-a76e-8190e7e22ae8","input_tokens":8,"output_tokens":20}"#;

    const ECHO_PRE: &str = r#"{"timestamp":"2026-02-14T09:10:19.030141Z","event":"PreToolUse","tool_name":"mcp__server-everything__echo","tool_use_id":"toolu_01DsnwXnG2r61h4RCi4cDTir","tool_input":{"message":"Hello from server-everything!"},"session_id":"1ff300da-1aea-43b5-8582-fd4498ddd443","input_tokens":11}"#;
    const ECHO_POST: &str = r#"{"timestamp":"2026-02-14T09:10:21.070083Z","event":"PostToolUse","tool_name":"mcp__server-everything__echo","tool_use_id":"toolu_01DsnwXnG2r61h4RCi4cDTir","tool_input":{"message":"Hello from server-everything!"},"tool_response":[{"text":"Echo: Hello from server-everything!","type":"text"}],"session_id":"1ff300da-1aea-43b5-8582-fd4498ddd443","input_tokens":11,"output_tokens":19}"#;

    const CLICKHOUSE_PRE: &str = r#"{"timestamp":"2026-02-14T09:23:12.450714Z","event":"PreToolUse","tool_name":"mcp__mcp-clickhouse__list_databases","tool_use_id":"toolu_01LHz4hnXYfrbh8911E5kiTH","tool_input":{},"session_id":"a5edf725-55eb-406f-b0c2-c3bea401411c","input_tokens":1}"#;
    const CLICKHOUSE_POST: &str = r#"{"timestamp":"2026-02-14T09:23:14.123821Z","event":"PostToolUse","tool_name":"mcp__mcp-clickhouse__list_databases","tool_use_id":"toolu_01LHz4hnXYfrbh8911E5kiTH","tool_input":{},"tool_response":[{"text":"[\"INFORMATION_SCHEMA\", \"default\", \"information_schema\", \"system\"]","type":"text"}],"session_id":"a5edf725-55eb-406f-b0c2-c3bea401411c","input_tokens":1,"output_tokens":26}"#;

    #[test]
    fn parse_glob_pre_entry() {
        let entry: HookLogEntry = serde_json::from_str(GLOB_PRE).unwrap();
        assert_eq!(entry.event, "PreToolUse");
        assert_eq!(entry.tool_name, "Glob");
        assert_eq!(entry.tool_use_id, "toolu_01RMuqyFzjzrdSFERRdZErYP");
        assert_eq!(
            entry.session_id.as_deref(),
            Some("a152e784-b187-4064-a76e-8190e7e22ae8")
        );
        assert_eq!(entry.input_tokens, Some(8));
        assert!(entry.tool_input.is_some());
        assert_eq!(
            entry.tool_input.unwrap().get("pattern").unwrap().as_str(),
            Some(".mcp.json")
        );
        assert!(entry.tool_response.is_none());
    }

    #[test]
    fn parse_glob_post_entry() {
        let entry: HookLogEntry = serde_json::from_str(GLOB_POST).unwrap();
        assert_eq!(entry.event, "PostToolUse");
        assert_eq!(entry.tool_name, "Glob");
        assert_eq!(entry.tool_use_id, "toolu_01RMuqyFzjzrdSFERRdZErYP");
        assert_eq!(entry.output_tokens, Some(20));
        assert!(entry.tool_response.is_some());
        let resp = entry.tool_response.unwrap();
        assert_eq!(resp.get("numFiles").unwrap().as_u64(), Some(0));
    }

    #[test]
    fn parse_echo_pre_entry() {
        let entry: HookLogEntry = serde_json::from_str(ECHO_PRE).unwrap();
        assert_eq!(entry.event, "PreToolUse");
        assert_eq!(entry.tool_name, "mcp__server-everything__echo");
        assert_eq!(entry.tool_use_id, "toolu_01DsnwXnG2r61h4RCi4cDTir");
        assert_eq!(entry.input_tokens, Some(11));
        assert_eq!(
            entry
                .tool_input
                .unwrap()
                .get("message")
                .unwrap()
                .as_str(),
            Some("Hello from server-everything!")
        );
    }

    #[test]
    fn parse_clickhouse_pre_entry() {
        let entry: HookLogEntry = serde_json::from_str(CLICKHOUSE_PRE).unwrap();
        assert_eq!(entry.event, "PreToolUse");
        assert_eq!(entry.tool_name, "mcp__mcp-clickhouse__list_databases");
        assert_eq!(entry.input_tokens, Some(1));
        // Empty object tool_input
        assert!(entry.tool_input.unwrap().as_object().unwrap().is_empty());
    }

    #[test]
    fn parse_entries_skips_malformed_lines() {
        let lines = vec![
            GLOB_PRE.to_string(),
            "not valid json {{{".to_string(),
            GLOB_POST.to_string(),
        ];
        let entries = parse_entries(&lines);
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn match_glob_pre_post_pair() {
        let lines = vec![GLOB_PRE.to_string(), GLOB_POST.to_string()];
        let entries = parse_entries(&lines);
        let result = compute_analysis(&entries);

        assert_eq!(result.total_events, 2);
        assert_eq!(result.pre_count, 1);
        assert_eq!(result.post_count, 1);
        assert_eq!(result.matched_count, 1);
        assert_eq!(result.unmatched_pre, 0);
        assert_eq!(result.unmatched_post, 0);

        let glob_stats = result.tools.get("Glob").expect("Glob tool stats missing");
        assert_eq!(glob_stats.calls, 1);
        // 09:08:17.796900 - 09:08:17.466100 = 330.8ms
        assert!((glob_stats.latencies[0] - 330.0).abs() < 2.0);
        assert_eq!(glob_stats.input_tokens, 8);
        assert_eq!(glob_stats.output_tokens, 20);
    }

    #[test]
    fn match_echo_pre_post_pair() {
        let lines = vec![ECHO_PRE.to_string(), ECHO_POST.to_string()];
        let entries = parse_entries(&lines);
        let result = compute_analysis(&entries);

        assert_eq!(result.matched_count, 1);

        let echo_stats = result
            .tools
            .get("mcp__server-everything__echo")
            .expect("echo tool stats missing");
        assert_eq!(echo_stats.calls, 1);
        // 09:10:21.070083 - 09:10:19.030141 = 2039.942ms
        assert!((echo_stats.latencies[0] - 2039.0).abs() < 2.0);
        assert_eq!(echo_stats.input_tokens, 11);
        assert_eq!(echo_stats.output_tokens, 19);
    }

    #[test]
    fn match_multiple_tools() {
        let lines = vec![
            GLOB_PRE.to_string(),
            ECHO_PRE.to_string(),
            GLOB_POST.to_string(),
            ECHO_POST.to_string(),
            CLICKHOUSE_PRE.to_string(),
            CLICKHOUSE_POST.to_string(),
        ];
        let entries = parse_entries(&lines);
        let result = compute_analysis(&entries);

        assert_eq!(result.total_events, 6);
        assert_eq!(result.pre_count, 3);
        assert_eq!(result.post_count, 3);
        assert_eq!(result.matched_count, 3);
        assert_eq!(result.unmatched_pre, 0);
        assert_eq!(result.unmatched_post, 0);
        assert_eq!(result.tools.len(), 3);

        assert!(result.tools.contains_key("Glob"));
        assert!(result.tools.contains_key("mcp__server-everything__echo"));
        assert!(result
            .tools
            .contains_key("mcp__mcp-clickhouse__list_databases"));

        let ch_stats = result
            .tools
            .get("mcp__mcp-clickhouse__list_databases")
            .unwrap();
        assert_eq!(ch_stats.calls, 1);
        assert_eq!(ch_stats.input_tokens, 1);
        assert_eq!(ch_stats.output_tokens, 26);
        // 09:23:14.123821 - 09:23:12.450714 = 1673.107ms
        assert!((ch_stats.latencies[0] - 1673.0).abs() < 2.0);
    }

    #[test]
    fn unmatched_pre_event() {
        // PreToolUse with no matching PostToolUse
        let lines = vec![GLOB_PRE.to_string()];
        let entries = parse_entries(&lines);
        let result = compute_analysis(&entries);

        assert_eq!(result.matched_count, 0);
        assert_eq!(result.unmatched_pre, 1);
        assert_eq!(result.unmatched_post, 0);
        assert!(result.tools.is_empty());
    }

    #[test]
    fn unmatched_post_event() {
        // PostToolUse with no matching PreToolUse
        let lines = vec![ECHO_POST.to_string()];
        let entries = parse_entries(&lines);
        let result = compute_analysis(&entries);

        assert_eq!(result.matched_count, 0);
        assert_eq!(result.unmatched_pre, 0);
        assert_eq!(result.unmatched_post, 1);
        assert!(result.tools.is_empty());
    }

    #[test]
    fn session_duration() {
        let lines = vec![
            GLOB_PRE.to_string(),
            GLOB_POST.to_string(),
            CLICKHOUSE_PRE.to_string(),
            CLICKHOUSE_POST.to_string(),
        ];
        let entries = parse_entries(&lines);
        let result = compute_analysis(&entries);

        // 09:23:14.123821 - 09:08:17.466100 ~= 896s
        assert!(result.duration_secs > 890 && result.duration_secs < 900);
    }

    #[test]
    fn analyze_with_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("test.jsonl");
        let content = format!("{}\n{}\n{}\n{}\n", GLOB_PRE, GLOB_POST, ECHO_PRE, ECHO_POST);
        std::fs::write(&log_path, content).unwrap();

        let result = analyze(&log_path);
        assert!(result.is_ok());
    }

    #[test]
    fn analyze_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("empty.jsonl");
        std::fs::write(&log_path, "").unwrap();

        let result = analyze(&log_path);
        assert!(result.is_ok());
    }

    #[test]
    fn token_aggregation_across_calls() {
        // Two Glob calls to test aggregation
        let pre2 = r#"{"timestamp":"2026-02-14T09:09:00.000000Z","event":"PreToolUse","tool_name":"Glob","tool_use_id":"toolu_second","tool_input":{"pattern":"*.rs"},"session_id":"test","input_tokens":5}"#;
        let post2 = r#"{"timestamp":"2026-02-14T09:09:00.500000Z","event":"PostToolUse","tool_name":"Glob","tool_use_id":"toolu_second","tool_input":{"pattern":"*.rs"},"tool_response":{"numFiles":3},"session_id":"test","input_tokens":5,"output_tokens":30}"#;

        let lines = vec![
            GLOB_PRE.to_string(),
            GLOB_POST.to_string(),
            pre2.to_string(),
            post2.to_string(),
        ];
        let entries = parse_entries(&lines);
        let result = compute_analysis(&entries);

        let glob_stats = result.tools.get("Glob").unwrap();
        assert_eq!(glob_stats.calls, 2);
        assert_eq!(glob_stats.input_tokens, 8 + 5);
        assert_eq!(glob_stats.output_tokens, 20 + 30);
        assert_eq!(glob_stats.latencies.len(), 2);
    }

    #[test]
    fn format_tokens_display() {
        assert_eq!(format_tokens(0), "0");
        assert_eq!(format_tokens(500), "500");
        assert_eq!(format_tokens(999), "999");
        assert_eq!(format_tokens(1000), "1.0K");
        assert_eq!(format_tokens(1500), "1.5K");
        assert_eq!(format_tokens(999_999), "1000.0K");
        assert_eq!(format_tokens(1_000_000), "1.0M");
        assert_eq!(format_tokens(2_500_000), "2.5M");
    }

    #[test]
    fn percentile_and_avg() {
        assert_eq!(avg(&[]), 0.0);
        assert_eq!(avg(&[10.0]), 10.0);
        assert_eq!(avg(&[10.0, 20.0]), 15.0);

        assert_eq!(percentile(&[], 50.0), 0.0);
        assert_eq!(percentile(&[10.0], 50.0), 10.0);
        assert_eq!(percentile(&[10.0, 20.0, 30.0], 50.0), 20.0);
        assert_eq!(percentile(&[10.0, 20.0, 30.0], 0.0), 10.0);
        assert_eq!(percentile(&[10.0, 20.0, 30.0], 100.0), 30.0);
    }
}
