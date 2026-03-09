use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

mod llama_server;
mod model_download;
mod inference;

use tauri::Manager;

pub fn get_app_dir(app: &tauri::AppHandle) -> PathBuf {
	app.path().app_data_dir().expect("Could not find app_data directory")
}

pub fn ensure_dirs(app: &tauri::AppHandle) {
	let app_dir = get_app_dir(app);
	for dir in [app_dir.clone(), app_dir.join("models"), app_dir.join("bin")] {
		if !dir.exists() {
			fs::create_dir_all(dir).ok();
		}
	}
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
	tokio::task::spawn_blocking(move || {
		let mut eng = engine_clone.lock().map_err(|e| format!("Lock-fel: {}", e))?;
		eng.set_server_binary(bin_path);
		let port = eng.start(&model_path, context_size, gpu_index)?;
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

// ─── Tauri App Entry ─────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
	let engine = inference::create_engine();

	let app = tauri::Builder::default()
		.plugin(tauri_plugin_opener::init())
		.setup(|app| {
			ensure_dirs(app.handle());
			Ok(())
		})
		.manage(engine.clone())
		.invoke_handler(tauri::generate_handler![
			// Model management
			list_models,
			list_available_models,
			download_model_cmd,
			delete_model_cmd,
			// Server lifecycle
			start_server,
			stop_server,
			get_server_url,
			// System
			get_system_info,
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
