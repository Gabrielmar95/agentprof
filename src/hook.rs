use crate::logger::{HookLogEntry, Logger};
use crate::tokens;
use chrono::Utc;
use std::io::Read;
use std::path::Path;
use std::process::ExitCode;

pub fn run_hook(log_path: &Path) -> ExitCode {
    let mut input = String::new();
    if let Err(e) = std::io::stdin().read_to_string(&mut input) {
        eprintln!("agentprof hook: failed to read stdin: {}", e);
        return ExitCode::from(1);
    }

    let json: serde_json::Value = match serde_json::from_str(&input) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("agentprof hook: failed to parse JSON: {}", e);
            return ExitCode::from(1);
        }
    };

    let event = match json.get("hook_event_name").and_then(|v| v.as_str()) {
        Some(e) => e.to_string(),
        None => {
            eprintln!("agentprof hook: missing hook_event_name field");
            return ExitCode::from(1);
        }
    };

    let tool_name = json
        .get("tool_name")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    let tool_use_id = json
        .get("tool_use_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let tool_input = json.get("tool_input").cloned();

    let tool_response = json.get("tool_response").cloned();

    let session_id = json
        .get("session_id")
        .and_then(|v| v.as_str())
        .map(String::from);

    let input_tokens = tool_input.as_ref().map(tokens::count_tokens_json);
    let output_tokens = tool_response.as_ref().map(tokens::count_tokens_json);

    let entry = HookLogEntry {
        timestamp: Utc::now(),
        event,
        tool_name,
        tool_use_id,
        tool_input,
        tool_response,
        session_id,
        input_tokens,
        output_tokens,
    };

    let mut logger = match Logger::new(log_path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("agentprof hook: failed to open log file: {}", e);
            return ExitCode::from(1);
        }
    };

    if let Err(e) = logger.log_hook_entry(&entry) {
        eprintln!("agentprof hook: failed to write log: {}", e);
        return ExitCode::from(1);
    }

    ExitCode::SUCCESS
}
