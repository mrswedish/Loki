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
            id: "qwen-3.5-2b-instruct".to_string(),
            name: "Qwen 3.5 2B".to_string(),
            filename: "Qwen3.5-2B-Q4_K_M.gguf".to_string(),
            url: "https://huggingface.co/unsloth/Qwen3.5-2B-GGUF/resolve/main/Qwen3.5-2B-Q4_K_M.gguf?download=true".to_string(),
            size_bytes: 1_280_835_840,
            description: "Qwen 3.5 2B Instruct – balanserad med kraftfullt resonemang (Thinking), Q4 (~1.2 GB)".to_string(),
            flavor: "Smidig".to_string(),
            ram_required_gb: 3.0,
            is_default: false,
        },
        ModelEntry {
            id: "qwen-3.5-4b-instruct".to_string(),
            name: "Qwen 3.5 4B".to_string(),
            filename: "Qwen3.5-4B-Q4_K_M.gguf".to_string(),
            url: "https://huggingface.co/unsloth/Qwen3.5-4B-GGUF/resolve/main/Qwen3.5-4B-Q4_K_M.gguf?download=true".to_string(),
            size_bytes: 2_740_937_888,
            description: "Qwen 3.5 4B Instruct – smart analytiskt resonemang, kräver mer RAM, Q4 (~2.6 GB)".to_string(),
            flavor: "Analytisk".to_string(),
            ram_required_gb: 6.0,
            is_default: false,
        },
        ModelEntry {
            id: "qwen-3.5-9b-instruct".to_string(),
            name: "Qwen 3.5 9B".to_string(),
            filename: "Qwen3.5-9B-Q4_K_M.gguf".to_string(),
            url: "https://huggingface.co/unsloth/Qwen3.5-9B-GGUF/resolve/main/Qwen3.5-9B-Q4_K_M.gguf?download=true".to_string(),
            size_bytes: 5_680_522_464,
            description: "Qwen 3.5 9B Instruct – tungviktare, kräver modern dator med mycket RAM, Q4 (~5.3 GB)".to_string(),
            flavor: "Kraftfull".to_string(),
            ram_required_gb: 10.0,
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
        ModelEntry {
            id: "qwen-2.5-coder-7b".to_string(),
            name: "Qwen 2.5 Coder 7B".to_string(),
            filename: "Qwen2.5-Coder-7B-Instruct-Q4_K_M.gguf".to_string(),
            url: "https://huggingface.co/unsloth/Qwen2.5-Coder-7B-Instruct-GGUF/resolve/main/Qwen2.5-Coder-7B-Instruct-Q4_K_M.gguf?download=true".to_string(),
            size_bytes: 4_683_073_504,
            description: "Qwen 2.5 Coder 7B – fokus på kod/logik, kräver mycket RAM, Q4 (~4.4 GB)".to_string(),
            flavor: "Logik".to_string(),
            ram_required_gb: 8.0,
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
