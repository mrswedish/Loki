mod documents;
mod logger;
mod summarizer;
mod transcript;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use summarizer::{
    get_model_spec, ModelSpec, SummarizeRequest, SummarizeResult, SummarizerEngine, ALL_MODELS,
    MODEL_12B, MODEL_E2B, MODEL_E4B,
};
use tauri::{AppHandle, Emitter, Manager};

static ENGINE: Mutex<Option<SummarizerEngine>> = Mutex::new(None);
/// Låsfri spegling av vilken modell som är inladdad i `ENGINE` (`0 = ingen`, `1 = e2b`, `2 = e4b`, `3 = 12b`).
/// Förhindrar all re-entrant låsning och UI-blockering när `build_models_overview` anropas under körning!
const SLOT_NONE: u8 = 0;
const SLOT_E2B: u8 = 1;
const SLOT_E4B: u8 = 2;
const SLOT_12B: u8 = 3;
static LOADED_MODEL_SLOT: AtomicU8 = AtomicU8::new(SLOT_NONE);

static SUMMARIZE_CANCEL: AtomicBool = AtomicBool::new(false);
static DOWNLOAD_CANCEL: AtomicBool = AtomicBool::new(false);
static APP_EXITING: AtomicBool = AtomicBool::new(false);
static STATE_LOCK: Mutex<()> = Mutex::new(());

/// Frigör alltid `SummarizerEngine` (`LlamaContext` först, därefter `LlamaModel`) explicit INNAN
/// `tao::EventLoop` anropar `std::process::exit(0)`. Annars ligger Metal residency sets (`rsets`)
/// respektive Vulkan-resurser kvar i `static ENGINE` när C++ globala destruktorer körs i
/// `__cxa_finalize_ranges`, vilket utlöser `GGML_ASSERT([rsets->data count] == 0)` -> `SIGABRT`!
fn shutdown_engine() {
    APP_EXITING.store(true, Ordering::SeqCst);
    SUMMARIZE_CANCEL.store(true, Ordering::SeqCst);
    DOWNLOAD_CANCEL.store(true, Ordering::SeqCst);
    let mut guard = ENGINE.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_some() {
        logger::log_info("Frigör LLM-motor och GPU-resurser inför avslut…");
        *guard = None;
        sync_loaded_slot(&guard);
        logger::log_info("LLM-motor frigjord rent.");
    }
}

fn sync_loaded_slot(guard: &Option<SummarizerEngine>) {
    let slot = match guard.as_ref().map(|e| e.model_id.as_str()) {
        Some("e2b") => SLOT_E2B,
        Some("e4b") => SLOT_E4B,
        Some("12b") => SLOT_12B,
        _ => SLOT_NONE,
    };
    LOADED_MODEL_SLOT.store(slot, Ordering::SeqCst);
}

fn loaded_model_id_lockfree() -> Option<String> {
    match LOADED_MODEL_SLOT.load(Ordering::SeqCst) {
        SLOT_E2B => Some("e2b".to_string()),
        SLOT_E4B => Some("e4b".to_string()),
        SLOT_12B => Some("12b".to_string()),
        _ => None,
    }
}

// ── Modellstatus & Nedladdningsstrukturer ────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelEntryStatus {
    pub id: String,
    pub label: String,
    pub short_label: String,
    pub description: String,
    pub filename: String,
    pub expected_bytes: u64,
    pub file_size_bytes: u64,
    pub recommended_min_ram_gb: u32,
    pub installed: bool,
    pub loaded: bool,
    /// Var modellen hittades: `"loki"` | `"maskera"` | `"portable"` | `""`
    pub source: String,
    pub can_delete: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelsOverview {
    pub active_model_id: String,
    pub loaded_model_id: Option<String>,
    pub any_installed: bool,
    pub total_ram_gb: f64,
    pub safe_ctx_tokens: u32,
    pub models: Vec<ModelEntryStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelDownloadProgress {
    pub model_id: String,
    pub phase: String, // "downloading" | "copying" | "loading" | "ready"
    pub label: String,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
}

// ── Lokal JSON-persistens (100 % på användarens egen dator) ──────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CustomTemplateSampling {
    pub temperature: Option<f32>,
    pub repeat_penalty: Option<f32>,
    pub presence_penalty: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredCustomTemplate {
    pub id: String,
    pub label: String,
    pub description: String,
    pub system_prompt: String,
    pub sampling: Option<CustomTemplateSampling>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedContextPreset {
    pub id: String,
    pub label: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub id: String,
    pub created_at: String,
    pub meeting_name: String,
    pub template_id: String,
    pub template_label: String,
    pub model_label: String,
    pub markdown: String,
    pub context: Option<String>,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalAppState {
    #[serde(default = "default_model_id")]
    pub active_model_id: String,
    #[serde(default)]
    pub custom_templates: Vec<StoredCustomTemplate>,
    #[serde(default)]
    pub saved_contexts: Vec<SavedContextPreset>,
    #[serde(default)]
    pub history: Vec<HistoryEntry>,
}

fn default_model_id() -> String {
    "e2b".to_string()
}

impl Default for LocalAppState {
    fn default() -> Self {
        Self {
            active_model_id: default_model_id(),
            custom_templates: Vec::new(),
            saved_contexts: Vec::new(),
            history: Vec::new(),
        }
    }
}

// ── Katalog- & Modellsökvägar (inkl. delning med Maskera!) ───────────────────

fn app_data_root(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Kan inte hitta app-datakatalog: {e}"))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("Kan inte skapa app-datakatalog: {e}"))?;
    Ok(dir)
}

fn loki_models_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app_data_root(app)?.join("models");
    std::fs::create_dir_all(&dir).map_err(|e| format!("Kan inte skapa modellkatalog: {e}"))?;
    Ok(dir)
}

fn state_file_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app_data_root(app)?.join("loki_state.json"))
}

fn read_state_unlocked(app: &AppHandle) -> LocalAppState {
    let Ok(path) = state_file_path(app) else {
        return LocalAppState::default();
    };
    if !path.exists() {
        return LocalAppState::default();
    }
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str::<LocalAppState>(&s).ok())
        .unwrap_or_default()
}

fn write_state_unlocked(app: &AppHandle, state: &LocalAppState) -> Result<(), String> {
    let path = state_file_path(app)?;
    let tmp = path.with_extension("json.tmp");
    let json = serde_json::to_string_pretty(state)
        .map_err(|e| format!("Kunde inte serialisera lokalt tillstånd: {e}"))?;
    std::fs::write(&tmp, json.as_bytes())
        .map_err(|e| format!("Kunde inte skriva lokal inställningsfil: {e}"))?;
    if std::fs::rename(&tmp, &path).is_err() {
        std::fs::copy(&tmp, &path)
            .map_err(|e| format!("Kunde inte uppdatera lokal inställningsfil: {e}"))?;
        let _ = std::fs::remove_file(&tmp);
    }
    Ok(())
}

fn is_valid_gguf_for_spec(path: &Path, spec: &ModelSpec) -> Option<u64> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.is_file() && meta.len() >= spec.min_valid_bytes {
        Some(meta.len())
    } else {
        None
    }
}

struct ResolvedModelPath {
    path: PathBuf,
    bytes: u64,
    source: &'static str, // "loki" | "portable" | "maskera"
}

/// Letar efter modellfilen i:
/// 1. Lokis egen `app_data_dir/models`
/// 2. Portabel mapp bredvid körbar fil (`{exe_dir}/models`) eller `resource_dir/models`
/// 3. Systerappen Maskeras modellmapp (`com.patriksvensk.maskera/models/judge` eller `models`)
///    -> ger 0 GB dubbelnedladdning om Gemma 4 E2B redan installerats i Maskera!
fn resolve_model_file(app: &AppHandle, spec: &ModelSpec) -> Option<ResolvedModelPath> {
    let legacy_alt = match spec.id {
        "12b" => Some("google_gemma-4-12B-it-Q4_K_M.gguf"),
        "e4b" => Some("google_gemma-4-E4B-it-Q4_K_M.gguf"),
        "e2b" => Some("google_gemma-4-E2B-it-Q4_K_M.gguf"),
        _ => None,
    };

    if let Ok(ldir) = loki_models_dir(app) {
        let p = ldir.join(spec.filename);
        if let Some(bytes) = is_valid_gguf_for_spec(&p, spec) {
            return Some(ResolvedModelPath {
                path: p,
                bytes,
                source: "loki",
            });
        }
        if let Some(alt_name) = legacy_alt {
            let alt_p = ldir.join(alt_name);
            if let Some(bytes) = is_valid_gguf_for_spec(&alt_p, spec) {
                return Some(ResolvedModelPath {
                    path: alt_p,
                    bytes,
                    source: "loki",
                });
            }
        }
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            for cand in [
                parent.join("models").join(spec.filename),
                parent.join("models").join("judge").join(spec.filename),
            ] {
                if let Some(bytes) = is_valid_gguf_for_spec(&cand, spec) {
                    return Some(ResolvedModelPath {
                        path: cand,
                        bytes,
                        source: "portable",
                    });
                }
            }
        }
    }

    if let Ok(res_dir) = app.path().resource_dir() {
        let cand = res_dir.join("models").join(spec.filename);
        if let Some(bytes) = is_valid_gguf_for_spec(&cand, spec) {
            return Some(ResolvedModelPath {
                path: cand,
                bytes,
                source: "portable",
            });
        }
    }

    // Kontrollera om systerappen Maskera redan har laddat ner samma QAT GGUF-modell
    if let Ok(data_dir) = app.path().app_data_dir() {
        if let Some(apps_parent) = data_dir.parent() {
            let maskera_root = apps_parent.join("com.patriksvensk.maskera").join("models");
            for cand in [
                maskera_root.join("judge").join(spec.filename),
                maskera_root.join(spec.filename),
            ] {
                if let Some(bytes) = is_valid_gguf_for_spec(&cand, spec) {
                    return Some(ResolvedModelPath {
                        path: cand,
                        bytes,
                        source: "maskera",
                    });
                }
            }
        }
    }

    None
}

fn build_models_overview(app: &AppHandle) -> ModelsOverview {
    let loaded_id = loaded_model_id_lockfree();

    let state = {
        let _g = STATE_LOCK.lock().ok();
        read_state_unlocked(app)
    };

    let mut entries = Vec::with_capacity(ALL_MODELS.len());
    for spec in ALL_MODELS {
        let resolved = resolve_model_file(app, spec);
        let is_loaded = loaded_id.as_deref() == Some(spec.id);
        let installed = is_loaded || resolved.is_some();
        let file_size_bytes = resolved.as_ref().map(|r| r.bytes).unwrap_or(0);
        let source = resolved.as_ref().map(|r| r.source).unwrap_or("");
        let can_delete = source == "loki";

        entries.push(ModelEntryStatus {
            id: spec.id.to_string(),
            label: spec.label.to_string(),
            short_label: spec.short_label.to_string(),
            description: spec.description.to_string(),
            filename: spec.filename.to_string(),
            expected_bytes: spec.expected_bytes,
            file_size_bytes,
            recommended_min_ram_gb: spec.recommended_min_ram_gb,
            installed,
            loaded: is_loaded,
            source: source.to_string(),
            can_delete,
        });
    }

    let any_installed = entries.iter().any(|m| m.installed);

    // Om vald aktiv modell inte är installerad men den andra modellen är installerad, peka på den installerade
    let mut active_id = state.active_model_id;
    let active_installed = entries
        .iter()
        .find(|m| m.id == active_id)
        .map(|m| m.installed)
        .unwrap_or(false);
    if !active_installed {
        if let Some(first_inst) = entries.iter().find(|m| m.installed) {
            active_id = first_inst.id.clone();
        } else {
            active_id = "e2b".to_string();
        }
    }

    let active_bytes = entries
        .iter()
        .find(|m| m.id == active_id)
        .map(|m| {
            if m.file_size_bytes > 0 {
                m.file_size_bytes
            } else {
                m.expected_bytes
            }
        })
        .unwrap_or(MODEL_E2B.expected_bytes);

    let total_ram_gb = summarizer::detect_total_ram_gb();
    let safe_ctx_tokens = summarizer::safe_ctx_ceiling(total_ram_gb, active_bytes);

    ModelsOverview {
        active_model_id: active_id,
        loaded_model_id: loaded_id,
        any_installed,
        total_ram_gb,
        safe_ctx_tokens,
        models: entries,
    }
}

// ── Tauri-kommandon: Modeller ────────────────────────────────────────────────

#[tauri::command]
fn get_models_status(app: AppHandle) -> Result<ModelsOverview, String> {
    Ok(build_models_overview(&app))
}

#[tauri::command]
fn cancel_model_download() {
    DOWNLOAD_CANCEL.store(true, Ordering::SeqCst);
}

#[tauri::command]
async fn select_active_model(model_id: String, app: AppHandle) -> Result<ModelsOverview, String> {
    let spec = get_model_spec(&model_id)
        .ok_or_else(|| format!("Okänd modellidentifierare: {model_id}"))?;

    {
        let _g = STATE_LOCK
            .lock()
            .map_err(|e| format!("Låsfel för inställningar: {e}"))?;
        let mut st = read_state_unlocked(&app);
        st.active_model_id = spec.id.to_string();
        write_state_unlocked(&app, &st)?;
    }

    if let Some(resolved) = resolve_model_file(&app, spec) {
        let app_c = app.clone();
        tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
            let mut guard = ENGINE
                .lock()
                .map_err(|e| format!("Låsfel för LLM-motor: {e}"))?;
            let already_loaded = guard
                .as_ref()
                .map(|e| e.model_id == spec.id)
                .unwrap_or(false);
            if !already_loaded {
                // Släpp först eventuell annan modell ur RAM innan den nya mmap-laddas
                *guard = None;
                sync_loaded_slot(&guard);
                let _ = app_c.emit(
                    "model_download_progress",
                    ModelDownloadProgress {
                        model_id: spec.id.to_string(),
                        phase: "loading".to_string(),
                        label: format!("Laddar in {} i arbetsminnet…", spec.short_label),
                        downloaded_bytes: resolved.bytes,
                        total_bytes: resolved.bytes,
                    },
                );
                let eng = SummarizerEngine::load(spec, &resolved.path)?;
                *guard = Some(eng);
                sync_loaded_slot(&guard);
            }
            Ok(())
        })
        .await
        .map_err(|e| format!("Trådfel vid modellbyte: {e}"))??;
    }

    Ok(build_models_overview(&app))
}

#[tauri::command]
async fn download_model(model_id: String, app: AppHandle) -> Result<ModelsOverview, String> {
    use futures_util::StreamExt;

    let spec = get_model_spec(&model_id)
        .ok_or_else(|| format!("Okänd modellidentifierare: {model_id}"))?;

    DOWNLOAD_CANCEL.store(false, Ordering::SeqCst);

    // Om filen redan finns (t.ex. via Maskera eller lokal mapp), aktivera och ladda den direkt!
    if resolve_model_file(&app, spec).is_some() {
        return select_active_model(model_id, app).await;
    }

    let dest_dir = loki_models_dir(&app)?;
    let dest_path = dest_dir.join(spec.filename);
    let tmp_path = dest_dir.join(format!("{}.part", spec.filename));

    let _ = app.emit(
        "model_download_progress",
        ModelDownloadProgress {
            model_id: spec.id.to_string(),
            phase: "downloading".to_string(),
            label: format!("Ansluter för att hämta {}…", spec.label),
            downloaded_bytes: 0,
            total_bytes: spec.expected_bytes,
        },
    );

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3600))
        .build()
        .map_err(|e| format!("Kunde inte starta HTTP-klient: {e}"))?;

    let resp = client
        .get(spec.hf_url)
        .send()
        .await
        .map_err(|e| format!("Nätverksfel vid hämtning av {}: {e}", spec.label))?;

    if !resp.status().is_success() {
        return Err(format!(
            "Servern svarade med HTTP {} vid hämtning av {}.",
            resp.status(),
            spec.filename
        ));
    }

    let total_bytes = resp.content_length().unwrap_or(spec.expected_bytes);
    let mut file = std::fs::File::create(&tmp_path)
        .map_err(|e| format!("Kunde inte skapa temporär fil på disk: {e}"))?;

    let mut downloaded: u64 = 0;
    let mut stream = resp.bytes_stream();
    let mut last_emit = Instant::now()
        .checked_sub(Duration::from_millis(100))
        .unwrap_or_else(Instant::now);

    while let Some(chunk_res) = stream.next().await {
        if DOWNLOAD_CANCEL.load(Ordering::Relaxed) {
            drop(file);
            let _ = std::fs::remove_file(&tmp_path);
            return Err("AVBRUTEN".to_string());
        }

        let chunk =
            chunk_res.map_err(|e| format!("Avbrott i nätverksströmmen för {}: {e}", spec.label))?;
        file.write_all(&chunk)
            .map_err(|e| format!("Kunde inte skriva till disk (kontrollera ledigt utrymme): {e}"))?;
        downloaded += chunk.len() as u64;

        if last_emit.elapsed() >= Duration::from_millis(45) {
            last_emit = Instant::now();
            let _ = app.emit(
                "model_download_progress",
                ModelDownloadProgress {
                    model_id: spec.id.to_string(),
                    phase: "downloading".to_string(),
                    label: format!("Hämtar {}…", spec.label),
                    downloaded_bytes: downloaded,
                    total_bytes: total_bytes.max(downloaded),
                },
            );
        }
    }

    drop(file);
    if std::fs::rename(&tmp_path, &dest_path).is_err() {
        std::fs::copy(&tmp_path, &dest_path)
            .map_err(|e| format!("Kunde inte slutföra modellfilen på disk: {e}"))?;
        let _ = std::fs::remove_file(&tmp_path);
    }

    select_active_model(spec.id.to_string(), app).await
}

/// Offline-import av GGUF-modell(er) från en lokal mapp eller fil (t.ex. USB-sticka eller IT-utdelning).
#[tauri::command]
async fn import_model_from_path(src_path: String, app: AppHandle) -> Result<ModelsOverview, String> {
    let root = PathBuf::from(&src_path);
    if !root.exists() {
        return Err("Den valda filen eller mappen finns inte.".to_string());
    }

    let dest_dir = loki_models_dir(&app)?;
    let mut matched: Vec<(&'static ModelSpec, PathBuf)> = Vec::new();

    if root.is_file() {
        let fname_lower = root
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if fname_lower.contains("12b") && is_valid_gguf_for_spec(&root, &MODEL_12B).is_some() {
            matched.push((&MODEL_12B, root.clone()));
        } else if fname_lower.contains("e4b") && is_valid_gguf_for_spec(&root, &MODEL_E4B).is_some()
        {
            matched.push((&MODEL_E4B, root.clone()));
        } else if is_valid_gguf_for_spec(&root, &MODEL_E2B).is_some() {
            matched.push((&MODEL_E2B, root.clone()));
        }
    } else {
        let search_dirs = [
            root.clone(),
            root.join("models"),
            root.join("models").join("judge"),
        ];
        for spec in ALL_MODELS {
            for d in &search_dirs {
                let exact = d.join(spec.filename);
                if is_valid_gguf_for_spec(&exact, spec).is_some() {
                    matched.push((spec, exact));
                    break;
                }
            }
        }
    }

    if matched.is_empty() {
        return Err(format!(
            "Hittade varken {}, {} eller {} i den valda platsen.",
            MODEL_E2B.filename, MODEL_E4B.filename, MODEL_12B.filename
        ));
    }

    let chosen_id = matched[0].0.id.to_string();
    let app_c = app.clone();

    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        for (spec, src_file) in matched {
            let dst = dest_dir.join(spec.filename);
            if src_file != dst {
                let _ = app_c.emit(
                    "model_download_progress",
                    ModelDownloadProgress {
                        model_id: spec.id.to_string(),
                        phase: "copying".to_string(),
                        label: format!("Kopierar {} från lokal mapp…", spec.label),
                        downloaded_bytes: 0,
                        total_bytes: spec.expected_bytes,
                    },
                );
                std::fs::copy(&src_file, &dst)
                    .map_err(|e| format!("Kunde inte kopiera {}: {e}", spec.filename))?;
            }
        }
        Ok(())
    })
    .await
    .map_err(|e| format!("Trådfel vid modellimport: {e}"))??;

    select_active_model(chosen_id, app).await
}

#[tauri::command]
async fn delete_model(model_id: String, app: AppHandle) -> Result<ModelsOverview, String> {
    let spec = get_model_spec(&model_id)
        .ok_or_else(|| format!("Okänd modellidentifierare: {model_id}"))?;

    DOWNLOAD_CANCEL.store(true, Ordering::SeqCst);

    // Frigör alltid modellen ur RAM först så att operativsystemet släpper mmap-lås på Windows/macOS!
    {
        let mut guard = ENGINE
            .lock()
            .map_err(|e| format!("Låsfel vid avinstallation av modell: {e}"))?;
        if guard.as_ref().map(|e| e.model_id == spec.id).unwrap_or(false) {
            *guard = None;
            sync_loaded_slot(&guard);
        }
    }

    if let Ok(ldir) = loki_models_dir(&app) {
        let main_file = ldir.join(spec.filename);
        let part_file = ldir.join(format!("{}.part", spec.filename));
        if main_file.exists() {
            std::fs::remove_file(&main_file)
                .map_err(|e| format!("Kunde inte ta bort modellfilen på disk: {e}"))?;
        }
        if part_file.exists() {
            let _ = std::fs::remove_file(&part_file);
        }
    }

    Ok(build_models_overview(&app))
}

// ── Tauri-kommandon: Transkripttvätt, Dokumentimport & Export ────────────────

#[tauri::command]
fn clean_transcript_text(
    raw: String,
    file_ext: Option<String>,
) -> transcript::CleanedTranscript {
    let mut cleaned = transcript::clean_transcript(&raw, file_ext.as_deref());
    // Om modellen redan ligger ledig i RAM, förfina tokenuppskattningen med exakt vokabulärräkning
    if !cleaned.text.is_empty() {
        if let Ok(guard) = ENGINE.try_lock() {
            if let Some(eng) = guard.as_ref() {
                cleaned.stats.estimated_tokens = eng.count_tokens(&cleaned.text);
            }
        }
    }
    cleaned
}

#[tauri::command]
async fn import_document_bytes(
    file_name: String,
    bytes: Vec<u8>,
    clean_as_transcript: bool,
) -> Result<documents::ImportedDocument, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut doc = documents::import_document_bytes(&file_name, &bytes, clean_as_transcript)?;
        if let Some(ref mut stats) = doc.transcript_stats {
            if !doc.text.is_empty() {
                if let Ok(guard) = ENGINE.try_lock() {
                    if let Some(eng) = guard.as_ref() {
                        stats.estimated_tokens = eng.count_tokens(&doc.text);
                    }
                }
            }
        }
        Ok(doc)
    })
    .await
    .map_err(|e| format!("Trådfel vid dokumentinläsning: {e}"))?
}

#[tauri::command]
async fn export_docx_file(
    path: String,
    title: String,
    subtitle: Option<String>,
    markdown: String,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let docx_bytes =
            documents::export_markdown_to_docx_bytes(&title, subtitle.as_deref(), &markdown)?;
        std::fs::write(&path, docx_bytes)
            .map_err(|e| format!("Kunde inte spara Word-filen (.docx): {e}"))
    })
    .await
    .map_err(|e| format!("Trådfel vid Word-export: {e}"))?
}

#[tauri::command]
fn write_text_file(path: String, content: String) -> Result<(), String> {
    std::fs::write(&path, content.as_bytes())
        .map_err(|e| format!("Kunde inte spara filen: {e}"))
}

// ── Tauri-kommandon: Sammanfattning & Justering (In-Process Streaming) ───────

#[tauri::command]
fn cancel_summarize() {
    SUMMARIZE_CANCEL.store(true, Ordering::SeqCst);
}

fn ensure_engine_for_model(
    guard: &mut Option<SummarizerEngine>,
    app: &AppHandle,
    requested_model_id: Option<&str>,
) -> Result<(), String> {
    // OBS: Anropa ALDRIG något som låser `ENGINE` här eftersom `guard` redan håller `ENGINE.lock()`!
    let saved_active_id = {
        let _g = STATE_LOCK.lock().ok();
        read_state_unlocked(app).active_model_id
    };
    let target_id = requested_model_id.unwrap_or(saved_active_id.as_str());
    let spec = get_model_spec(target_id).unwrap_or(&MODEL_E2B);

    let already_loaded = guard
        .as_ref()
        .map(|e| e.model_id == spec.id)
        .unwrap_or(false);
    if already_loaded {
        return Ok(());
    }

    let (load_spec, resolved) = resolve_model_file(app, spec)
        .map(|r| (spec, r))
        .or_else(|| {
            // Om önskad modell inte finns, fall tillbaka på första modell som faktiskt är installerad
            ALL_MODELS
                .iter()
                .find_map(|alt| resolve_model_file(app, alt).map(|r| (alt, r)))
        })
        .ok_or_else(|| {
            "Ingen lokal språkmodell är installerad ännu. Hämta Gemma 4 E2B i startkortet först."
                .to_string()
        })?;

    *guard = None;
    sync_loaded_slot(guard);
    let _ = app.emit(
        "summarize_progress",
        summarizer::SummarizeProgressPayload {
            phase: "loading_model".to_string(),
            label: format!("Laddar in {} i minnet…", load_spec.short_label),
            prefill_percent: 0,
            kv_reused_tokens: 0,
            prompt_tokens: 0,
            thinking_tokens: 0,
            generated_tokens: 0,
            tokens_per_sec: 0.0,
            chunk_current: None,
            chunk_total: None,
        },
    );

    let eng = SummarizerEngine::load(load_spec, &resolved.path)?;
    *guard = Some(eng);
    sync_loaded_slot(guard);
    Ok(())
}

#[tauri::command]
async fn summarize_meeting(
    request: SummarizeRequest,
    app: AppHandle,
) -> Result<SummarizeResult, String> {
    SUMMARIZE_CANCEL.store(false, Ordering::SeqCst);

    logger::log_info(&format!(
        "summarize_meeting anropad (mall='{}', längd={} tecken)",
        request.template_id,
        request.transcript.chars().count()
    ));

    tauri::async_runtime::spawn_blocking(move || -> Result<SummarizeResult, String> {
        if loaded_model_id_lockfree().is_none() {
            let _ = app.emit(
                "summarize_progress",
                summarizer::SummarizeProgressPayload {
                    phase: "loading_model".to_string(),
                    label: "Laddar in språkmodellen i arbetsminnet…".to_string(),
                    prefill_percent: 0,
                    kv_reused_tokens: 0,
                    prompt_tokens: 0,
                    thinking_tokens: 0,
                    generated_tokens: 0,
                    tokens_per_sec: 0.0,
                    chunk_current: None,
                    chunk_total: None,
                },
            );
        }
        logger::log_info("Väntar på ENGINE-lås…");
        let mut guard = ENGINE
            .lock()
            .map_err(|e| format!("Låsfel för LLM-motor: {e}"))?;
        logger::log_info("ENGINE-lås erhållet.");

        ensure_engine_for_model(&mut guard, &app, request.model_id.as_deref())?;

        let engine = guard
            .as_mut()
            .ok_or_else(|| "Språkmodellen kunde inte initieras.".to_string())?;

        let res = engine.run_summarize(
            &request,
            &SUMMARIZE_CANCEL,
            |prog| {
                let _ = app.emit("summarize_progress", prog);
            },
            |chunk| {
                let _ = app.emit("summarize_chunk", chunk);
            },
        );
        match &res {
            Ok(r) => logger::log_info(&format!(
                "summarize_meeting klar: {} tokens på {} ms ({:.1} tok/s, strategi={})",
                r.generated_tokens, r.duration_ms, r.tokens_per_sec, r.strategy
            )),
            Err(e) => logger::log_warn(&format!("summarize_meeting avslutad med: {e}")),
        }
        res
    })
    .await
    .map_err(|e| format!("Trådfel under sammanfattning: {e}"))?
}

#[tauri::command]
async fn adjust_meeting_result(
    current_markdown: String,
    directive: String,
    model_id: Option<String>,
    app: AppHandle,
) -> Result<SummarizeResult, String> {
    SUMMARIZE_CANCEL.store(false, Ordering::SeqCst);

    tauri::async_runtime::spawn_blocking(move || -> Result<SummarizeResult, String> {
        let mut guard = ENGINE
            .lock()
            .map_err(|e| format!("Låsfel för LLM-motor: {e}"))?;

        ensure_engine_for_model(&mut guard, &app, model_id.as_deref())?;

        let engine = guard
            .as_mut()
            .ok_or_else(|| "Språkmodellen kunde inte initieras.".to_string())?;

        engine.run_adjust(
            &current_markdown,
            &directive,
            &SUMMARIZE_CANCEL,
            |prog| {
                let _ = app.emit("summarize_progress", prog);
            },
            |chunk| {
                let _ = app.emit("summarize_chunk", chunk);
            },
        )
    })
    .await
    .map_err(|e| format!("Trådfel vid dokumentjustering: {e}"))?
}

// ── Tauri-kommandon: Lokal Historik, Egna Mallar & Sparade Sammanhang ────────

#[tauri::command]
fn load_local_state(app: AppHandle) -> Result<LocalAppState, String> {
    let _g = STATE_LOCK
        .lock()
        .map_err(|e| format!("Låsfel för lokalt tillstånd: {e}"))?;
    Ok(read_state_unlocked(&app))
}

#[tauri::command]
fn save_custom_templates(
    templates: Vec<StoredCustomTemplate>,
    app: AppHandle,
) -> Result<(), String> {
    let _g = STATE_LOCK
        .lock()
        .map_err(|e| format!("Låsfel för lokalt tillstånd: {e}"))?;
    let mut state = read_state_unlocked(&app);
    state.custom_templates = templates;
    write_state_unlocked(&app, &state)
}

#[tauri::command]
fn save_context_presets(
    contexts: Vec<SavedContextPreset>,
    app: AppHandle,
) -> Result<(), String> {
    let _g = STATE_LOCK
        .lock()
        .map_err(|e| format!("Låsfel för lokalt tillstånd: {e}"))?;
    let mut state = read_state_unlocked(&app);
    state.saved_contexts = contexts;
    write_state_unlocked(&app, &state)
}

#[tauri::command]
fn save_history_entry(entry: HistoryEntry, app: AppHandle) -> Result<Vec<HistoryEntry>, String> {
    let _g = STATE_LOCK
        .lock()
        .map_err(|e| format!("Låsfel för lokalt tillstånd: {e}"))?;
    let mut state = read_state_unlocked(&app);
    if let Some(existing) = state.history.iter_mut().find(|h| h.id == entry.id) {
        *existing = entry;
    } else {
        state.history.insert(0, entry);
        if state.history.len() > 150 {
            state.history.truncate(150);
        }
    }
    write_state_unlocked(&app, &state)?;
    Ok(state.history)
}

#[tauri::command]
fn delete_history_entry(id: String, app: AppHandle) -> Result<Vec<HistoryEntry>, String> {
    let _g = STATE_LOCK
        .lock()
        .map_err(|e| format!("Låsfel för lokalt tillstånd: {e}"))?;
    let mut state = read_state_unlocked(&app);
    state.history.retain(|h| h.id != id);
    write_state_unlocked(&app, &state)?;
    Ok(state.history)
}

#[tauri::command]
fn clear_all_history(app: AppHandle) -> Result<(), String> {
    let _g = STATE_LOCK
        .lock()
        .map_err(|e| format!("Låsfel för lokalt tillstånd: {e}"))?;
    let mut state = read_state_unlocked(&app);
    state.history.clear();
    write_state_unlocked(&app, &state)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogInfoPayload {
    pub path: String,
    pub content: String,
    pub accelerator: String,
}

#[tauri::command]
fn get_log_info() -> LogInfoPayload {
    let path = logger::log_file_path()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "(okänd)".to_string());
    let content = logger::read_recent_logs(250);
    LogInfoPayload {
        path,
        content,
        accelerator: summarizer::accelerator_label().to_string(),
    }
}

#[tauri::command]
fn clear_log_file() -> Result<(), String> {
    logger::clear_logs()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // Spara loki.log i exakt samma OS-loggkatalog som Loki 2.9 (`%LOCALAPPDATA%\com.loki.app\logs\` på Windows)
            let log_dir = app
                .path()
                .app_log_dir()
                .or_else(|_| app.path().app_data_dir());
            if let Ok(dir) = log_dir {
                logger::init_logger(dir);
                logger::log_info(&format!(
                    "Loki {} startad ({}-{}, accelerator={}, RAM={:.1} GB)",
                    env!("CARGO_PKG_VERSION"),
                    std::env::consts::OS,
                    std::env::consts::ARCH,
                    summarizer::accelerator_label(),
                    summarizer::detect_total_ram_gb()
                ));
            }

            // Värm upp språkmodellen i bakgrunden direkt vid uppstart om den redan finns på disk (t.ex. via Maskera!)
            let handle = app.handle().clone();
            tauri::async_runtime::spawn_blocking(move || {
                if APP_EXITING.load(Ordering::SeqCst) {
                    return;
                }
                let ov = build_models_overview(&handle);
                if ov.any_installed && !APP_EXITING.load(Ordering::SeqCst) {
                    logger::log_info(&format!(
                        "Förladdar aktiv språkmodell ({}) i bakgrunden…",
                        ov.active_model_id
                    ));
                    if let Ok(mut guard) = ENGINE.lock() {
                        if APP_EXITING.load(Ordering::SeqCst) {
                            return;
                        }
                        if let Err(e) =
                            ensure_engine_for_model(&mut guard, &handle, Some(&ov.active_model_id))
                        {
                            logger::log_warn(&format!("Bakgrundsförladdning hoppades över: {e}"));
                        } else {
                            let _ = handle.emit("models_status_changed", build_models_overview(&handle));
                        }
                    }
                } else {
                    logger::log_info("Ingen språkmodell installerad vid uppstart.");
                }
            });

            Ok(())
        })
        .on_window_event(|_window, event| {
            if matches!(
                event,
                tauri::WindowEvent::CloseRequested { .. } | tauri::WindowEvent::Destroyed
            ) {
                shutdown_engine();
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_models_status,
            select_active_model,
            download_model,
            cancel_model_download,
            import_model_from_path,
            delete_model,
            clean_transcript_text,
            import_document_bytes,
            export_docx_file,
            write_text_file,
            summarize_meeting,
            adjust_meeting_result,
            cancel_summarize,
            load_local_state,
            save_custom_templates,
            save_context_presets,
            save_history_entry,
            delete_history_entry,
            clear_all_history,
            get_log_info,
            clear_log_file,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Loki application")
        .run(|_app_handle, event| {
            if matches!(
                event,
                tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit
            ) {
                shutdown_engine();
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loaded_slot_is_lockfree_while_engine_mutex_is_held() {
        let guard = ENGINE.lock().expect("lock ENGINE");
        sync_loaded_slot(&guard);
        // Måste kunna läsas utan att ta ENGINE.lock() på samma tråd (förhindrar re-entrant deadlock!)
        assert_eq!(loaded_model_id_lockfree(), None);
        drop(guard);
    }
}
