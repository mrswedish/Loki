use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use futures_util::StreamExt;
use std::fs;
use std::path::PathBuf;

/// Predefined models available for download from Hugging Face.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelEntry {
    pub id: String,
    pub name: String,
    pub filename: String,
    pub url: String,
    pub size_bytes: u64,
    pub description: String,
    pub flavor: String,
    pub ram_required_gb: f32,
    pub is_default: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelStatus {
    #[serde(flatten)]
    pub entry: ModelEntry,
    pub downloaded: bool,
    pub local_path: Option<String>,
}

#[derive(Clone, Serialize)]
pub struct DownloadProgress {
    pub model_id: String,
    pub percent: f32,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
}

pub fn model_registry() -> Vec<ModelEntry> {
    vec![
        ModelEntry {
            id: "gemma-3n-e2b".to_string(),
            name: "Gemma 3n E2B".to_string(),
            filename: "google_gemma-3n-E2B-it-Q4_K_M.gguf".to_string(),
            url: "https://huggingface.co/bartowski/google_gemma-3n-E2B-it-GGUF/resolve/main/google_gemma-3n-E2B-it-Q4_K_M.gguf".to_string(),
            size_bytes: 1_850_000_000,
            description: "Google Gemma 3n E2B – blixtsnabb och effektiv, Q4 (~1.7 GB)".to_string(),
            flavor: "Kompakt".to_string(),
            ram_required_gb: 3.0,
            is_default: true,
        },
        ModelEntry {
            id: "phi-4-mini".to_string(),
            name: "Phi-4 Mini".to_string(),
            filename: "Phi-4-mini-instruct-Q4_K_M.gguf".to_string(),
            url: "https://huggingface.co/bartowski/Phi-4-mini-instruct-GGUF/resolve/main/Phi-4-mini-instruct-Q4_K_M.gguf".to_string(),
            size_bytes: 2_500_000_000,
            description: "Microsoft Phi-4 Mini – stark på instruktioner och svenska texter, Q4 (~2.5 GB)".to_string(),
            flavor: "Smidig".to_string(),
            ram_required_gb: 4.0,
            is_default: false,
        },
        ModelEntry {
            id: "ministral-3b".to_string(),
            name: "Ministral 3B".to_string(),
            filename: "Ministral-3-3B-Instruct-2512-Q5_K_M.gguf".to_string(),
            url: "https://huggingface.co/mistralai/Ministral-3-3B-Instruct-2512-GGUF/resolve/main/Ministral-3-3B-Instruct-2512-Q5_K_M.gguf".to_string(),
            size_bytes: 2_474_178_720,
            description: "Ministral 3B Instruct 2512 – kompakt expert. Tips: Sänk kontextfönstret i inställningar vid lite VRAM. Q5 (~2.3 GB)".to_string(),
            flavor: "Kompakt".to_string(),
            ram_required_gb: 5.0,
            is_default: false,
        },
        ModelEntry {
            id: "gemma-3n-e4b".to_string(),
            name: "Gemma 3n E4B".to_string(),
            filename: "google_gemma-3n-E4B-it-Q4_K_M.gguf".to_string(),
            url: "https://huggingface.co/bartowski/google_gemma-3n-E4B-it-GGUF/resolve/main/google_gemma-3n-E4B-it-Q4_K_M.gguf".to_string(),
            size_bytes: 3_500_000_000,
            description: "Google Gemma 3n E4B – kraftfullare syskon till E2B, bättre på längre texter, Q4 (~3.5 GB)".to_string(),
            flavor: "Analytisk".to_string(),
            ram_required_gb: 6.0,
            is_default: false,
        },
        ModelEntry {
            id: "ministral-8b".to_string(),
            name: "Ministral 8B".to_string(),
            filename: "Ministral-3-8B-Instruct-2512-Q5_K_M.gguf".to_string(),
            url: "https://huggingface.co/mistralai/Ministral-3-8B-Instruct-2512-GGUF/resolve/main/Ministral-3-8B-Instruct-2512-Q5_K_M.gguf".to_string(),
            size_bytes: 6_059_268_512,
            description: "Ministral 8B Instruct 2512 – efterföljaren till Mistral 7B. Kräver sänkt kontext vid < 8GB VRAM. Q5 (~5.6 GB)".to_string(),
            flavor: "Balanserad".to_string(),
            ram_required_gb: 10.0,
            is_default: false,
        },
    ]
}

fn models_dir(app: &tauri::AppHandle) -> PathBuf {
    crate::get_app_dir(app).join("models")
}

pub fn list_models_with_status(app: &tauri::AppHandle) -> Vec<ModelStatus> {
    let dir = models_dir(app);
    model_registry()
        .into_iter()
        .map(|entry| {
            let path = dir.join(&entry.filename);
            let legacy_filename = entry.filename.replace("-E2B", "_E2B");
            let legacy_path = dir.join(&legacy_filename);
            
            let downloaded = path.exists() || legacy_path.exists();
            let final_path = if path.exists() {
                Some(path.to_string_lossy().to_string())
            } else if legacy_path.exists() {
                Some(legacy_path.to_string_lossy().to_string())
            } else {
                None
            };

            ModelStatus {
                local_path: final_path,
                downloaded,
                entry,
            }
        })
        .collect()
}


pub async fn download_model(model_id: String, app: AppHandle) -> Result<String, String> {
    let entry = model_registry()
        .into_iter()
        .find(|e| e.id == model_id)
        .ok_or_else(|| format!("Okänd modell: {}", model_id))?;

    let dir = models_dir(&app);
    fs::create_dir_all(&dir).map_err(|e| format!("Kunde inte skapa katalog: {}", e))?;

    let dest = dir.join(&entry.filename);

    // If already downloaded, return immediately
    if dest.exists() {
        return Ok(dest.to_string_lossy().to_string());
    }

    // Download with progress
    let client = reqwest::Client::new();
    let response = client
        .get(&entry.url)
        .send()
        .await
        .map_err(|e| format!("Nedladdningsfel: {}", e))?;

    // Validate HTTP status – HF returns 404 with "Entry not found" for wrong filenames
    if !response.status().is_success() {
        return Err(format!("Nedladdningsfel: HTTP {}", response.status()));
    }

    let total = response.content_length().unwrap_or(entry.size_bytes);
    let mut downloaded: u64 = 0;

    // Write to temp file, then rename (atomic)
    let tmp_path = dest.with_extension("gguf.part");
    let mut file = tokio::fs::File::create(&tmp_path)
        .await
        .map_err(|e| format!("Kunde inte skapa fil: {}", e))?;

    let mut stream = response.bytes_stream();
    let mut last_percent: i32 = -1;

    use tokio::io::AsyncWriteExt;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Nedladdningsfel: {}", e))?;
        file.write_all(&chunk)
            .await
            .map_err(|e| format!("Skrivfel: {}", e))?;

        downloaded += chunk.len() as u64;
        let percent = ((downloaded as f64 / total as f64) * 100.0) as i32;

        // Only emit every 1%
        if percent != last_percent {
            last_percent = percent;
            let _ = app.emit(
                "download-progress",
                DownloadProgress {
                    model_id: model_id.clone(),
                    percent: percent as f32,
                    downloaded_bytes: downloaded,
                    total_bytes: total,
                },
            );
        }
    }

    file.sync_all().await.map_err(|e| format!("Sync-fel: {}", e))?;
    drop(file);

    // Fallback till kopiering för Windows om filen fortfarande är låst
    if let Err(_e) = std::fs::rename(&tmp_path, &dest) {
        std::fs::copy(&tmp_path, &dest).map_err(|e| format!("Kunde inte kopiera temp-filen: {}", e))?;
        let _ = std::fs::remove_file(&tmp_path);
    }

    let _ = app.emit("download-complete", model_id.clone());

    Ok(dest.to_string_lossy().to_string())
}

/// Tar bort .gguf-filer i models-katalogen som inte längre finns i model_registry().
/// Körs vid appstart för att rensa bort gamla modeller efter en registerbyte.
pub fn cleanup_unknown_models(app: &tauri::AppHandle) {
    let known: std::collections::HashSet<String> = model_registry()
        .into_iter()
        .map(|e| e.filename)
        .collect();

    let models_dir = crate::get_app_dir(app).join("models");
    let Ok(entries) = std::fs::read_dir(&models_dir) else { return };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().map_or(false, |e| e == "gguf") {
            let filename = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            if !known.contains(&filename) {
                eprintln!("[cleanup] Tar bort okänd modell: {filename}");
                let _ = std::fs::remove_file(&path);
            }
        }
    }
}
