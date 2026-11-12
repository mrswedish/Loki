use std::num::NonZeroU32;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::context::LlamaContext;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::LlamaModel;
use llama_cpp_2::sampling::LlamaSampler;
use llama_cpp_2::token::LlamaToken;
use serde::{Deserialize, Serialize};
use sysinfo::System;

pub const CANCELLED_ERR: &str = "AVBRUTEN";

/// Max kontextfönster för Gemma 4 (128K tokens).
pub const MAX_CTX: u32 = 131_072;
/// Minsta standardfönster så att vanliga möten aldrig kräver omallokering av kontext.
const DEFAULT_MIN_CTX: u32 = 16_384;
/// Batchstorlek vid prompt-inläsning (balanserar hög hastighet med responsivt avbrott < 50 ms).
const PREFILL_BATCH: usize = 1024;
const PREFILL_UBATCH: u32 = 512;
/// Reserverat utrymme för modellens svar i normalt respektive "Noggrannare"-läge.
const RESPONSE_MARGIN_FAST: u32 = 4096;
const RESPONSE_MARGIN_THOROUGH: u32 = 8192;
/// Max antal tanketokens i "Noggrannare"-läget innan `</thought>` (`\n<channel|>`) injiceras i KV-cachen.
const MAX_THOUGHT_TOKENS: usize = 1536;
/// Max genererade svarstokens per pass.
const MAX_ANSWER_TOKENS: usize = 4096;
/// Konservativ KV-cache-kostnad i GB per token (~0,03 MB/token).
const KV_GB_PER_TOKEN: f64 = 0.00003;

#[derive(Debug, Clone, Copy)]
pub struct ModelSpec {
    pub id: &'static str,
    pub label: &'static str,
    pub short_label: &'static str,
    pub description: &'static str,
    pub filename: &'static str,
    pub hf_url: &'static str,
    pub expected_bytes: u64,
    pub min_valid_bytes: u64,
}

pub const MODEL_E2B: ModelSpec = ModelSpec {
    id: "e2b",
    label: "Gemma 4 E2B QAT (2,62 GB)",
    short_label: "E2B · Snabb",
    description: "Rekommenderad standard. Snabb och resurssnål med QAT-precision – utmärkt på vanliga bärbara datorer.",
    filename: "gemma-4-E2B-it-qat-UD-Q4_K_XL.gguf",
    hf_url: "https://huggingface.co/unsloth/gemma-4-E2B-it-qat-GGUF/resolve/main/gemma-4-E2B-it-qat-UD-Q4_K_XL.gguf",
    expected_bytes: 2_620_370_976,
    min_valid_bytes: 2_400_000_000,
};

pub const MODEL_E4B: ModelSpec = ModelSpec {
    id: "e4b",
    label: "Gemma 4 E4B QAT (4,22 GB)",
    short_label: "E4B · Fördjupad",
    description: "Större språkmodell för extra nyanserade protokoll och svårare fackterminologi.",
    filename: "gemma-4-E4B-it-qat-UD-Q4_K_XL.gguf",
    hf_url: "https://huggingface.co/unsloth/gemma-4-E4B-it-qat-GGUF/resolve/main/gemma-4-E4B-it-qat-UD-Q4_K_XL.gguf",
    expected_bytes: 4_215_695_776,
    min_valid_bytes: 3_900_000_000,
};

pub const ALL_MODELS: &[ModelSpec] = &[MODEL_E2B, MODEL_E4B];

pub fn get_model_spec(model_id: &str) -> Option<&'static ModelSpec> {
    ALL_MODELS.iter().find(|m| m.id == model_id)
}

// ── Domänprompter (bevarade och optimerade från Loki 2.9) ────────────────────

pub const COMMON_PREAMBLE: &str = "\
Du är expert på administrativ struktur och textförädling. Du bearbetar en rå, brusig \
transkribering av ett möte eller samtal till ett rent, begripligt dokument.\n\n\
KONTEXTUELL STÄDNING: Texten kommer från automatisk röstigenkänning och innehåller stora \
mängder fonetiska fel – programmet har skrivit ord som LÅTER som det som sades men blir fel \
i sammanhanget. Tillämpa följande generella regler:\n\
- LOGIKKONTROLL: Om ett ord eller en förkortning låter malplacerat i ett professionellt \
tjänstemannamöte (t.ex. \"raggningsarbete\", \"skolor\", \"kårkort\", \"DR\", \"IHL\"), stanna upp och \
fråga: vilket ord som LÅTER liknande är logiskt i denna verksamhetskontext? Ersätt med det \
logiska ordet.\n\
- FONETISKA ENGELSKA TERMER: Röstigenkänningen stavar ofta engelska IT- och facktermer som \
de låter på svenska. Känner du igen ett sådant förvanskat uttryck, återge den korrekta \
engelska/etablerade termen i stället för den svensk-fonetiska stavningen.\n\
- AVSIKT FRAMFÖR EXAKTA ORD: Fokusera på vad meningen betyder, inte de exakta orden. Är en \
mening trasig, bygg om den till korrekt, professionell svenska utifrån sammanhanget.\n\
- VID OSÄKERHET: kan du inte lista ut ett skevt ord, skriv en generell, neutral term \
(\"organisationen\", \"systemet\", \"en avdelning\", \"ett internt verktyg\") i stället för att föra \
vidare det trasiga ordet eller gissa fritt. Hitta INTE på en \"smart\" förklaring till ett ord \
du inte förstår.\n\
- Hitta ALDRIG på fakta, namn, beslut eller siffror som inte finns i texten.\n\n\
FILTRERA BORT KALLPRAT (STENHÅRT): Ta bort allt socialt småprat och tekniskt strul som inte \
hör till sakfrågorna – inledande prat om kameror, mikrofoner, ljud/bild, hälsningar, väder, \
helger, hälsa och privata anekdoter. Börja minnesanteckningen först när det faktiska arbetet \
eller agendan drar igång.\n\n\
KRONOLOGI: Följ mötet i den ordning det skedde, ämne för ämne.\n\n\
TROHET: Använd talarnas egna ord och termer. Byt INTE ut ord mot synonymer i onödan och \
använd inga metaforer eller tolkande omskrivningar som kan förvränga vad som sades. \
Sammanfatta vad diskussionen landade i – skriv inte av dialogen rakt av.\n\n\
Svara på samma språk som transkriberingen (oftast svenska). Skriv inga inledande eller \
avslutande kommentarer om uppgiften – leverera bara det begärda dokumentet i Markdown.";

pub const CLEANUP_PROMPT: &str = "\
Du är korrekturläsare för en rå transkribering från automatisk röstigenkänning (STT). \
Texten innehåller hörselfel, fonetiska felstavningar (särskilt engelska IT- och facktermer \
stavade som de låter på svenska) och avbrutna meningar.\n\n\
Din enda uppgift: korrigera språket utifrån sammanhanget.\n\
- Rätta uppenbara hörselfel och trasiga meningar till korrekt, professionell svenska.\n\
- Återge förvanskade engelska facktermer i sin korrekta form.\n\
- Kan du inte lista ut ett skevt ord, ersätt det med en neutral term hellre än att gissa \
fritt eller hitta på en förklaring.\n\n\
VIKTIGT: Detta är BARA ett korrektursteg. Sammanfatta INTE, ta INTE bort information, ändra \
INTE ordningen och lägg INTE till rubriker eller kommentarer. Behåll all text och dess \
struktur. Leverera den korrigerade texten rakt upp och ner.";

pub const CONTEXT_INSTRUCTION: &str = "\
SAMMANHANG: Detta möte rör följande verksamhet/domän (använd det BARA för att tolka \
oklara ord, fackuttryck och förkortningar rätt – det styr INTE vad som ska tas med och \
du får ALDRIG lägga till information som inte finns i transkriberingen):";

pub const CONTEXT_INSTRUCTION_CREATIVE: &str = "\
SAMMANHANG: Detta möte rör följande verksamhet/domän. Använd det för att tolka oklara ord \
och fackuttryck rätt, och låt domänen forma tonen och strukturen så att dokumentet känns \
naturligt för sammanhanget. Du får formulera dig friare, MEN du får ALDRIG hitta på fakta, \
namn, beslut eller siffror som inte finns i transkriberingen:";

pub const ADJUST_PROMPT: &str = "\
Du får ett färdigt mötesdokument och en lista med justeringsdirektiv från användaren. \
Tillämpa ENDAST de begärda justeringarna på dokumentet:\n\
- Rätta feltolkade ord, egennamn och facktermer enligt direktiven (t.ex. om användaren \
säger att \"Kulturum\" ska vara \"Qulturum\", ersätt överallt).\n\
- Korrigera felaktiga sammankopplingar eller missförstånd som användaren pekar ut.\n\
VIKTIGT: Ändra inget annat. Lägg inte till, ta inte bort och tolka inte om fakta, beslut, \
namn, siffror eller åtgärder utöver det direktiven säger. Behåll dokumentets struktur och \
Markdown-format. Svara enbart med det justerade dokumentet – inga kommentarer.";

pub const REDUCE_PROMPT: &str = "\
Du får flera delsammanfattningar av olika avsnitt av samma möte, i kronologisk ordning \
(separerade med \"---\"). Slå ihop dem till ETT enda sammanhängande dokument.\n\n\
- Behåll kronologin strikt: avsnitten kommer i den ordning de inträffade.\n\
- Slå ihop redundans: om samma ämne, beslut eller åtgärd nämns i flera delar, ta med det \
en gång.\n\
- Bevara alla beslut, åtgärder, namn och siffror exakt.\n\
- Ignorera delar som bara säger \"Inget relevant i detta avsnitt\".\n\
- Använd tydliga underrubriker per ämne. Lägg inte till nytt innehåll och tolka inte om.\n\
Svara enbart med det sammanslagna dokumentet i Markdown.";

// ── Datastrukturer för körning & progress ────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SamplingOptions {
    pub temperature: Option<f32>,
    pub repeat_penalty: Option<f32>,
    pub presence_penalty: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SummarizeRequest {
    pub transcript: String,
    pub template_id: String,
    pub template_label: String,
    pub template_prompt: String,
    pub is_builtin_template: bool,
    pub context: Option<String>,
    pub agenda: Option<String>,
    pub creative: bool,
    pub thorough: bool,
    pub model_id: Option<String>,
    pub sampling: Option<SamplingOptions>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SummarizeProgressPayload {
    pub phase: String, // "loading_model" | "prefill" | "thinking" | "generating" | "reducing"
    pub label: String,
    pub prefill_percent: u8,
    pub kv_reused_tokens: usize,
    pub prompt_tokens: usize,
    pub thinking_tokens: usize,
    pub generated_tokens: usize,
    pub tokens_per_sec: f32,
    pub chunk_current: Option<usize>,
    pub chunk_total: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SummarizeChunkPayload {
    pub full_text: String,
    pub generated_tokens: usize,
    pub tokens_per_sec: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SummarizeResult {
    pub markdown: String,
    pub model_id: String,
    pub model_label: String,
    pub prompt_tokens: usize,
    pub kv_reused_tokens: usize,
    pub generated_tokens: usize,
    pub thinking_tokens: usize,
    pub duration_ms: u64,
    pub tokens_per_sec: f32,
    pub strategy: String, // "single" | "chunked"
    pub used_temperature: f32,
}

// ── LlamaBackend & Systemhjälpare ────────────────────────────────────────────

static LLAMA_BACKEND: OnceLock<Result<LlamaBackend, String>> = OnceLock::new();

pub fn ensure_llama_backend() -> Result<&'static LlamaBackend, String> {
    let res = LLAMA_BACKEND.get_or_init(|| {
        let mut backend = LlamaBackend::init()
            .map_err(|e| format!("Kunde inte initiera lokal LLM-motor (llama.cpp): {e}"))?;
        backend.void_logs();
        Ok(backend)
    });
    match res {
        Ok(b) => Ok(b),
        Err(e) => Err(e.clone()),
    }
}

/// Delad CPU-trådning: fysiska kärnor för minnesbunden textgenerering, högre parallellism för prompt-prefill.
pub fn optimal_cpu_threads() -> (i32, i32) {
    let logical = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    let gen_threads = if logical <= 4 {
        logical.max(1)
    } else {
        (logical / 2).clamp(4, 6)
    };
    let batch_threads = if logical <= 4 {
        logical.max(1)
    } else {
        ((logical * 3) / 4).clamp(4, 12)
    };
    (gen_threads as i32, batch_threads as i32)
}

pub fn round_ctx_up(tokens: u32) -> u32 {
    tokens.div_ceil(1024) * 1024
}

pub fn detect_total_ram_gb() -> f64 {
    let mut sys = System::new();
    sys.refresh_memory();
    let bytes = sys.total_memory();
    if bytes == 0 {
        16.0
    } else {
        (bytes as f64) / 1_073_741_824.0
    }
}

/// Beräknar säkert tak för kontextfönstret utifrån systemets RAM och modellfilens storlek.
pub fn safe_ctx_ceiling(total_ram_gb: f64, model_bytes: u64) -> u32 {
    if total_ram_gb <= 0.0 {
        return MAX_CTX;
    }
    let model_gb = (model_bytes as f64) / 1_000_000_000.0;
    let kv_budget_gb = (total_ram_gb - model_gb - 2.0).max(0.0);
    let raw_tokens = (kv_budget_gb / KV_GB_PER_TOKEN).floor() as u32;
    let rounded = round_ctx_up(raw_tokens.max(8192));
    rounded.clamp(8192, MAX_CTX)
}

// ── Prompt-uppbyggnad optimerad för Varm KV-cache över mallbyten ─────────────

/// Delar upp en malls prompt så att `COMMON_PREAMBLE` + möteskontext + hela transkriptet
/// hamnar i ett invariant prefix `[0 .. P2)`, medan själva malluppgiften och agendan ligger i `[P2 ..)`.
/// När användaren byter mall på samma möte (t.ex. Minnesanteckning -> Åtgärdspunkter) återanvänds
/// 99 % av kontexten direkt ur KV-cachen på < 0,3 s!
pub fn build_kv_friendly_prompt(req: &SummarizeRequest, map_mode_only: bool) -> String {
    let is_cleanup = req.template_id == "transcription-cleanup";

    let raw_tpl = req.template_prompt.trim();
    let stripped_task = raw_tpl
        .strip_prefix(COMMON_PREAMBLE)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(raw_tpl);

    let mut system_block = if is_cleanup {
        CLEANUP_PROMPT.to_string()
    } else {
        COMMON_PREAMBLE.to_string()
    };

    if let Some(ctx_str) = req.context.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        let ctx_hdr = if req.creative {
            CONTEXT_INSTRUCTION_CREATIVE
        } else {
            CONTEXT_INSTRUCTION
        };
        system_block.push_str("\n\n");
        system_block.push_str(ctx_hdr);
        system_block.push('\n');
        system_block.push_str(ctx_str);
    }

    let mut task_block = if is_cleanup {
        "Korrigera transkriberingen ovan enligt korrekturreglerna utan att sammanfatta eller utelämna något.".to_string()
    } else {
        stripped_task.to_string()
    };

    if map_mode_only {
        task_block.push_str(
            "\n\nOBS: Detta är ETT AVSNITT av en längre transkribering, inte hela mötet. \
             Sammanfatta bara innehållet i detta avsnitt. Om avsnittet enbart innehåller \
             kallprat eller socialt småprat utan relevans, svara exakt: \
             \"Inget relevant i detta avsnitt.\"",
        );
    } else if let Some(ag_str) = req.agenda.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        if !is_cleanup {
            task_block.push_str(
                "\n\nSTRUKTUR ENLIGT AGENDA: Strukturera dokumentet efter agendan nedan. Använd \
                 agendans punkter som rubriker i den ordning de står. Placera det som diskuterades \
                 under rätt agendapunkt. Punkter som togs upp men inte finns på agendan samlas under \
                 rubriken \"Övrigt\". Agendapunkter som inte berördes markeras kort som \"Ej behandlad\".\n\n\
                 --- AGENDA ---\n",
            );
            task_block.push_str(ag_str);
            task_block.push_str("\n--- SLUT AGENDA ---");
        }
    }

    let thought_turn = if req.thorough {
        "<|turn>model\n<|channel>thought\n"
    } else {
        "<|turn>model\n<|channel>thought\n<channel|>"
    };

    format!(
        "<bos><|turn>system\n{system_block}<turn|>\n<|turn>user\n--- MÖTESTRANSKRIBERING ---\n{transcript}\n--- SLUT PÅ MÖTESTRANSKRIBERING ---\n\n--- UPPGIFT ({label}) ---\n{task_block}<turn|>\n{thought_turn}",
        transcript = req.transcript.trim(),
        label = req.template_label.trim(),
    )
}

pub fn build_reduce_prompt(parts: &[String], req: &SummarizeRequest) -> String {
    let joined_parts = parts.join("\n\n---\n\n");
    let raw_tpl = req.template_prompt.trim();
    let stripped_task = raw_tpl
        .strip_prefix(COMMON_PREAMBLE)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(raw_tpl);

    let mut task_tail = format!("MÅLFORMAT ({}):\n{}", req.template_label, stripped_task);
    if let Some(ag_str) = req.agenda.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        task_tail.push_str(
            "\n\nSTRUKTUR ENLIGT AGENDA: Använd agendans punkter som rubriker i den ordning de står. \
             Punkter utanför agendan samlas under \"Övrigt\". Ej berörda punkter markeras \"Ej behandlad\".\n\n\
             --- AGENDA ---\n",
        );
        task_tail.push_str(ag_str);
        task_tail.push_str("\n--- SLUT AGENDA ---");
    }

    let thought_turn = if req.thorough {
        "<|turn>model\n<|channel>thought\n"
    } else {
        "<|turn>model\n<|channel>thought\n<channel|>"
    };

    format!(
        "<bos><|turn>system\n{REDUCE_PROMPT}<turn|>\n<|turn>user\n--- DELSAMMANFATTNINGAR I KRONOLOGISK ORDNING ---\n{joined_parts}\n--- SLUT PÅ DELSAMMANFATTNINGAR ---\n\n{task_tail}<turn|>\n{thought_turn}"
    )
}

pub fn build_adjust_prompt(current_markdown: &str, directive: &str) -> String {
    format!(
        "<bos><|turn>system\n{ADJUST_PROMPT}<turn|>\n<|turn>user\n--- NUVARANDE MÖTESDOKUMENT ---\n{}\n--- SLUT PÅ DOKUMENT ---\n\n--- JUSTERINGSDIREKTIV FRÅN ANVÄNDAREN ---\n{}\n--- SLUT PÅ DIREKTIV ---<turn|>\n<|turn>model\n<|channel>thought\n<channel|>",
        current_markdown.trim(),
        directive.trim()
    )
}

pub fn is_empty_chunk_response(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.chars().count() >= 90 {
        return false;
    }
    let cleaned = trimmed
        .trim_start_matches(['*', '_', '"', '\'', '`', ' '])
        .to_lowercase();
    cleaned.contains("inget relevant")
}

/// Delar upp en mycket lång transkribering vid stycke-/meningsgränser för map-reduce-fallback.
pub fn split_into_chunks(
    text: &str,
    token_budget: usize,
    chars_per_token: f64,
    overlap_tokens: usize,
) -> Vec<String> {
    let char_budget = ((token_budget as f64) * chars_per_token).floor().max(500.0) as usize;
    let overlap_chars = ((overlap_tokens as f64) * chars_per_token).floor().max(0.0) as usize;

    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= char_budget {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut start = 0usize;
    while start < chars.len() {
        let mut end = (start + char_budget).min(chars.len());
        if end < chars.len() {
            // Sök bakåt i de sista 35 % av fönstret efter stycke- eller meningsbrytning
            let min_break = start + (char_budget * 65 / 100);
            let mut best_break = None;
            for pos in (min_break..end).rev() {
                if pos + 1 < chars.len() && chars[pos] == '\n' && chars[pos + 1] == '\n' {
                    best_break = Some(pos + 1);
                    break;
                }
            }
            if best_break.is_none() {
                for pos in (min_break..end).rev() {
                    if (chars[pos] == '.' || chars[pos] == '!' || chars[pos] == '?')
                        && pos + 1 < chars.len()
                        && chars[pos + 1].is_whitespace()
                    {
                        best_break = Some(pos + 1);
                        break;
                    }
                }
            }
            if let Some(b) = best_break {
                end = b;
            }
        }

        let slice_str: String = chars[start..end].iter().collect();
        let trimmed = slice_str.trim();
        if !trimmed.is_empty() {
            chunks.push(trimmed.to_string());
        }
        if end >= chars.len() {
            break;
        }
        start = end.saturating_sub(overlap_chars).max(start + 1);
    }

    chunks
}

/// Säker UTF-8-avkodning av strömmande BPE-bytes: klipper eventuella ofullständiga avslutande
/// flerbytessekvenser (t.ex. halvt svenskt `å`/`ä`/`ö`) tills nästa token anländer.
fn valid_utf8_prefix(bytes: &[u8]) -> &str {
    match std::str::from_utf8(bytes) {
        Ok(s) => s,
        Err(e) => {
            let valid_len = e.valid_up_to();
            // SAFETY: `valid_up_to()` garanterar giltig UTF-8 upp till `valid_len`.
            unsafe { std::str::from_utf8_unchecked(&bytes[..valid_len]) }
        }
    }
}

// ── SummarizerEngine (In-Process llama-cpp-2 med Varm KV-Cache) ──────────────

pub struct SummarizerEngine {
    /// Viktigt: `ctx` ligger i `Option` och töms explicit i `Drop` INNAN `model` frigörs.
    ctx: Option<LlamaContext<'static>>,
    /// Heap-allokerad modell på stabil adress under hela `SummarizerEngine`s livstid.
    model: Box<LlamaModel>,
    pub model_id: String,
    pub model_label: String,
    current_n_ctx: u32,
    safe_ceiling: u32,
    /// De prompt-tokens som just nu ligger giltiga i KV-cachen `[0 .. cached_tokens.len())`.
    cached_tokens: Vec<LlamaToken>,
}

impl Drop for SummarizerEngine {
    fn drop(&mut self) {
        self.ctx = None;
    }
}

impl SummarizerEngine {
    pub fn load(spec: &ModelSpec, model_path: &Path) -> Result<Self, String> {
        if !model_path.exists() {
            return Err(format!(
                "Modellfilen hittades inte på disk: {}",
                model_path.display()
            ));
        }

        let file_bytes = std::fs::metadata(model_path)
            .map(|m| m.len())
            .unwrap_or(spec.expected_bytes);

        let backend = ensure_llama_backend()?;
        let mut model_params = LlamaModelParams::default().with_use_mmap(true);

        // Aktivera Apple Silicon Metal automatiskt på macOS; på Windows körs 100 % portabel SIMD/AVX2 CPU.
        #[cfg(target_os = "macos")]
        if backend.supports_gpu_offload() {
            model_params = model_params.with_n_gpu_layers(999);
        }

        let model = Box::new(
            LlamaModel::load_from_file(backend, model_path, &model_params)
                .map_err(|e| format!("Kunde inte ladda språkmodellen ({}): {e}", spec.label))?,
        );

        let total_ram_gb = detect_total_ram_gb();
        let ram_ceiling = safe_ctx_ceiling(total_ram_gb, file_bytes);
        let train_ctx = model.n_ctx_train().max(8192);
        let safe_ceiling = ram_ceiling.min(train_ctx).min(MAX_CTX);
        let initial_ctx = DEFAULT_MIN_CTX.min(safe_ceiling);

        let ctx = Self::create_context(&model, backend, initial_ctx)?;

        Ok(Self {
            ctx: Some(ctx),
            model,
            model_id: spec.id.to_string(),
            model_label: spec.label.to_string(),
            current_n_ctx: initial_ctx,
            safe_ceiling,
            cached_tokens: Vec::with_capacity(initial_ctx as usize),
        })
    }

    fn create_context(
        model: &Box<LlamaModel>,
        backend: &'static LlamaBackend,
        n_ctx: u32,
    ) -> Result<LlamaContext<'static>, String> {
        let (gen_threads, batch_threads) = optimal_cpu_threads();
        let ctx_params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(n_ctx))
            .with_n_batch(PREFILL_BATCH as u32)
            .with_n_ubatch(PREFILL_UBATCH)
            .with_swa_full(true)
            .with_no_perf(true)
            .with_n_threads(gen_threads)
            .with_n_threads_batch(batch_threads);

        // SAFETY: `model` ägs i en `Box<LlamaModel>` på stabil minnesadress inne i `SummarizerEngine`,
        // flyttas aldrig ur sin heap-allokering och `self.ctx` töms alltid explicit före `model` i `Drop`.
        let model_static_ref: &'static LlamaModel = unsafe { &*(&**model as *const LlamaModel) };
        model_static_ref
            .new_context(backend, ctx_params)
            .map_err(|e| format!("Kunde inte allokera kontextfönster ({n_ctx} tokens): {e}"))
    }

    fn ensure_context_capacity(&mut self, needed_ctx: u32) -> Result<(), String> {
        let target = round_ctx_up(needed_ctx).clamp(DEFAULT_MIN_CTX.min(self.safe_ceiling), self.safe_ceiling);
        if self.ctx.is_some() && target <= self.current_n_ctx {
            return Ok(());
        }
        // Släpp gamla kontexten först så minnet frigörs innan det större kontextfönstret allokeras
        self.ctx = None;
        self.cached_tokens.clear();
        let backend = ensure_llama_backend()?;
        let new_ctx = Self::create_context(&self.model, backend, target)?;
        self.ctx = Some(new_ctx);
        self.current_n_ctx = target;
        Ok(())
    }

    pub fn count_tokens(&self, text: &str) -> usize {
        self.model
            .vocab()
            .tokenize(text.as_bytes(), false, false)
            .len()
    }

    pub fn effective_temperature(req: &SummarizeRequest) -> (f32, f32, f32) {
        let base_temp = if req.creative {
            0.30
        } else {
            req.sampling
                .as_ref()
                .and_then(|s| s.temperature)
                .unwrap_or(0.05)
        };
        let repeat_pen = req
            .sampling
            .as_ref()
            .and_then(|s| s.repeat_penalty)
            .unwrap_or(1.10);
        let presence_pen = req
            .sampling
            .as_ref()
            .and_then(|s| s.presence_penalty)
            .unwrap_or(0.0);
        (base_temp.clamp(0.0, 1.5), repeat_pen.clamp(0.8, 1.8), presence_pen.clamp(0.0, 2.0))
    }

    pub fn run_summarize<FProg, FChunk>(
        &mut self,
        req: &SummarizeRequest,
        cancel_flag: &AtomicBool,
        mut on_progress: FProg,
        mut on_chunk: FChunk,
    ) -> Result<SummarizeResult, String>
    where
        FProg: FnMut(SummarizeProgressPayload),
        FChunk: FnMut(SummarizeChunkPayload),
    {
        let t0 = Instant::now();
        let (temperature, repeat_penalty, presence_penalty) = Self::effective_temperature(req);
        let margin = if req.thorough {
            RESPONSE_MARGIN_THOROUGH
        } else {
            RESPONSE_MARGIN_FAST
        };

        let full_prompt = build_kv_friendly_prompt(req, false);
        let prompt_tokens = self
            .model
            .vocab()
            .tokenize(full_prompt.as_bytes(), false, true);

        if prompt_tokens.is_empty() {
            return Err("Mötesunderlaget är tomt eller kunde inte tokeniseras.".to_string());
        }

        let needed_total = (prompt_tokens.len() as u32).saturating_add(margin);

        if needed_total <= self.safe_ceiling {
            self.ensure_context_capacity(needed_total)?;
            let stats = self.generate_from_tokens(
                &prompt_tokens,
                req.thorough,
                temperature,
                repeat_penalty,
                presence_penalty,
                None,
                cancel_flag,
                &mut on_progress,
                &mut on_chunk,
            )?;

            let elapsed_ms = t0.elapsed().as_millis() as u64;
            return Ok(SummarizeResult {
                markdown: stats.text,
                model_id: self.model_id.clone(),
                model_label: self.model_label.clone(),
                prompt_tokens: prompt_tokens.len(),
                kv_reused_tokens: stats.kv_reused_tokens,
                generated_tokens: stats.generated_tokens,
                thinking_tokens: stats.thinking_tokens,
                duration_ms: elapsed_ms,
                tokens_per_sec: stats.tokens_per_sec,
                strategy: "single".to_string(),
                used_temperature: temperature,
            });
        }

        // Map-Reduce-fallback för extremt långa möten som överskrider RAM-kontexttaket
        let chunk_ctx = self.safe_ceiling.min(32_768);
        self.ensure_context_capacity(chunk_ctx)?;
        let transcript_tok_len = self.count_tokens(&req.transcript).max(1);
        let chars_per_tok =
            (req.transcript.chars().count() as f64) / (transcript_tok_len as f64);
        let chunk_budget = ((chunk_ctx.saturating_sub(margin + 1500)) as usize).max(4096);
        let chunks = split_into_chunks(&req.transcript, chunk_budget, chars_per_tok, 180);
        let total_chunks = chunks.len();

        let mut map_outputs: Vec<String> = Vec::with_capacity(total_chunks);
        let mut total_gen_tokens = 0usize;
        let mut total_think_tokens = 0usize;

        for (idx, chunk_text) in chunks.into_iter().enumerate() {
            if cancel_flag.load(Ordering::Relaxed) {
                return Err(CANCELLED_ERR.to_string());
            }
            let mut chunk_req = req.clone();
            chunk_req.transcript = chunk_text;
            let map_prompt = build_kv_friendly_prompt(&chunk_req, true);
            let map_tokens = self
                .model
                .vocab()
                .tokenize(map_prompt.as_bytes(), false, true);

            let mut silent_chunk = |_c: SummarizeChunkPayload| {};
            let part_stats = self.generate_from_tokens(
                &map_tokens,
                req.thorough,
                temperature,
                repeat_penalty,
                presence_penalty,
                Some((idx + 1, total_chunks)),
                cancel_flag,
                &mut on_progress,
                &mut silent_chunk,
            )?;

            total_gen_tokens += part_stats.generated_tokens;
            total_think_tokens += part_stats.thinking_tokens;

            if !is_empty_chunk_response(&part_stats.text) {
                map_outputs.push(part_stats.text);
            }
        }

        if map_outputs.is_empty() {
            let empty_msg = "Inget relevant mötesinnehåll identifierades i underlaget.".to_string();
            on_chunk(SummarizeChunkPayload {
                full_text: empty_msg.clone(),
                generated_tokens: total_gen_tokens,
                tokens_per_sec: 0.0,
            });
            return Ok(SummarizeResult {
                markdown: empty_msg,
                model_id: self.model_id.clone(),
                model_label: self.model_label.clone(),
                prompt_tokens: prompt_tokens.len(),
                kv_reused_tokens: 0,
                generated_tokens: total_gen_tokens,
                thinking_tokens: total_think_tokens,
                duration_ms: t0.elapsed().as_millis() as u64,
                tokens_per_sec: 0.0,
                strategy: "chunked".to_string(),
                used_temperature: temperature,
            });
        }

        if map_outputs.len() == 1 && req.agenda.as_deref().unwrap_or("").trim().is_empty() {
            let only = map_outputs.remove(0);
            on_chunk(SummarizeChunkPayload {
                full_text: only.clone(),
                generated_tokens: total_gen_tokens,
                tokens_per_sec: 0.0,
            });
            return Ok(SummarizeResult {
                markdown: only,
                model_id: self.model_id.clone(),
                model_label: self.model_label.clone(),
                prompt_tokens: prompt_tokens.len(),
                kv_reused_tokens: 0,
                generated_tokens: total_gen_tokens,
                thinking_tokens: total_think_tokens,
                duration_ms: t0.elapsed().as_millis() as u64,
                tokens_per_sec: 0.0,
                strategy: "chunked".to_string(),
                used_temperature: temperature,
            });
        }

        let reduce_prompt = build_reduce_prompt(&map_outputs, req);
        let reduce_tokens = self
            .model
            .vocab()
            .tokenize(reduce_prompt.as_bytes(), false, true);
        let needed_reduce = (reduce_tokens.len() as u32).saturating_add(margin);
        self.ensure_context_capacity(needed_reduce.min(self.safe_ceiling))?;

        on_progress(SummarizeProgressPayload {
            phase: "reducing".to_string(),
            label: "Slår ihop alla delsammanfattningar till ett sammanhängande dokument…".to_string(),
            prefill_percent: 0,
            kv_reused_tokens: 0,
            prompt_tokens: reduce_tokens.len(),
            thinking_tokens: total_think_tokens,
            generated_tokens: total_gen_tokens,
            tokens_per_sec: 0.0,
            chunk_current: Some(total_chunks),
            chunk_total: Some(total_chunks),
        });

        let red_stats = self.generate_from_tokens(
            &reduce_tokens,
            req.thorough,
            temperature,
            repeat_penalty,
            presence_penalty,
            None,
            cancel_flag,
            &mut on_progress,
            &mut on_chunk,
        )?;

        total_gen_tokens += red_stats.generated_tokens;
        total_think_tokens += red_stats.thinking_tokens;

        Ok(SummarizeResult {
            markdown: red_stats.text,
            model_id: self.model_id.clone(),
            model_label: self.model_label.clone(),
            prompt_tokens: prompt_tokens.len(),
            kv_reused_tokens: 0,
            generated_tokens: total_gen_tokens,
            thinking_tokens: total_think_tokens,
            duration_ms: t0.elapsed().as_millis() as u64,
            tokens_per_sec: red_stats.tokens_per_sec,
            strategy: "chunked".to_string(),
            used_temperature: temperature,
        })
    }

    pub fn run_adjust<FProg, FChunk>(
        &mut self,
        current_markdown: &str,
        directive: &str,
        cancel_flag: &AtomicBool,
        mut on_progress: FProg,
        mut on_chunk: FChunk,
    ) -> Result<SummarizeResult, String>
    where
        FProg: FnMut(SummarizeProgressPayload),
        FChunk: FnMut(SummarizeChunkPayload),
    {
        let t0 = Instant::now();
        let prompt = build_adjust_prompt(current_markdown, directive);
        let prompt_tokens = self
            .model
            .vocab()
            .tokenize(prompt.as_bytes(), false, true);

        if prompt_tokens.is_empty() {
            return Err("Dokumentet som ska justeras är tomt.".to_string());
        }

        let needed = (prompt_tokens.len() as u32).saturating_add(RESPONSE_MARGIN_FAST);
        if needed > self.safe_ceiling {
            return Err("Dokumentet är för stort för att rymmas i kontextfönstret vid justering.".to_string());
        }
        self.ensure_context_capacity(needed)?;

        let stats = self.generate_from_tokens(
            &prompt_tokens,
            false,
            0.08,
            1.05,
            0.0,
            None,
            cancel_flag,
            &mut on_progress,
            &mut on_chunk,
        )?;

        Ok(SummarizeResult {
            markdown: stats.text,
            model_id: self.model_id.clone(),
            model_label: self.model_label.clone(),
            prompt_tokens: prompt_tokens.len(),
            kv_reused_tokens: stats.kv_reused_tokens,
            generated_tokens: stats.generated_tokens,
            thinking_tokens: 0,
            duration_ms: t0.elapsed().as_millis() as u64,
            tokens_per_sec: stats.tokens_per_sec,
            strategy: "single".to_string(),
            used_temperature: 0.08,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn generate_from_tokens<FProg, FChunk>(
        &mut self,
        prompt_tokens: &[LlamaToken],
        thorough: bool,
        temperature: f32,
        repeat_penalty: f32,
        presence_penalty: f32,
        chunk_info: Option<(usize, usize)>,
        cancel_flag: &AtomicBool,
        on_progress: &mut FProg,
        on_chunk: &mut FChunk,
    ) -> Result<GenerationStats, String>
    where
        FProg: FnMut(SummarizeProgressPayload),
        FChunk: FnMut(SummarizeChunkPayload),
    {
        let vocab = self.model.vocab();
        let n_vocab = self.model.n_vocab();

        // Hitta längsta gemensamma token-prefix i KV-cachen (behåll som mest len - 1 så att sista token ger färska logits)
        let common_prefix = self
            .cached_tokens
            .iter()
            .zip(prompt_tokens.iter())
            .take_while(|(a, b)| a == b)
            .count();
        let mut keep_len = common_prefix.min(prompt_tokens.len().saturating_sub(1));

        let ctx = self
            .ctx
            .as_mut()
            .ok_or_else(|| "Inferenskontext saknas.".to_string())?;

        if keep_len > 0 {
            let cleared_ok = matches!(
                ctx.clear_kv_cache_seq(Some(0), Some(keep_len as u32), None),
                Ok(true)
            );
            if !cleared_ok {
                ctx.clear_kv_cache();
                self.cached_tokens.clear();
                keep_len = 0;
            } else {
                self.cached_tokens.truncate(keep_len);
            }
        } else {
            ctx.clear_kv_cache();
            self.cached_tokens.clear();
        }

        let to_decode = &prompt_tokens[keep_len..];
        let total_to_decode = to_decode.len();
        let mut batch = LlamaBatch::new(PREFILL_BATCH, 1);
        let mut last_chunk_len = 0usize;

        for (step_idx, chunk_start) in (0..total_to_decode).step_by(PREFILL_BATCH).enumerate() {
            if cancel_flag.load(Ordering::Relaxed) {
                return Err(CANCELLED_ERR.to_string());
            }

            let chunk_end = (chunk_start + PREFILL_BATCH).min(total_to_decode);
            last_chunk_len = chunk_end - chunk_start;

            let pct = (((keep_len + chunk_start) as f64 / (prompt_tokens.len() as f64)) * 100.0)
                .round()
                .clamp(0.0, 99.0) as u8;
            let label = if let Some((c, tot)) = chunk_info {
                format!("Läser in mötesdel {c} av {tot} ({pct} %)…")
            } else if keep_len > 500 && step_idx == 0 {
                format!("Återanvänder mötet ur varm cache ({keep_len} tokens)…")
            } else {
                format!("Läser in mötestexten ({pct} %)…")
            };

            on_progress(SummarizeProgressPayload {
                phase: "prefill".to_string(),
                label,
                prefill_percent: pct,
                kv_reused_tokens: keep_len,
                prompt_tokens: prompt_tokens.len(),
                thinking_tokens: 0,
                generated_tokens: 0,
                tokens_per_sec: 0.0,
                chunk_current: chunk_info.map(|x| x.0),
                chunk_total: chunk_info.map(|x| x.1),
            });

            batch.clear();
            for (offset, &tok) in to_decode[chunk_start..chunk_end].iter().enumerate() {
                let rel = chunk_start + offset;
                let pos = (keep_len + rel) as i32;
                let want_logit = rel + 1 == total_to_decode;
                batch
                    .add(tok, pos, &[0], want_logit)
                    .map_err(|e| format!("Batch-fel vid promptinläsning: {e}"))?;
            }

            ctx.decode(&mut batch)
                .map_err(|e| format!("Kunde inte läsa in mötesunderlaget i modellen: {e}"))?;
            self.cached_tokens
                .extend_from_slice(&to_decode[chunk_start..chunk_end]);
        }

        // Konfigurera sampler: ren greedy eller lätt kedja med temperatur + repetitionsstraff
        let mut sampler = build_sampler(n_vocab, temperature, repeat_penalty, presence_penalty);

        // Identifiera Gemma 4 `<channel|>`-slutmarkör för tankekanalen
        let channel_end_seq = vocab.tokenize(b"<channel|>", false, true);
        let channel_end_tok = channel_end_seq.first().copied();
        let forced_close_seq = vocab.tokenize(b"\n<channel|>", false, true);

        let mut in_thought = thorough;
        let mut thinking_tokens = 0usize;
        let mut answer_tokens: Vec<LlamaToken> = Vec::with_capacity(1024);
        let mut step_batch = LlamaBatch::new(16, 1);
        let mut cur_pos = prompt_tokens.len() as i32;
        let mut logits_idx = (last_chunk_len as i32) - 1;

        let gen_start = Instant::now();
        let mut last_ui_emit = Instant::now()
            .checked_sub(Duration::from_millis(50))
            .unwrap_or_else(Instant::now);

        let max_total_steps = if thorough {
            MAX_THOUGHT_TOKENS + MAX_ANSWER_TOKENS
        } else {
            MAX_ANSWER_TOKENS
        };

        for _ in 0..max_total_steps {
            if cancel_flag.load(Ordering::Relaxed) {
                return Err(CANCELLED_ERR.to_string());
            }

            // Om tankebudgeten slår i taket: tvinga in `\n<channel|>` direkt i KV-cachen utan att kasta tankearbetet!
            if in_thought && thinking_tokens >= MAX_THOUGHT_TOKENS && !forced_close_seq.is_empty() {
                step_batch.clear();
                for (i, &tok) in forced_close_seq.iter().enumerate() {
                    let want = i + 1 == forced_close_seq.len();
                    step_batch
                        .add(tok, cur_pos, &[0], want)
                        .map_err(|e| format!("Batch-fel vid tankestängning: {e}"))?;
                    cur_pos += 1;
                }
                ctx.decode(&mut step_batch)
                    .map_err(|e| format!("Inferensfel vid tankestängning: {e}"))?;
                logits_idx = (forced_close_seq.len() as i32) - 1;
                in_thought = false;
            }

            let next_token = sampler.sample(ctx, logits_idx);
            if vocab.is_eog(next_token) {
                break;
            }

            if in_thought {
                if Some(next_token) == channel_end_tok {
                    in_thought = false;
                } else {
                    thinking_tokens += 1;
                    if last_ui_emit.elapsed() >= Duration::from_millis(80) {
                        last_ui_emit = Instant::now();
                        let secs = gen_start.elapsed().as_secs_f32().max(0.001);
                        let tps = (thinking_tokens as f32) / secs;
                        let lbl = if let Some((c, tot)) = chunk_info {
                            format!("Resonerar kring mötesdel {c}/{tot} ({thinking_tokens} tanketokens)…")
                        } else {
                            format!("Tänker igenom struktur och detaljer ({thinking_tokens} tanketokens)…")
                        };
                        on_progress(SummarizeProgressPayload {
                            phase: "thinking".to_string(),
                            label: lbl,
                            prefill_percent: 100,
                            kv_reused_tokens: keep_len,
                            prompt_tokens: prompt_tokens.len(),
                            thinking_tokens,
                            generated_tokens: 0,
                            tokens_per_sec: tps,
                            chunk_current: chunk_info.map(|x| x.0),
                            chunk_total: chunk_info.map(|x| x.1),
                        });
                    }
                }
            } else {
                // Stoppa om modellen skulle nå `<turn|>` utan att is_eog triggas
                answer_tokens.push(next_token);

                if answer_tokens.len() >= MAX_ANSWER_TOKENS {
                    break;
                }

                if last_ui_emit.elapsed() >= Duration::from_millis(28) {
                    last_ui_emit = Instant::now();
                    let secs = gen_start.elapsed().as_secs_f32().max(0.001);
                    let total_out = thinking_tokens + answer_tokens.len();
                    let tps = (total_out as f32) / secs;

                    let raw_bytes = vocab.detokenize(&answer_tokens, false, false);
                    let clean_str = sanitize_model_output(valid_utf8_prefix(&raw_bytes));

                    on_chunk(SummarizeChunkPayload {
                        full_text: clean_str,
                        generated_tokens: answer_tokens.len(),
                        tokens_per_sec: tps,
                    });

                    let lbl = if let Some((c, tot)) = chunk_info {
                        format!("Sammanfattar del {c} av {tot}…")
                    } else {
                        "Skriver mötesanteckning…".to_string()
                    };
                    on_progress(SummarizeProgressPayload {
                        phase: "generating".to_string(),
                        label: lbl,
                        prefill_percent: 100,
                        kv_reused_tokens: keep_len,
                        prompt_tokens: prompt_tokens.len(),
                        thinking_tokens,
                        generated_tokens: answer_tokens.len(),
                        tokens_per_sec: tps,
                        chunk_current: chunk_info.map(|x| x.0),
                        chunk_total: chunk_info.map(|x| x.1),
                    });
                }
            }

            // Mata in det valda tokenet i KV-cachen för nästa steg
            if cur_pos + 1 >= self.current_n_ctx as i32 {
                break;
            }
            step_batch.clear();
            step_batch
                .add(next_token, cur_pos, &[0], true)
                .map_err(|e| format!("Batch-fel under textgenerering: {e}"))?;
            cur_pos += 1;
            ctx.decode(&mut step_batch)
                .map_err(|e| format!("Inferensfel under textgenerering: {e}"))?;
            logits_idx = 0;
        }

        let secs = gen_start.elapsed().as_secs_f32().max(0.001);
        let tps = ((thinking_tokens + answer_tokens.len()) as f32) / secs;
        let final_bytes = vocab.detokenize(&answer_tokens, false, false);
        let final_text = sanitize_model_output(&String::from_utf8_lossy(&final_bytes))
            .trim()
            .to_string();

        on_chunk(SummarizeChunkPayload {
            full_text: final_text.clone(),
            generated_tokens: answer_tokens.len(),
            tokens_per_sec: tps,
        });

        Ok(GenerationStats {
            text: final_text,
            kv_reused_tokens: keep_len,
            generated_tokens: answer_tokens.len(),
            thinking_tokens,
            tokens_per_sec: tps,
        })
    }
}

struct GenerationStats {
    text: String,
    kv_reused_tokens: usize,
    generated_tokens: usize,
    thinking_tokens: usize,
    tokens_per_sec: f32,
}

fn build_sampler(
    n_vocab: i32,
    temperature: f32,
    repeat_penalty: f32,
    presence_penalty: f32,
) -> LlamaSampler {
    let has_penalties = (repeat_penalty - 1.0).abs() > 0.005 || presence_penalty > 0.005;

    if temperature <= 0.01 && !has_penalties {
        return LlamaSampler::greedy();
    }

    let mut chain: Vec<LlamaSampler> = Vec::with_capacity(5);
    if has_penalties {
        chain.push(LlamaSampler::penalties(
            n_vocab,
            128,
            repeat_penalty,
            0.0,
            presence_penalty,
        ));
    }

    if temperature <= 0.01 {
        chain.push(LlamaSampler::greedy());
    } else {
        chain.push(LlamaSampler::top_k(64));
        chain.push(LlamaSampler::top_p(0.95, 1));
        chain.push(LlamaSampler::temp(temperature));
        chain.push(LlamaSampler::dist(42));
    }

    LlamaSampler::chain_simple(chain)
}

/// Rensar eventuella kvarhängande kontrollmarkörer (`<turn|>`, `<|turn>`, ```markdown-omslag runt hela svaret).
fn sanitize_model_output(raw: &str) -> String {
    let mut s = raw
        .replace("<turn|>", "")
        .replace("<|turn>", "")
        .replace("<channel|>", "");

    let trimmed = s.trim_start();
    if let Some(after_fence) = trimmed.strip_prefix("```markdown\n") {
        if let Some(clean_body) = after_fence.trim_end().strip_suffix("```") {
            s = clean_body.to_string();
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_req(template_id: &str, template_label: &str, prompt: &str) -> SummarizeRequest {
        SummarizeRequest {
            transcript: "Anna: Vi beslutar att införa nytt intranät den 1 november.".to_string(),
            template_id: template_id.to_string(),
            template_label: template_label.to_string(),
            template_prompt: prompt.to_string(),
            is_builtin_template: true,
            context: Some("Kommunal IT-avdelning".to_string()),
            agenda: None,
            creative: false,
            thorough: false,
            model_id: Some("e2b".to_string()),
            sampling: None,
        }
    }

    #[test]
    fn kv_friendly_prompt_shares_exact_transcript_prefix_across_templates() {
        let p_notes = build_kv_friendly_prompt(
            &sample_req(
                "meeting-notes",
                "Minnesanteckning",
                &format!("{COMMON_PREAMBLE}\n\nSkapa en kronologisk minnesanteckning."),
            ),
            false,
        );
        let p_actions = build_kv_friendly_prompt(
            &sample_req(
                "action-items",
                "Åtgärdspunkter",
                &format!("{COMMON_PREAMBLE}\n\nExtrahera enbart konkreta åtgärder."),
            ),
            false,
        );

        let marker = "--- SLUT PÅ MÖTESTRANSKRIBERING ---";
        let end1 = p_notes.find(marker).expect("marker i notes") + marker.len();
        let end2 = p_actions.find(marker).expect("marker i actions") + marker.len();

        assert_eq!(
            &p_notes[..end1],
            &p_actions[..end2],
            "Systemprompt, möteskontext och hela transkriberingen måste vara identiska i prefixet för varm KV-cache!"
        );
        assert!(p_notes.ends_with("<|turn>model\n<|channel>thought\n<channel|>"));
    }

    #[test]
    fn safe_ctx_ceiling_scales_with_ram_and_respects_bounds() {
        assert_eq!(safe_ctx_ceiling(8.0, MODEL_E2B.expected_bytes), 113_664);
        assert_eq!(safe_ctx_ceiling(16.0, MODEL_E4B.expected_bytes), MAX_CTX);
        assert_eq!(safe_ctx_ceiling(4.0, MODEL_E4B.expected_bytes), 8192);
    }

    #[test]
    fn splits_long_text_cleanly_at_sentence_boundaries() {
        let sentence = "Detta är en mening om projektets tidsplan och leveranser. ";
        let long_text = sentence.repeat(80);
        let chunks = split_into_chunks(&long_text, 300, 3.5, 30);
        assert!(chunks.len() >= 4);
        assert!(chunks[0].ends_with('.'));
    }
}
