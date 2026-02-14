use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookLogEntry {
    pub timestamp: DateTime<Utc>,
    pub event: String,
    pub tool_name: String,
    pub tool_use_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_input: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_response: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<usize>,
}

pub struct Logger {
    writer: BufWriter<File>,
}

impl Logger {
    pub fn new(path: &Path) -> std::io::Result<Self> {
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(Logger {
            writer: BufWriter::new(file),
        })
    }

    pub fn log_hook_entry(&mut self, entry: &HookLogEntry) -> std::io::Result<()> {
        let line = serde_json::to_string(entry)?;
        writeln!(self.writer, "{}", line)?;
        self.writer.flush()
    }
}
