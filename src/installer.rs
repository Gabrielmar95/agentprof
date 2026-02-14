use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// Resolve the settings file path based on scope.
pub fn settings_path(global: bool) -> Result<PathBuf, String> {
    if global {
        let home = std::env::var("HOME").map_err(|_| "HOME environment variable not set")?;
        Ok(PathBuf::from(home).join(".claude").join("settings.json"))
    } else {
        Ok(PathBuf::from(".claude").join("settings.local.json"))
    }
}

/// Find the agentprof binary path.
fn binary_path() -> Result<String, String> {
    std::env::current_exe()
        .map_err(|e| format!("failed to determine binary path: {}", e))
        .map(|p| p.to_string_lossy().into_owned())
}

/// Read a JSON settings file, returning an empty object if the file doesn't exist.
fn read_settings(path: &Path) -> Result<Value, String> {
    if !path.exists() {
        return Ok(json!({}));
    }
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read {}: {}", path.display(), e))?;
    if content.trim().is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_str(&content).map_err(|e| format!("failed to parse {}: {}", path.display(), e))
}

/// Write a JSON settings file, creating parent directories if needed.
fn write_settings(path: &Path, value: &Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create directory {}: {}", parent.display(), e))?;
    }
    let content = serde_json::to_string_pretty(value)
        .map_err(|e| format!("failed to serialize JSON: {}", e))?;
    std::fs::write(path, content + "\n")
        .map_err(|e| format!("failed to write {}: {}", path.display(), e))
}

/// Build the hook command string.
fn hook_command(binary: &str, log_path: &Path) -> String {
    let log_abs = std::fs::canonicalize(log_path).unwrap_or_else(|_| {
        std::path::absolute(log_path).unwrap_or_else(|_| log_path.to_path_buf())
    });
    format!("{} hook --log {}", binary, log_abs.display())
}

/// Check if a hook entry contains an agentprof command.
fn is_agentprof_hook(entry: &Value) -> bool {
    if let Some(hooks) = entry.get("hooks").and_then(|h| h.as_array()) {
        for hook in hooks {
            if let Some(cmd) = hook.get("command").and_then(|c| c.as_str()) {
                if cmd.contains("agentprof") || cmd.contains("mcp-profiler") {
                    return true;
                }
            }
        }
    }
    false
}

/// Build a single hook entry for a given event.
fn make_hook_entry(command: &str) -> Value {
    json!({
        "matcher": "*",
        "hooks": [{"type": "command", "command": command}]
    })
}

/// Install agentprof hooks into the settings file.
pub fn install(log_path: &Path, global: bool) -> Result<(), String> {
    let settings_file = settings_path(global)?;
    let binary = binary_path()?;
    let command = hook_command(&binary, log_path);

    let scope = if global { "global" } else { "local" };
    let abs_settings = std::fs::canonicalize(&settings_file).unwrap_or_else(|_| {
        std::path::absolute(&settings_file).unwrap_or_else(|_| settings_file.clone())
    });
    eprintln!("agentprof: installing hooks ({} scope)", scope);
    eprintln!("agentprof: settings file: {}", abs_settings.display());
    eprintln!("agentprof: hook command: {}", command);

    let mut settings = read_settings(&settings_file)?;
    let obj = settings
        .as_object_mut()
        .ok_or("settings file is not a JSON object")?;

    // Ensure "hooks" key exists as an object.
    if !obj.contains_key("hooks") {
        obj.insert("hooks".to_string(), json!({}));
    }
    let hooks = obj
        .get_mut("hooks")
        .and_then(|h| h.as_object_mut())
        .ok_or("\"hooks\" is not a JSON object")?;

    let new_entry = make_hook_entry(&command);

    for event in &["PreToolUse", "PostToolUse"] {
        if let Some(arr) = hooks.get_mut(*event).and_then(|v| v.as_array_mut()) {
            // Remove existing agentprof/mcp-profiler entries.
            let had_existing = arr.iter().any(|e| is_agentprof_hook(e));
            arr.retain(|e| !is_agentprof_hook(e));
            arr.push(new_entry.clone());
            if had_existing {
                eprintln!("agentprof: replaced existing {} hook", event);
            } else {
                eprintln!("agentprof: added {} hook", event);
            }
        } else {
            hooks.insert(event.to_string(), json!([new_entry.clone()]));
            eprintln!("agentprof: added {} hook", event);
        }
    }

    write_settings(&settings_file, &settings)?;
    eprintln!("agentprof: wrote {}", abs_settings.display());
    eprintln!("agentprof: install complete");
    Ok(())
}

/// Uninstall agentprof hooks from the settings file.
pub fn uninstall(global: bool) -> Result<(), String> {
    let settings_file = settings_path(global)?;
    let scope = if global { "global" } else { "local" };

    if !settings_file.exists() {
        eprintln!(
            "agentprof: {} settings file does not exist ({}), nothing to uninstall",
            scope,
            settings_file.display()
        );
        return Ok(());
    }

    let abs_settings =
        std::fs::canonicalize(&settings_file).unwrap_or_else(|_| settings_file.clone());
    eprintln!("agentprof: uninstalling hooks ({} scope)", scope);
    eprintln!("agentprof: settings file: {}", abs_settings.display());

    let mut settings = read_settings(&settings_file)?;
    let obj = settings
        .as_object_mut()
        .ok_or("settings file is not a JSON object")?;

    let hooks = match obj.get_mut("hooks").and_then(|h| h.as_object_mut()) {
        Some(h) => h,
        None => {
            eprintln!("agentprof: no hooks section found, nothing to uninstall");
            return Ok(());
        }
    };

    let mut removed_any = false;
    for event in &["PreToolUse", "PostToolUse"] {
        if let Some(arr) = hooks.get_mut(*event).and_then(|v| v.as_array_mut()) {
            let before = arr.len();
            arr.retain(|e| !is_agentprof_hook(e));
            let after = arr.len();
            if before != after {
                eprintln!(
                    "agentprof: removed {} hook ({} entries removed)",
                    event,
                    before - after
                );
                removed_any = true;
            }
        }
    }

    // Clean up empty arrays and empty hooks object.
    let empty_events: Vec<String> = hooks
        .iter()
        .filter(|(_, v)| v.as_array().map_or(false, |a| a.is_empty()))
        .map(|(k, _)| k.clone())
        .collect();
    for key in &empty_events {
        hooks.remove(key);
        eprintln!("agentprof: removed empty {} array", key);
    }
    if hooks.is_empty() {
        obj.remove("hooks");
        eprintln!("agentprof: removed empty hooks section");
    }

    if !removed_any {
        eprintln!("agentprof: no agentprof hooks found, nothing to uninstall");
        return Ok(());
    }

    write_settings(&settings_file, &settings)?;
    eprintln!("agentprof: wrote {}", abs_settings.display());
    eprintln!("agentprof: uninstall complete");
    Ok(())
}
