use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_LOG_BYTES: u64 = 2 * 1024 * 1024; // 2 MB

static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();
static LOG_LOCK: Mutex<()> = Mutex::new(());

/// Initierar sökvägen till den lokala loggfilen (`{app_data_dir}/loki.log`).
pub fn init_logger(app_data_dir: PathBuf) {
    let _ = fs::create_dir_all(&app_data_dir);
    let path = app_data_dir.join("loki.log");
    let _ = LOG_PATH.set(path);
}

pub fn log_file_path() -> Option<PathBuf> {
    LOG_PATH.get().cloned()
}

fn format_timestamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let s = secs % 60;
    let m = (secs / 60) % 60;
    let h = (secs / 3600) % 24;
    let days = secs / 86400;
    format!("d{days} {h:02}:{m:02}:{s:02}Z")
}

pub fn log_line(level: &str, msg: &str) {
    let Some(path) = LOG_PATH.get() else {
        return;
    };
    let Ok(_g) = LOG_LOCK.lock() else {
        return;
    };

    if let Ok(meta) = fs::metadata(path) {
        if meta.len() > MAX_LOG_BYTES {
            let old = path.with_extension("log.old");
            let _ = fs::remove_file(&old);
            let _ = fs::rename(path, &old);
        }
    }

    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let ts = format_timestamp();
        let _ = writeln!(file, "[{ts}] [{level}] {msg}");
    }
}

pub fn log_info(msg: &str) {
    log_line("INFO", msg);
}

pub fn log_warn(msg: &str) {
    log_line("WARN", msg);
}

pub fn log_error(msg: &str) {
    log_line("ERROR", msg);
}

/// Läser de senaste `max_lines` raderna ur `loki.log` utan att ladda jättefiler i minnet.
pub fn read_recent_logs(max_lines: usize) -> String {
    let Some(path) = LOG_PATH.get() else {
        return "Loggfilen är inte initierad ännu.".to_string();
    };
    let Ok(_g) = LOG_LOCK.lock() else {
        return "Kunde inte låsa loggfilen för läsning.".to_string();
    };
    match fs::read_to_string(path) {
        Ok(content) => {
            let lines: Vec<&str> = content.lines().collect();
            let start = lines.len().saturating_sub(max_lines);
            lines[start..].join("\n")
        }
        Err(_) => "Ingen logg skapad ännu.".to_string(),
    }
}

pub fn clear_logs() -> Result<(), String> {
    let Some(path) = LOG_PATH.get() else {
        return Ok(());
    };
    let _g = LOG_LOCK
        .lock()
        .map_err(|e| format!("Låsfel för loggfil: {e}"))?;
    fs::write(path, b"").map_err(|e| format!("Kunde inte rensa loggfilen: {e}"))
}

