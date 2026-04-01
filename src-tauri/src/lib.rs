use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

mod llama_server;
mod model_download;
mod inference;

use tauri::Manager;
use tauri_plugin_dialog::DialogExt;

pub fn get_app_dir(app: &tauri::AppHandle) -> PathBuf {
	app.path().app_data_dir().expect("Could not find app_data directory")
}

pub fn ensure_dirs(app: &tauri::AppHandle) {
	let app_dir = get_app_dir(app);
	let mut dirs = vec![app_dir.clone(), app_dir.join("models"), app_dir.join("bin")];
	if let Ok(log_dir) = app.path().app_log_dir() {
		dirs.push(log_dir);
	}
	for dir in dirs {
		if !dir.exists() {
			fs::create_dir_all(dir).ok();
		}
	}
}

// ─── File Export ─────────────────────────────────────────

/// Opens a native "Save As" dialog and writes the given text content to the chosen path.
/// Returns the saved file path on success.
#[tauri::command]
async fn save_text_file(content: String, app: tauri::AppHandle) -> Result<String, String> {
	let path = app
		.dialog()
		.file()
		.set_file_name("anonymiserat-dokument.txt")
		.add_filter("Textfil", &["txt"])
		.blocking_save_file()
		.ok_or_else(|| "Ingen fil vald".to_string())?;
	let path_str = path.to_string();
	std::fs::write(&path_str, content.as_bytes())
		.map_err(|e| format!("Skrivfel: {}", e))?;
	Ok(path_str)
}

// ─── System Info ─────────────────────────────────────────

#[derive(Serialize)]
pub struct SystemInfo {
	pub total_ram_gb: f32,
	pub available_ram_gb: f32,
}

#[tauri::command]
fn get_system_info() -> SystemInfo {
	use sysinfo::System;
	let mut sys = System::new();
	sys.refresh_memory();
	SystemInfo {
		total_ram_gb: sys.total_memory() as f32 / (1024.0 * 1024.0 * 1024.0),
		available_ram_gb: sys.available_memory() as f32 / (1024.0 * 1024.0 * 1024.0),
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

#[tauri::command]
fn list_available_models(app: tauri::AppHandle) -> Vec<model_download::ModelStatus> {
	model_download::list_models_with_status(&app)
}

#[tauri::command]
async fn download_model_cmd(model_id: String, app: tauri::AppHandle) -> Result<String, String> {
	model_download::download_model(model_id, app).await
}

#[tauri::command]
fn delete_model_cmd(model_id: String, app: tauri::AppHandle) -> Result<(), String> {
	let models_dir = get_app_dir(&app).join("models");
	let entry = model_download::model_registry()
		.into_iter()
		.find(|e| e.id == model_id)
		.ok_or_else(|| format!("Okänd modell: {}", model_id))?;

	let dest = models_dir.join(&entry.filename);
	if dest.exists() {
		fs::remove_file(&dest).map_err(|e| format!("Kunde inte ta bort fil: {}", e))?;
	}
	Ok(())
}

// ─── Binary Management ───────────────────────────────────

/// Returns the installed llama-server release tag (e.g. "b5262"), or null if unknown.
#[tauri::command]
fn get_server_binary_version(app: tauri::AppHandle) -> Option<String> {
	llama_server::get_installed_version(&app)
}

/// Removes the current llama-server binary so the next server start re-downloads the latest release.
#[tauri::command]
fn update_server_binary(app: tauri::AppHandle) -> Result<(), String> {
	llama_server::clear_server_binary(&app)
}

/// Returns true if the llama-server binary file exists (even if version.txt is missing).
#[tauri::command]
fn server_binary_exists(app: tauri::AppHandle) -> bool {
	llama_server::server_binary_path(&app).exists()
}

/// Downloads (or re-downloads) the llama-server binary and returns the installed version tag.
#[tauri::command]
async fn download_server_binary(app: tauri::AppHandle) -> Result<String, String> {
	llama_server::ensure_server_binary(&app).await?;
	Ok(llama_server::get_installed_version(&app).unwrap_or_else(|| "okänd".to_string()))
}

// ─── Server Lifecycle ────────────────────────────────────

/// Starta llama-server med vald modell. Returnerar "http://127.0.0.1:{port}".
#[tauri::command]
async fn start_server(
	model_path: String,
	context_size: Option<u32>,
	gpu_index: Option<i32>,
	engine: tauri::State<'_, inference::SharedEngine>,
	app: tauri::AppHandle,
) -> Result<String, String> {
	let bin_path = llama_server::ensure_server_binary(&app).await?;

	let engine_clone = engine.inner().clone();
	let log_dir = app.path().app_log_dir().ok();
	tokio::task::spawn_blocking(move || {
		let mut eng = engine_clone.lock().map_err(|e| format!("Lock-fel: {}", e))?;
		eng.set_server_binary(bin_path);
		let port = eng.start(&model_path, context_size, gpu_index, log_dir)?;
		Ok(format!("http://127.0.0.1:{}", port))
	})
	.await
	.map_err(|e| format!("Tokio join error: {}", e))?
}

/// Stäng av llama-server.
#[tauri::command]
fn stop_server(engine: tauri::State<'_, inference::SharedEngine>) -> Result<(), String> {
	let mut eng = engine.lock().map_err(|e| format!("Lock-fel: {}", e))?;
	eng.stop();
	Ok(())
}

/// Returnera aktuell server-URL om servern körs, annars null.
#[tauri::command]
fn get_server_url(engine: tauri::State<'_, inference::SharedEngine>) -> Option<String> {
	engine.lock().ok().and_then(|eng| eng.server_url())
}

/// Returns true if the llama-server process is currently responding.
#[tauri::command]
fn check_server_health(engine: tauri::State<'_, inference::SharedEngine>) -> bool {
	engine.lock().ok().map_or(false, |eng| eng.server_is_alive())
}

/// If the server is dead but a model was previously loaded, restarts it.
/// Returns the new server URL on restart, or null if already alive / nothing to restart.
#[tauri::command]
async fn restart_server_if_dead(
	engine: tauri::State<'_, inference::SharedEngine>,
) -> Result<Option<String>, String> {
	let engine_clone = engine.inner().clone();
	tokio::task::spawn_blocking(move || {
		let mut eng = engine_clone.lock().map_err(|e| format!("Lock-fel: {}", e))?;
		eng.restart_if_dead()
	})
	.await
	.map_err(|e| format!("Tokio join error: {}", e))?
}

// ─── Tauri App Entry ─────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
	let engine = inference::create_engine();

	let app = tauri::Builder::default()
		.plugin(tauri_plugin_opener::init())
		.plugin(tauri_plugin_dialog::init())
		.setup(|app| {
			ensure_dirs(app.handle());
			model_download::cleanup_unknown_models(app.handle());
			Ok(())
		})
		.manage(engine.clone())
		.invoke_handler(tauri::generate_handler![
			// Model management
			list_models,
			list_available_models,
			download_model_cmd,
			delete_model_cmd,
			// Binary management
			get_server_binary_version,
			update_server_binary,
			server_binary_exists,
			download_server_binary,
			// Server lifecycle
			start_server,
			stop_server,
			get_server_url,
			check_server_health,
			restart_server_if_dead,
			// System
			get_system_info,
			// File export
			save_text_file,
		])
		.build(tauri::generate_context!())
		.expect("error while building tauri application");

	app.run(move |_app_handle, event| match event {
		tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit => {
			if let Ok(mut eng) = engine.lock() {
				eng.stop();
			}
		}
		_ => {}
	});
}
