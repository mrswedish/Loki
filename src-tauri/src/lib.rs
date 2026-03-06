use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};
use uuid::Uuid;
use std::fs;
use std::path::PathBuf;

mod llama_server;
mod model_download;
mod inference;

use tauri::Manager;

/// Get the app data directory
pub fn get_app_dir(app: &tauri::AppHandle) -> PathBuf {
    app.path().app_data_dir().expect("Could not find app_data directory")
}

/// Ensure all app directories exist
pub fn ensure_dirs(app: &tauri::AppHandle) {
    let app_dir = get_app_dir(app);
    let dirs_to_create = [
        app_dir.clone(),
        app_dir.join("models"),
        app_dir.join("sessions"),
    ];
    for dir in &dirs_to_create {
        if !dir.exists() {
            fs::create_dir_all(dir).ok();
        }
    }
}

// ─── Data Types ──────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub name: String,
    pub filename: String,
    pub path: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String, // "user" or "assistant"
    pub content: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub info: SessionInfo,
    pub messages: Vec<Message>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub active_model: Option<String>,
    pub font_size: u32,
    pub scanline_intensity: u32,
    pub text_color: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            active_model: None,
            font_size: 16,
            scanline_intensity: 0,
            text_color: "#ffb000".to_string(),
        }
    }
}

// ─── Model Management ────────────────────────────────────

#[tauri::command]
fn list_models(app: tauri::AppHandle) -> Vec<ModelInfo> {
    let models_dir = get_app_dir(&app).join("models");
    let mut models = Vec::new();

    if let Ok(entries) = fs::read_dir(&models_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map_or(false, |ext| ext == "gguf") {
                if let Ok(metadata) = fs::metadata(&path) {
                    let filename = path.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    let name = filename
                        .trim_end_matches(".gguf")
                        .replace('-', " ")
                        .replace('_', " ");
                    models.push(ModelInfo {
                        name,
                        filename: filename.clone(),
                        path: path.to_string_lossy().to_string(),
                        size_bytes: metadata.len(),
                    });
                }
            }
        }
    }

    models
}

// ─── Model Download ──────────────────────────────────────

#[tauri::command]
fn list_available_models(app: tauri::AppHandle) -> Vec<model_download::ModelStatus> {
    model_download::list_models_with_status(&app)
}

#[tauri::command]
async fn download_model_cmd(model_id: String, app: tauri::AppHandle) -> Result<String, String> {
    model_download::download_model(model_id, app).await
}

#[tauri::command]
fn check_default_model(app: tauri::AppHandle) -> bool {
    model_download::get_default_model_path(&app).is_some()
}

#[tauri::command]
fn delete_model_cmd(model_id: String, app: tauri::AppHandle) -> Result<(), String> {
    let models_dir = get_app_dir(&app).join("models");
    let entry = model_download::model_registry()
        .into_iter()
        .find(|e| e.id == model_id)
        .ok_or_else(|| format!("Okänd modell (kan ej ta bort): {}", model_id))?;

    let dest = models_dir.join(&entry.filename);
    let legacy_filename = entry.filename.replace("-E2B", "_E2B");
    let legacy_dest = models_dir.join(&legacy_filename);

    if dest.exists() {
        fs::remove_file(&dest).map_err(|e| format!("Kunde inte ta bort fil: {}", e))?;
    }
    if legacy_dest.exists() {
        let _ = fs::remove_file(&legacy_dest);
    }
    
    Ok(())
}

// ─── Inference ───────────────────────────────────────────

#[tauri::command]
async fn load_model_cmd(
    model_path: String,
    engine: tauri::State<'_, inference::SharedEngine>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    // Ensure the llama-server binary is downloaded
    let bin_path = llama_server::ensure_server_binary(&app).await?;

    // Set the binary path and load the model (blocking work in spawn_blocking)
    let engine_clone = engine.inner().clone();
    tokio::task::spawn_blocking(move || {
        let mut eng = engine_clone.lock().map_err(|e| format!("Lock-fel: {}", e))?;
        eng.set_server_binary(bin_path);
        eng.load_model(&model_path)
    })
    .await
    .map_err(|e| format!("Tokio join error: {}", e))?
}

#[tauri::command]
async fn chat_stream(
    session_id: String,
    message: String,
    engine: tauri::State<'_, inference::SharedEngine>,
    app: tauri::AppHandle,
) -> Result<String, String> {
    let messages = serde_json::json!([
        {"role": "user", "content": message}
    ]);

    let engine_clone = engine.inner().clone();
    let session_id_clone = session_id.clone();
    let app_clone = app.clone();

    // Run inference on a blocking thread to avoid blocking the async runtime
    let result = tokio::task::spawn_blocking(move || {
        let eng = engine_clone.lock().map_err(|e| format!("Lock-fel: {}", e))?;
        let msgs = messages.as_array().unwrap().clone();
        eng.generate(&msgs, 1024, &app_clone, &session_id_clone)
    })
    .await
    .map_err(|e| format!("Tokio join error: {}", e))??;

    Ok(result)
}

#[tauri::command]
fn is_model_loaded(engine: tauri::State<'_, inference::SharedEngine>) -> bool {
    engine.lock().map(|eng| eng.is_loaded()).unwrap_or(false)
}

// ─── Session Management ──────────────────────────────────

fn sessions_dir(app: &tauri::AppHandle) -> PathBuf {
    get_app_dir(app).join("sessions")
}

fn session_file(id: &str, app: &tauri::AppHandle) -> PathBuf {
    sessions_dir(app).join(format!("{}.json", id))
}

#[tauri::command]
fn list_sessions(app: tauri::AppHandle) -> Vec<SessionInfo> {
    let dir = sessions_dir(&app);
    let mut sessions = Vec::new();

    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map_or(false, |ext| ext == "json") {
                if let Ok(data) = fs::read_to_string(&path) {
                    if let Ok(session) = serde_json::from_str::<Session>(&data) {
                        sessions.push(session.info);
                    }
                }
            }
        }
    }

    sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    sessions
}

#[tauri::command]
fn create_session(name: String, app: tauri::AppHandle) -> Result<SessionInfo, String> {
    ensure_dirs(&app);
    let now = Utc::now();
    let info = SessionInfo {
        id: Uuid::new_v4().to_string(),
        name,
        created_at: now,
        updated_at: now,
    };
    let session = Session {
        info: info.clone(),
        messages: Vec::new(),
    };
    let json = serde_json::to_string_pretty(&session)
        .map_err(|e| e.to_string())?;
    fs::write(session_file(&info.id, &app), json)
        .map_err(|e| e.to_string())?;
    Ok(info)
}

#[tauri::command]
fn delete_session(session_id: String, app: tauri::AppHandle) -> Result<(), String> {
    let path = session_file(&session_id, &app);
    if path.exists() {
        fs::remove_file(path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
fn get_session_messages(session_id: String, app: tauri::AppHandle) -> Result<Vec<Message>, String> {
    let path = session_file(&session_id, &app);
    let data = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let session: Session = serde_json::from_str(&data).map_err(|e| e.to_string())?;
    Ok(session.messages)
}

#[tauri::command]
fn add_message(session_id: String, role: String, content: String, app: tauri::AppHandle) -> Result<(), String> {
    let path = session_file(&session_id, &app);
    let data = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let mut session: Session = serde_json::from_str(&data).map_err(|e| e.to_string())?;

    session.messages.push(Message {
        role,
        content,
        timestamp: Utc::now(),
    });
    session.info.updated_at = Utc::now();

    let json = serde_json::to_string_pretty(&session).map_err(|e| e.to_string())?;
    fs::write(&path, json).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn rename_session(session_id: String, new_name: String, app: tauri::AppHandle) -> Result<(), String> {
    let path = session_file(&session_id, &app);
    let data = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let mut session: Session = serde_json::from_str(&data).map_err(|e| e.to_string())?;

    session.info.name = new_name;
    session.info.updated_at = Utc::now();

    let json = serde_json::to_string_pretty(&session).map_err(|e| e.to_string())?;
    fs::write(&path, json).map_err(|e| e.to_string())?;
    Ok(())
}

// ─── Settings ────────────────────────────────────────────

fn settings_file(app: &tauri::AppHandle) -> PathBuf {
    get_app_dir(app).join("settings.json")
}

#[tauri::command]
fn get_settings(app: tauri::AppHandle) -> Settings {
    let path = settings_file(&app);
    if let Ok(data) = fs::read_to_string(&path) {
        serde_json::from_str(&data).unwrap_or_default()
    } else {
        Settings::default()
    }
}

#[tauri::command]
fn save_settings(settings: Settings, app: tauri::AppHandle) -> Result<(), String> {
    ensure_dirs(&app);
    let json = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
    fs::write(settings_file(&app), json).map_err(|e| e.to_string())?;
    Ok(())
}

// ─── File Attachment ─────────────────────────────────────

#[tauri::command]
fn read_text_file(file_path: String) -> Result<String, String> {
    fs::read_to_string(&file_path).map_err(|e| format!("Kunde inte läsa filen: {}", e))
}

// ─── Tauri App Entry ─────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let engine = inference::create_engine();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            ensure_dirs(app.handle());
            Ok(())
        })
        .manage(engine)
        .invoke_handler(tauri::generate_handler![
            // Model management
            list_models,
            list_available_models,
            download_model_cmd,
            delete_model_cmd,
            check_default_model,
            load_model_cmd,
            is_model_loaded,
            // Inference
            chat_stream,
            // Sessions
            list_sessions,
            create_session,
            delete_session,
            get_session_messages,
            add_message,
            rename_session,
            // Settings
            get_settings,
            save_settings,
            // File
            read_text_file,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
