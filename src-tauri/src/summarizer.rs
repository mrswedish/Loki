use std::num::NonZeroU32;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use llama_cpp_2::context::params::{KvCacheType, LlamaContextParams};
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
/// Minsta standardfönster på datorer med mindre RAM.
const DEFAULT_MIN_CTX: u32 = 16_384;
/// Startfönster på GPU-datorer med gott om minne (>= 22 GB RAM) så 25k–28k-möten slipper omallokering.
const HIGH_RAM_INIT_CTX: u32 = 36_864;
/// Tak för enkelpass på ren CPU utan GPU-offload (över ~8k–9k transkripttokens ger diskbaserade bevispaket både högre kvalitet och slipper kvadratisk CPU-sirap).
const CPU_SINGLE_PASS_MAX_CTX: u32 = 12_288;
/// Kompakt kontextfönster för svag hårdvara och Pass 1-bevisextrahering (~160 MB KV-cache i RAM).
const WEAK_HW_EVIDENCE_CTX: u32 = 8_192;
/// Avsnittsstorlek (~12–15 min tal) och överlapp vid strukturerad bevisextrahering till disk.
const EVIDENCE_CHUNK_TOKENS: usize = 3_400;
const EVIDENCE_OVERLAP_TOKENS: usize = 200;
/// Avsnittsstorlek vid sekventiell korrekturläsning ("Rätta transkriberingen") på långa filer.
const CLEANUP_CHUNK_TOKENS: usize = 2_200;
/// Gräns i tokens för samlade bevispaket där Pass 2 byter från direktsyntes till tvåstegskodning (temamatris -> sektionssyntes från disk).
const DIRECT_SYNTHESIS_MAX_EVIDENCE_TOKENS: usize = 4_800;
/// Batchstorlek vid prompt-inläsning på GPU:
/// - macOS Metal (Unified Memory): 2048/2048 för maximal SIMD-matrisgenomströmning.
/// - Windows Vulkan (WDDM): 512/512 ger hög beräkningsmättnad utan att någonsin passera Windows WDDM:s
///   2,0-sekunders TDR-gräns (`vk::DeviceLostError`) på iGPU/mellanklasskort, och ger 4x snabbare avbrott vid modellbyte/avslut!
#[cfg(target_os = "macos")]
const GPU_PREFILL_BATCH: usize = 2048;
#[cfg(target_os = "macos")]
const GPU_PREFILL_UBATCH: u32 = 2048;
#[cfg(not(target_os = "macos"))]
const GPU_PREFILL_BATCH: usize = 512;
#[cfg(not(target_os = "macos"))]
const GPU_PREFILL_UBATCH: u32 = 512;
/// Batchstorlek på ren CPU (responsivt avbrott och lägre cache-tryck).
const CPU_PREFILL_BATCH: usize = 512;
const CPU_PREFILL_UBATCH: u32 = 512;
/// Stegstorlek vid tyst bakgrundsförvärmning på GPU (lämnar över låset snabbt vid klick på "Skapa" eller modellbyte).
#[cfg(target_os = "macos")]
const PREWARM_BATCH: usize = 1024;
#[cfg(not(target_os = "macos"))]
const PREWARM_BATCH: usize = 512;
/// Reserverat utrymme för modellens svar i normalt respektive "Noggrannare"-läge.
const RESPONSE_MARGIN_FAST: u32 = 4096;
const RESPONSE_MARGIN_THOROUGH: u32 = 8192;
/// Max antal tanketokens i "Noggrannare"-läget innan `</thought>` (`\n<channel|>`) injiceras i KV-cachen.
const MAX_THOUGHT_TOKENS: usize = 1536;
/// Max genererade svarstokens per pass.
const MAX_ANSWER_TOKENS: usize = 4096;
/// KV-cache-kostnad i GB per token med Q8_0 + säkerhetsmarginal (~0,02 MB/token).
const KV_GB_PER_TOKEN: f64 = 0.00002;

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
    pub recommended_min_ram_gb: u32,
}

pub const MODEL_E2B: ModelSpec = ModelSpec {
    id: "e2b",
    label: "Gemma 4 E2B QAT (2,62 GB)",
    short_label: "E2B · Snabb",
    description: "Rekommenderad standard. Snabb och resurssnål med QAT-precision – utmärkt på alla bärbara datorer.",
    filename: "gemma-4-E2B-it-qat-UD-Q4_K_XL.gguf",
    hf_url: "https://huggingface.co/unsloth/gemma-4-E2B-it-qat-GGUF/resolve/main/gemma-4-E2B-it-qat-UD-Q4_K_XL.gguf",
    expected_bytes: 2_620_370_976,
    min_valid_bytes: 2_400_000_000,
    recommended_min_ram_gb: 8,
};

pub const MODEL_E4B: ModelSpec = ModelSpec {
    id: "e4b",
    label: "Gemma 4 E4B QAT (4,22 GB)",
    short_label: "E4B · Fördjupad",
    description: "Mellanstor språkmodell för mer nyanserade protokoll och fackterminologi. Passar datorer med minst 16 GB RAM.",
    filename: "gemma-4-E4B-it-qat-UD-Q4_K_XL.gguf",
    hf_url: "https://huggingface.co/unsloth/gemma-4-E4B-it-qat-GGUF/resolve/main/gemma-4-E4B-it-qat-UD-Q4_K_XL.gguf",
    expected_bytes: 4_215_695_776,
    min_valid_bytes: 3_900_000_000,
    recommended_min_ram_gb: 15,
};

pub const MODEL_12B: ModelSpec = ModelSpec {
    id: "12b",
    label: "Gemma 4 12B QAT (6,72 GB)",
    short_label: "12B · Maximal",
    description: "Maximal språklig skärpa och fackordstolkning för långa, komplexa möten. Rekommenderas för datorer med mer än 16 GB RAM.",
    filename: "gemma-4-12B-it-qat-UD-Q4_K_XL.gguf",
    hf_url: "https://huggingface.co/unsloth/gemma-4-12B-it-qat-GGUF/resolve/main/gemma-4-12B-it-qat-UD-Q4_K_XL.gguf",
    expected_bytes: 6_716_356_800,
    min_valid_bytes: 6_200_000_000,
    recommended_min_ram_gb: 20,
};

pub const ALL_MODELS: &[ModelSpec] = &[MODEL_E2B, MODEL_E4B, MODEL_12B];

pub fn get_model_spec(model_id: &str) -> Option<&'static ModelSpec> {
    ALL_MODELS.iter().find(|m| m.id == model_id)
}

// ── Domänprompter (bevarade och optimerade från Loki 2.9) ────────────────────

/// Gemensam, uppgiftsneutral basprompt i `<|turn>system` som gäller för ALLA körningar
/// (både sammanfattningsmallar och "Rätta transkriberingen"). Gör att hela transkriptet
/// i KV-cachen kan förvärmas direkt vid filimport och återanvändas till 99,9 % vid byte
/// av mall, sammanhang, kreativt läge och agenda!
pub const SHARED_SYSTEM_PROMPT: &str = "\
Du är expert på administrativ struktur och svensk textförädling av råa mötestranskriberingar.\n\n\
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
Svara på samma språk som transkriberingen (oftast svenska). Skriv inga inledande eller \
avslutande kommentarer om uppgiften – leverera bara det begärda dokumentet i Markdown.";

/// Sammanfattningsdisciplin som läggs i uppgiftsblocket för alla dokumentmallar (ej ren korrekturläsning).
pub const SUMMARY_DISCIPLINE: &str = "\
FILTRERA BORT KALLPRAT (STENHÅRT): Ta bort allt socialt småprat och tekniskt strul som inte \
hör till sakfrågorna – inledande prat om kameror, mikrofoner, ljud/bild, hälsningar, väder, \
helger, hälsa och privata anekdoter. Börja dokumentet först när det faktiska arbetet \
eller agendan drar igång.\n\n\
KRONOLOGI: Följ mötet i den ordning det skedde, ämne för ämne.\n\n\
TROHET: Använd talarnas egna ord och termer. Byt INTE ut ord mot synonymer i onödan och \
använd inga metaforer eller tolkande omskrivningar som kan förvränga vad som sades. \
Sammanfatta vad diskussionen landade i – skriv inte av dialogen rakt av.";

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
    #[serde(default)]
    pub deep_evidence: Option<bool>,
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

pub fn accelerator_label() -> &'static str {
    if cfg!(target_os = "macos") {
        "Metal (Apple Silicon)"
    } else if cfg!(feature = "vulkan") {
        "Vulkan (GPU)"
    } else {
        "CPU (SIMD/AVX2)"
    }
}

pub fn ensure_llama_backend() -> Result<&'static LlamaBackend, String> {
    let res = LLAMA_BACKEND.get_or_init(|| {
        // Tillåt ggml-vulkan att automatiskt spill-allokera i delat systemminne (HostVisible) när
        // dedikerat grafikkortsminne (VRAM) inte räcker till vid byte till större modell (E4B/12B).
        // Måste sättas INNAN Vulkan-enheten skapas i LlamaBackend::init()!
        #[cfg(feature = "vulkan")]
        if std::env::var_os("GGML_VK_ALLOW_SYSMEM_FALLBACK").is_none() {
            std::env::set_var("GGML_VK_ALLOW_SYSMEM_FALLBACK", "1");
        }

        crate::logger::log_info(&format!(
            "Initierar llama.cpp-backend ({})…",
            accelerator_label()
        ));
        let mut backend = LlamaBackend::init()
            .map_err(|e| format!("Kunde inte initiera lokal LLM-motor (llama.cpp): {e}"))?;
        backend.void_logs();
        crate::logger::log_info(&format!(
            "llama.cpp-backend initierad (supports_gpu_offload={})",
            backend.supports_gpu_offload()
        ));
        Ok(backend)
    });
    match res {
        Ok(b) => Ok(b),
        Err(e) => {
            crate::logger::log_error(e);
            Err(e.clone())
        }
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
        (logical / 2).clamp(4, 8)
    };
    let batch_threads = if logical <= 4 {
        logical.max(1)
    } else {
        ((logical * 3) / 4).clamp(4, 16)
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

// ── Prompt-uppbyggnad optimerad för Varm KV-cache & Bakgrundsförvärmning ─────

/// Bygger det helt invarianta prefixet (systemprompt + hela mötestranskriberingen) som delas av
/// samtliga mallar, korrektursteget, sammanhang, kreativt läge och agenda.
/// Kan förvärmas tyst i KV-cachen redan när användaren öppnar/klistrar in en mötesfil!
pub fn build_transcript_prefix(transcript: &str, thorough: bool) -> String {
    let think_prefix = if thorough { "<|think|>" } else { "" };
    format!(
        "<bos><|turn>system\n{think_prefix}{SHARED_SYSTEM_PROMPT}<turn|>\n<|turn>user\n--- MÖTESTRANSKRIBERING ---\n{transcript}\n--- SLUT PÅ MÖTESTRANSKRIBERING ---",
        transcript = transcript.trim(),
    )
}

/// Delar upp en malls prompt så att `SHARED_SYSTEM_PROMPT` + hela transkriptet hamnar i ett
/// 100 % invariant prefix `[0 .. P2)`, medan sammanhang, kreativt läge, malluppgift och agenda
/// placeras efter `--- SLUT PÅ MÖTESTRANSKRIBERING ---` i `[P2 ..)`.
/// När användaren byter mall, sammanhang, kreativt läge eller agenda på samma möte återanvänds
/// 99,9 % av kontexten direkt ur KV-cachen på ~0,01 s!
pub fn build_kv_friendly_prompt(req: &SummarizeRequest, map_mode_only: bool) -> String {
    let is_cleanup = req.template_id == "transcription-cleanup";
    let prefix = build_transcript_prefix(&req.transcript, req.thorough);

    let raw_tpl = req.template_prompt.trim();
    let stripped_task = raw_tpl
        .strip_prefix(COMMON_PREAMBLE)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(raw_tpl);

    let mut post_transcript = String::new();

    if let Some(ctx_str) = req.context.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        let ctx_hdr = if req.creative {
            CONTEXT_INSTRUCTION_CREATIVE
        } else {
            CONTEXT_INSTRUCTION
        };
        post_transcript.push_str("\n\n--- MÖTETS SAMMANHANG ---\n");
        post_transcript.push_str(ctx_hdr);
        post_transcript.push('\n');
        post_transcript.push_str(ctx_str);
        post_transcript.push_str("\n--- SLUT SAMMANHANG ---");
    }

    let mut task_block = if is_cleanup {
        format!(
            "{CLEANUP_PROMPT}\n\nKorrigera transkriberingen ovan enligt korrekturreglerna utan att sammanfatta eller utelämna något."
        )
    } else {
        format!("{SUMMARY_DISCIPLINE}\n\n{stripped_task}")
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

    format!(
        "{prefix}{post_transcript}\n\n--- UPPGIFT ({label}) ---\n{task_block}<turn|>\n<|turn>model\n",
        label = req.template_label.trim(),
    )
}

pub fn build_adjust_prompt(current_markdown: &str, directive: &str) -> String {
    format!(
        "<bos><|turn>system\n{ADJUST_PROMPT}<turn|>\n<|turn>user\n--- NUVARANDE MÖTESDOKUMENT ---\n{}\n--- SLUT PÅ DOKUMENT ---\n\n--- JUSTERINGSDIREKTIV FRÅN ANVÄNDAREN ---\n{}\n--- SLUT PÅ DIREKTIV ---<turn|>\n<|turn>model\n",
        current_markdown.trim(),
        directive.trim()
    )
}

fn format_template_task_tail(req: &SummarizeRequest) -> String {
    let raw_tpl = req.template_prompt.trim();
    let stripped_task = raw_tpl
        .strip_prefix(COMMON_PREAMBLE)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(raw_tpl);

    let mut out = String::new();
    if let Some(ctx_str) = req.context.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        let ctx_hdr = if req.creative {
            CONTEXT_INSTRUCTION_CREATIVE
        } else {
            CONTEXT_INSTRUCTION
        };
        out.push_str("--- MÖTETS SAMMANHANG ---\n");
        out.push_str(ctx_hdr);
        out.push('\n');
        out.push_str(ctx_str);
        out.push_str("\n--- SLUT SAMMANHANG ---\n\n");
    }

    out.push_str(&format!(
        "MÅLFORMAT ({}):\n{}",
        req.template_label.trim(),
        stripped_task
    ));

    if let Some(ag_str) = req.agenda.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        out.push_str(
            "\n\nSTRUKTUR ENLIGT AGENDA: Använd agendans punkter som rubriker i den ordning de står. \
             Punkter utanför agendan samlas under \"Övrigt\". Ej berörda punkter markeras \"Ej behandlad\".\n\n\
             --- AGENDA ---\n",
        );
        out.push_str(ag_str);
        out.push_str("\n--- SLUT AGENDA ---");
    }
    out
}

pub fn build_evidence_synthesis_prompt(
    formatted_packets: &str,
    consolidated_register: &str,
    req: &SummarizeRequest,
) -> String {
    let think_prefix = if req.thorough { "<|think|>" } else { "" };
    let task_tail = format_template_task_tail(req);
    let sys = crate::evidence::EVIDENCE_SYNTHESIS_SYSTEM_PROMPT;
    format!(
        "<bos><|turn>system\n{think_prefix}{sys}<turn|>\n<|turn>user\n\
         --- STRUKTURERADE BEVISPAKET FRÅN MÖTET ---\n{formatted_packets}--- SLUT PÅ BEVISPAKET ---\n\
         {consolidated_register}\n{task_tail}<turn|>\n<|turn>model\n"
    )
}

pub fn build_section_synthesis_prompt(
    clusters: &[crate::evidence::ThemeCluster],
    all_packets: &[crate::evidence::EvidencePacket],
    completed_titles: &[String],
    req: &SummarizeRequest,
) -> String {
    let think_prefix = if req.thorough { "<|think|>" } else { "" };
    let sys = crate::evidence::SECTION_SYNTHESIS_SYSTEM_PROMPT;

    let mut target_headings = String::new();
    let mut relevant_indices: Vec<usize> = Vec::new();
    for c in clusters {
        target_headings.push_str(&format!(
            "- ### {} (baseras på avsnitt {})\n",
            c.title,
            c.chunk_indices
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ));
        relevant_indices.extend(c.chunk_indices.iter().copied());
    }
    relevant_indices.sort_unstable();
    relevant_indices.dedup();

    let mut evidence_body = String::new();
    for idx in relevant_indices {
        if let Some(pkt) = all_packets.get(idx.saturating_sub(1)) {
            let formatted = pkt.format_for_synthesis();
            if !formatted.is_empty() {
                evidence_body.push_str(&formatted);
                evidence_body.push('\n');
            }
        }
    }

    let mut prev_note = String::new();
    if !completed_titles.is_empty() {
        prev_note.push_str("REDAN SKRIVNA TEMARUBRIKER TIDIGARE I DOKUMENTET (upprepa inte dessa):\n- ");
        prev_note.push_str(&completed_titles.join("\n- "));
        prev_note.push_str("\n\n");
    }

    let raw_tpl = req.template_prompt.trim();
    let stripped_task = raw_tpl
        .strip_prefix(COMMON_PREAMBLE)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(raw_tpl);

    let mut ctx_block = String::new();
    if let Some(ctx_str) = req.context.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        ctx_block.push_str(&format!("MÖTETS SAMMANHANG: {ctx_str}\n\n"));
    }

    format!(
        "<bos><|turn>system\n{think_prefix}{sys}<turn|>\n<|turn>user\n\
         {ctx_block}{prev_note}\
         ÖVERGRIPANDE STIL & MALL ({label}):\n{stripped_task}\n\n\
         TEMASEKTIONER ATT SKRIVA NU:\n{target_headings}\n\
         --- BEVISPAKET FÖR DESSA TEMAN ---\n{evidence_body}--- SLUT BEVISPAKET ---\n\n\
         Skriv nu enbart dessa temasektioner (`### Rubrik`) i Markdown med full skärpa, inklusive eventuella oenigheter, risker och nyckelcitat.<turn|>\n<|turn>model\n",
        label = req.template_label.trim()
    )
}

pub fn build_decisions_appendix_prompt(consolidated_register: &str, thorough: bool) -> String {
    let think_prefix = if thorough { "<|think|>" } else { "" };
    format!(
        "<bos><|turn>system\n{think_prefix}Du sammanställer avslutande besluts- och åtgärdsavsnitt för ett långt mötesdokument. \
         Slå ihop dubbletter från överlappande avsnitt, bevara ansvariga och datum exakt, och lägg aldrig till fakta.<turn|>\n\
         <|turn>user\n{consolidated_register}\n\
         Skriv nu enbart avslutande Markdown-avsnitt med `## Samlade beslut` (om beslut finns) och `## Samlade åtgärder` \
         (format `- [ ] Åtgärd — Ansvarig` om åtgärder finns). Inga inledande kommentarer.<turn|>\n<|turn>model\n"
    )
}

pub fn build_cleanup_chunk_prompt(
    chunk_text: &str,
    chunk_index: usize,
    total_chunks: usize,
    rolling_state: &crate::evidence::RollingMeetingState,
    context: Option<&str>,
) -> String {
    let mut extra = String::new();
    if let Some(ctx_str) = context.map(str::trim).filter(|s| !s.is_empty()) {
        extra.push_str(&format!(
            "MÖTETS SAMMANHANG (för rätt stavning av facktermer och namn): {ctx_str}\n\n"
        ));
    }
    let rolling = rolling_state.format_header();
    if !rolling.is_empty() {
        extra.push_str(&rolling);
    }
    format!(
        "<bos><|turn>system\n{SHARED_SYSTEM_PROMPT}\n\n{CLEANUP_PROMPT}<turn|>\n<|turn>user\n\
         {extra}--- DEL {chunk_index} AV {total_chunks} ATT KORREKTURLÄSA ---\n{}\n--- SLUT DEL {chunk_index} ---\n\n\
         Leverera ENBART den språkligt korrigerade texten för del {chunk_index} utan att sammanfatta eller utelämna något.<turn|>\n<|turn>model\n",
        chunk_text.trim()
    )
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
    pub gpu_active: bool,
    pub kv_q8_active: bool,
    current_n_ctx: u32,
    safe_ceiling: u32,
    /// De prompt-tokens som just nu ligger giltiga i KV-cachen `[0 .. cached_tokens.len())`.
    cached_tokens: Vec<LlamaToken>,
}

impl Drop for SummarizerEngine {
    fn drop(&mut self) {
        let t_drop = Instant::now();
        self.ctx = None;
        crate::logger::log_info(&format!(
            "Inferenskontext frigjord för modell '{}' ({} ms); frigör modellvikter…",
            self.model_id,
            t_drop.elapsed().as_millis()
        ));
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

        let t_load = Instant::now();
        crate::logger::log_info(&format!(
            "Laddar modell {} från {} ({:.2} GB)",
            spec.id,
            model_path.display(),
            (file_bytes as f64) / 1_000_000_000.0
        ));

        let backend = ensure_llama_backend()?;
        let total_ram_gb = detect_total_ram_gb();
        let ram_ceiling = safe_ctx_ceiling(total_ram_gb, file_bytes);

        let want_gpu = cfg!(any(target_os = "macos", feature = "vulkan"))
            && backend.supports_gpu_offload();

        let try_load_and_ctx =
            |use_gpu: bool| -> Result<(Box<LlamaModel>, LlamaContext<'static>, u32, u32, bool, bool), String> {
                // På Windows Vulkan med GPU-offload läser vi in vikterna direkt i Vulkan-buffertar utan
                // `MapViewOfFile` (`use_mmap=false`). Det undviker `VK_EXT_external_memory_host`-buggar
                // på Intel/AMD-drivrutiner och släpper filhandtag/minne 100 % rent direkt vid modellbyte!
                let use_mmap = !(use_gpu && cfg!(all(target_os = "windows", feature = "vulkan")));
                let mut mparams = LlamaModelParams::default().with_use_mmap(use_mmap);
                if use_gpu {
                    crate::logger::log_info(&format!(
                        "Aktiverar GPU-offload (n_gpu_layers=999, use_mmap={use_mmap})"
                    ));
                    mparams = mparams.with_n_gpu_layers(999);
                } else {
                    mparams = mparams.with_n_gpu_layers(0);
                }

                let m = Box::new(
                    LlamaModel::load_from_file(backend, model_path, &mparams)
                        .map_err(|e| format!("Kunde inte ladda språkmodellen ({}): {e}", spec.label))?,
                );

                let train_ctx = m.n_ctx_train().max(8192);
                let ceiling = ram_ceiling.min(train_ctx).min(MAX_CTX);
                // Förallokera endast det stora 36k-fönstret på macOS (Unified Memory där system-RAM = GPU-minne).
                // På Windows/Vulkan kan PC:n ha 32 GB system-RAM men 4–8 GB VRAM — starta med 16k så modellbyte till
                // E4B/12B alltid ryms smidigt, och väx dynamiskt via `ensure_context_capacity` vid långa möten!
                let base_init = if cfg!(target_os = "macos") && use_gpu && total_ram_gb >= 22.0 {
                    HIGH_RAM_INIT_CTX
                } else if !use_gpu || total_ram_gb <= 10.5 {
                    WEAK_HW_EVIDENCE_CTX
                } else {
                    DEFAULT_MIN_CTX
                };
                let init_ctx = base_init.min(ceiling);
                let (c, kv_q8) = Self::create_context(&m, backend, init_ctx, use_gpu)?;
                Ok((m, c, init_ctx, ceiling, use_gpu, kv_q8))
            };

        let (model, ctx, initial_ctx, safe_ceiling, gpu_active, kv_q8_active) =
            match try_load_and_ctx(want_gpu) {
                Ok(ok) => ok,
                Err(gpu_err) if want_gpu => {
                    crate::logger::log_warn(&format!(
                        "GPU-inläsning misslyckades ({gpu_err}) — faller automatiskt tillbaka till CPU (n_gpu_layers=0)!"
                    ));
                    try_load_and_ctx(false).map_err(|cpu_err| {
                        crate::logger::log_error(&cpu_err);
                        cpu_err
                    })?
                }
                Err(e) => {
                    crate::logger::log_error(&e);
                    return Err(e);
                }
            };

        crate::logger::log_info(&format!(
            "Modell + kontext redo på {} ms (RAM={:.1} GB, gpu={}, kv_q8={}, safe_ceiling={}, initial_ctx={})",
            t_load.elapsed().as_millis(),
            total_ram_gb,
            gpu_active,
            kv_q8_active,
            safe_ceiling,
            initial_ctx
        ));

        Ok(Self {
            ctx: Some(ctx),
            model,
            model_id: spec.id.to_string(),
            model_label: spec.label.to_string(),
            gpu_active,
            kv_q8_active,
            current_n_ctx: initial_ctx,
            safe_ceiling,
            cached_tokens: Vec::with_capacity(initial_ctx as usize),
        })
    }

    fn create_context(
        model: &Box<LlamaModel>,
        backend: &'static LlamaBackend,
        n_ctx: u32,
        use_gpu: bool,
    ) -> Result<(LlamaContext<'static>, bool), String> {
        let (gen_threads, batch_threads) = optimal_cpu_threads();
        let (n_batch, n_ubatch) = if use_gpu {
            (GPU_PREFILL_BATCH as u32, GPU_PREFILL_UBATCH)
        } else {
            (CPU_PREFILL_BATCH as u32, CPU_PREFILL_UBATCH)
        };

        // SAFETY: `model` ägs i en `Box<LlamaModel>` på stabil minnesadress inne i `SummarizerEngine`,
        // flyttas aldrig ur sin heap-allokering och `self.ctx` töms alltid explicit före `model` i `Drop`.
        let model_static_ref: &'static LlamaModel = unsafe { &*(&**model as *const LlamaModel) };

        let base_params = || {
            LlamaContextParams::default()
                .with_n_ctx(NonZeroU32::new(n_ctx))
                .with_n_batch(n_batch)
                .with_n_ubatch(n_ubatch)
                .with_swa_full(true)
                .with_no_perf(true)
                .with_n_threads(gen_threads)
                .with_n_threads_batch(batch_threads)
        };

        // På Apple Silicon (Metal) har GPU:n dedikerade hårdvaruenheter för F16 (`simdgroup_half8x8`)
        // utan kvantiserings-overhead vid prefill. På Windows (Vulkan/CPU) halverar däremot Q8_0
        // KV-cachens VRAM-/RAM-förbrukning så att dubbelt så långa möten ryms i grafikkortsminnet!
        if !cfg!(target_os = "macos") {
            let q8_params = base_params()
                .with_type_k(KvCacheType::Q8_0)
                .with_type_v(KvCacheType::Q8_0);

            if let Ok(ctx) = model_static_ref.new_context(backend, q8_params) {
                crate::logger::log_info(&format!(
                    "Inferenskontext allokerad: n_ctx={n_ctx}, batch={n_batch}/{n_ubatch}, kv_cache=Q8_0 (Flash Attention), gpu={use_gpu}"
                ));
                return Ok((ctx, true));
            }
        }

        let f16_ctx = model_static_ref
            .new_context(backend, base_params())
            .map_err(|e| format!("Kunde inte allokera kontextfönster ({n_ctx} tokens): {e}"))?;
        crate::logger::log_info(&format!(
            "Inferenskontext allokerad: n_ctx={n_ctx}, batch={n_batch}/{n_ubatch}, kv_cache=F16 (Flash Attention Auto), gpu={use_gpu}"
        ));
        Ok((f16_ctx, false))
    }

    fn ensure_context_capacity(&mut self, needed_ctx: u32) -> Result<(), String> {
        let base_floor = if self.gpu_active {
            DEFAULT_MIN_CTX
        } else {
            WEAK_HW_EVIDENCE_CTX
        };
        let min_floor = base_floor.min(self.safe_ceiling);
        let target = round_ctx_up(needed_ctx).clamp(min_floor, self.safe_ceiling);
        if self.ctx.is_some() && target <= self.current_n_ctx {
            return Ok(());
        }
        // Lägg till marginal för "Noggrannare"-läget när vi ändå växer kontexten så vi slipper riva den igen!
        let grown_target = round_ctx_up(needed_ctx.saturating_add(4096)).clamp(target, self.safe_ceiling);

        // Släpp gamla kontexten först så minnet frigörs innan det större kontextfönstret allokeras
        self.ctx = None;
        self.cached_tokens.clear();
        let backend = ensure_llama_backend()?;
        match Self::create_context(&self.model, backend, grown_target, self.gpu_active) {
            Ok((new_ctx, kv_q8)) => {
                self.ctx = Some(new_ctx);
                self.current_n_ctx = grown_target;
                self.kv_q8_active = kv_q8;
                Ok(())
            }
            Err(err) if grown_target > WEAK_HW_EVIDENCE_CTX => {
                crate::logger::log_warn(&format!(
                    "Kunde inte växa kontextfönstret till {grown_target} tokens ({err}) — begränsar till {WEAK_HW_EVIDENCE_CTX} och använder diskbaserade bevispaket."
                ));
                let fallback = WEAK_HW_EVIDENCE_CTX.min(self.safe_ceiling);
                let (new_ctx, kv_q8) =
                    Self::create_context(&self.model, backend, fallback, self.gpu_active)?;
                self.safe_ceiling = fallback;
                self.current_n_ctx = fallback;
                self.kv_q8_active = kv_q8;
                self.ctx = Some(new_ctx);
                Ok(())
            }
            Err(err) => Err(err),
        }
    }

    /// Tyst bakgrundsförvärmning på GPU: läser stegvis in det invarianta transkript-prefixet i KV-cachen
    /// medan användaren fyller i mötesrubrik/mall/sammanhang. Avbryts omedelbart om `cancel_flag` sätts
    /// och lämnar alla hittills inlästa batchar redo att återanvändas direkt av `run_summarize`!
    pub fn prewarm_transcript(
        &mut self,
        transcript: &str,
        thorough: bool,
        cancel_flag: &AtomicBool,
    ) -> Result<usize, String> {
        if !self.gpu_active || transcript.trim().is_empty() {
            return Ok(0);
        }

        let prefix = build_transcript_prefix(transcript, thorough);
        let full_tokens = self
            .model
            .vocab()
            .tokenize(prefix.as_bytes(), false, true);

        // Lämna de sista 2 gränstoken utanför så BPE-sammanslagning mot efterföljande text aldrig kan skilja sig
        let safe_len = full_tokens.len().saturating_sub(2);
        if safe_len < 64 {
            return Ok(0);
        }
        let target_tokens = &full_tokens[..safe_len];

        let needed_total = (target_tokens.len() as u32).saturating_add(RESPONSE_MARGIN_THOROUGH + 1024);
        if needed_total > self.safe_ceiling {
            return Ok(0);
        }
        self.ensure_context_capacity(needed_total)?;
        if needed_total > self.current_n_ctx {
            return Ok(0);
        }

        let common_prefix = self
            .cached_tokens
            .iter()
            .zip(target_tokens.iter())
            .take_while(|(a, b)| a == b)
            .count();

        if common_prefix >= target_tokens.len() {
            return Ok(target_tokens.len());
        }

        let mut keep_len = common_prefix;
        let ctx = self
            .ctx
            .as_mut()
            .ok_or_else(|| "Inferenskontext saknas vid förvärmning.".to_string())?;

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

        let to_decode = &target_tokens[keep_len..];
        let t_warm = Instant::now();
        crate::logger::log_info(&format!(
            "Startar tyst GPU-förvärmning av mötestext ({} nya tokens, redan i cache={keep_len})…",
            to_decode.len()
        ));

        let mut batch = LlamaBatch::new(PREWARM_BATCH, 1);
        for chunk_start in (0..to_decode.len()).step_by(PREWARM_BATCH) {
            if cancel_flag.load(Ordering::Relaxed) {
                crate::logger::log_info(&format!(
                    "GPU-förvärmning lämnade över låset vid {} cachade tokens ({} ms)",
                    self.cached_tokens.len(),
                    t_warm.elapsed().as_millis()
                ));
                return Ok(self.cached_tokens.len());
            }

            let chunk_end = (chunk_start + PREWARM_BATCH).min(to_decode.len());
            batch.clear();
            for (offset, &tok) in to_decode[chunk_start..chunk_end].iter().enumerate() {
                let pos = (keep_len + chunk_start + offset) as i32;
                batch
                    .add(tok, pos, &[0], false)
                    .map_err(|e| format!("Batch-fel vid bakgrundsförvärmning: {e}"))?;
            }

            ctx.decode(&mut batch)
                .map_err(|e| format!("Fel vid bakgrundsförvärmning av KV-cache: {e}"))?;
            self.cached_tokens
                .extend_from_slice(&to_decode[chunk_start..chunk_end]);
        }

        crate::logger::log_info(&format!(
            "GPU-förvärmning klar: {} tokens redo i varm KV-cache på {} ms!",
            self.cached_tokens.len(),
            t_warm.elapsed().as_millis()
        ));
        Ok(self.cached_tokens.len())
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
        evidence_cache_root: Option<&Path>,
        cancel_flag: &AtomicBool,
        mut on_progress: FProg,
        mut on_chunk: FChunk,
    ) -> Result<SummarizeResult, String>
    where
        FProg: FnMut(SummarizeProgressPayload),
        FChunk: FnMut(SummarizeChunkPayload),
    {
        let t0 = Instant::now();
        on_progress(SummarizeProgressPayload {
            phase: "prefill".to_string(),
            label: "Förbereder och tokeniserar mötestexten…".to_string(),
            prefill_percent: 0,
            kv_reused_tokens: 0,
            prompt_tokens: 0,
            thinking_tokens: 0,
            generated_tokens: 0,
            tokens_per_sec: 0.0,
            chunk_current: None,
            chunk_total: None,
        });

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
        let effective_single_ceiling = if self.gpu_active {
            self.safe_ceiling
        } else {
            self.safe_ceiling.min(CPU_SINGLE_PASS_MAX_CTX)
        };

        let is_cleanup = req.template_id == "transcription-cleanup";
        let user_requested_deep = req.deep_evidence.unwrap_or(false);
        let workshop_auto_deep =
            req.template_id == "thematic-workshop" && prompt_tokens.len() >= 5_500;
        let force_evidence =
            !is_cleanup && ((user_requested_deep && prompt_tokens.len() >= 900) || workshop_auto_deep);

        crate::logger::log_info(&format!(
            "run_summarize: mall='{}', prompt_tokens={}, needed_total={}, single_ceiling={}, safe_ceiling={}, gpu={}, kv_q8={}, thorough={}, creative={}, deep_evidence={}",
            req.template_id,
            prompt_tokens.len(),
            needed_total,
            effective_single_ceiling,
            self.safe_ceiling,
            self.gpu_active,
            self.kv_q8_active,
            req.thorough,
            req.creative,
            force_evidence
        ));

        if !force_evidence && needed_total <= effective_single_ceiling {
            self.ensure_context_capacity(needed_total)?;
            if needed_total <= self.current_n_ctx {
                let stats = self.generate_from_tokens(
                    &prompt_tokens,
                    req.thorough,
                    temperature,
                    repeat_penalty,
                    presence_penalty,
                    StepLabel::SingleDocument,
                    MAX_ANSWER_TOKENS,
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
        }

        // Specialflöde för "Rätta transkriberingen" på långa möten: sekventiell korrekturläsning utan trunkerande REDUCE_PROMPT!
        if is_cleanup {
            return self.run_long_cleanup_stream(
                req,
                prompt_tokens.len(),
                temperature,
                repeat_penalty,
                presence_penalty,
                t0,
                cancel_flag,
                &mut on_progress,
                &mut on_chunk,
            );
        }

        // Diskbaserad Bevisextrahering & Tvåstegssyntes för långa möten och svag hårdvara!
        let default_cache_dir = std::env::temp_dir().join("loki_evidence_cache");
        let cache_root = evidence_cache_root.unwrap_or(&default_cache_dir);
        self.run_evidence_pipeline(
            req,
            cache_root,
            prompt_tokens.len(),
            margin,
            temperature,
            repeat_penalty,
            presence_penalty,
            t0,
            cancel_flag,
            &mut on_progress,
            &mut on_chunk,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn run_long_cleanup_stream<FProg, FChunk>(
        &mut self,
        req: &SummarizeRequest,
        total_prompt_tokens: usize,
        temperature: f32,
        repeat_penalty: f32,
        presence_penalty: f32,
        t0: Instant,
        cancel_flag: &AtomicBool,
        on_progress: &mut FProg,
        on_chunk: &mut FChunk,
    ) -> Result<SummarizeResult, String>
    where
        FProg: FnMut(SummarizeProgressPayload),
        FChunk: FnMut(SummarizeChunkPayload),
    {
        self.ensure_context_capacity(WEAK_HW_EVIDENCE_CTX)?;
        let transcript_tok_len = self.count_tokens(&req.transcript).max(1);
        let chars_per_tok =
            (req.transcript.chars().count() as f64) / (transcript_tok_len as f64);
        let chunks = crate::evidence::split_transcript_on_turns(
            &req.transcript,
            CLEANUP_CHUNK_TOKENS,
            chars_per_tok,
            0,
        );
        let total_chunks = chunks.len().max(1);

        let rolling_state = crate::evidence::RollingMeetingState::default();
        let mut accumulated = String::new();
        let mut total_gen_tokens = 0usize;
        let mut last_tps = 0.0f32;

        for (idx, chunk_text) in chunks.into_iter().enumerate() {
            if cancel_flag.load(Ordering::Relaxed) {
                return Err(CANCELLED_ERR.to_string());
            }
            let c_num = idx + 1;
            let c_prompt = build_cleanup_chunk_prompt(
                &chunk_text,
                c_num,
                total_chunks,
                &rolling_state,
                req.context.as_deref(),
            );
            let c_tokens = self
                .model
                .vocab()
                .tokenize(c_prompt.as_bytes(), false, true);

            let prefix_so_far = accumulated.clone();
            let gen_so_far = total_gen_tokens;
            let mut stream_cb = |payload: SummarizeChunkPayload| {
                let merged = if prefix_so_far.is_empty() {
                    payload.full_text
                } else {
                    format!("{prefix_so_far}\n\n{}", payload.full_text)
                };
                on_chunk(SummarizeChunkPayload {
                    full_text: merged,
                    generated_tokens: gen_so_far + payload.generated_tokens,
                    tokens_per_sec: payload.tokens_per_sec,
                });
            };

            let stats = self.generate_from_tokens(
                &c_tokens,
                false,
                temperature,
                repeat_penalty,
                presence_penalty,
                StepLabel::CleanupChunk {
                    idx: c_num,
                    total: total_chunks,
                },
                MAX_ANSWER_TOKENS,
                cancel_flag,
                on_progress,
                &mut stream_cb,
            )?;

            total_gen_tokens += stats.generated_tokens;
            last_tps = stats.tokens_per_sec;
            if !stats.text.trim().is_empty() {
                if !accumulated.is_empty() {
                    accumulated.push_str("\n\n");
                }
                accumulated.push_str(stats.text.trim());
            }
        }

        on_chunk(SummarizeChunkPayload {
            full_text: accumulated.clone(),
            generated_tokens: total_gen_tokens,
            tokens_per_sec: last_tps,
        });

        Ok(SummarizeResult {
            markdown: accumulated,
            model_id: self.model_id.clone(),
            model_label: self.model_label.clone(),
            prompt_tokens: total_prompt_tokens,
            kv_reused_tokens: 0,
            generated_tokens: total_gen_tokens,
            thinking_tokens: 0,
            duration_ms: t0.elapsed().as_millis() as u64,
            tokens_per_sec: last_tps,
            strategy: "cleanup-stream".to_string(),
            used_temperature: temperature,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn run_evidence_pipeline<FProg, FChunk>(
        &mut self,
        req: &SummarizeRequest,
        cache_root: &Path,
        total_prompt_tokens: usize,
        margin: u32,
        temperature: f32,
        repeat_penalty: f32,
        presence_penalty: f32,
        t0: Instant,
        cancel_flag: &AtomicBool,
        on_progress: &mut FProg,
        on_chunk: &mut FChunk,
    ) -> Result<SummarizeResult, String>
    where
        FProg: FnMut(SummarizeProgressPayload),
        FChunk: FnMut(SummarizeChunkPayload),
    {
        self.ensure_context_capacity(WEAK_HW_EVIDENCE_CTX)?;
        let transcript_tok_len = self.count_tokens(&req.transcript).max(1);
        let chars_per_tok =
            (req.transcript.chars().count() as f64) / (transcript_tok_len as f64);
        let chunks = crate::evidence::split_transcript_on_turns(
            &req.transcript,
            EVIDENCE_CHUNK_TOKENS,
            chars_per_tok,
            EVIDENCE_OVERLAP_TOKENS,
        );
        let total_chunks = chunks.len().max(1);

        let store = crate::evidence::EvidenceDiskStore::open(
            cache_root,
            &self.model_id,
            req.context.as_deref(),
            total_chunks,
            &req.transcript,
        )?;

        crate::logger::log_info(&format!(
            "Startar diskbaserad bevispipeline: {} avsnitt (~{} tokens/avsnitt), modell='{}'",
            total_chunks, EVIDENCE_CHUNK_TOKENS, self.model_id
        ));

        let mut rolling_state = crate::evidence::RollingMeetingState::default();
        let mut total_gen_tokens = 0usize;
        let mut total_think_tokens = 0usize;
        let mut disk_reused_chunks = 0usize;

        for (idx, chunk_text) in chunks.into_iter().enumerate() {
            if cancel_flag.load(Ordering::Relaxed) {
                return Err(CANCELLED_ERR.to_string());
            }
            let c_num = idx + 1;

            // Checkpointing & Mallbytes-cache: läs direkt från lokal disk om avsnittet redan är extraherat i sessionen!
            if let Some(cached_pkt) = store.load_chunk(c_num) {
                rolling_state.update_from_packet(&cached_pkt);
                disk_reused_chunks += 1;
                on_progress(SummarizeProgressPayload {
                    phase: "extracting_evidence".to_string(),
                    label: format!(
                        "Återanvänder sparat bevispaket {c_num} av {total_chunks} från lokal disk…"
                    ),
                    prefill_percent: ((c_num * 100) / total_chunks) as u8,
                    kv_reused_tokens: disk_reused_chunks * EVIDENCE_CHUNK_TOKENS,
                    prompt_tokens: total_prompt_tokens,
                    thinking_tokens: total_think_tokens,
                    generated_tokens: total_gen_tokens,
                    tokens_per_sec: 0.0,
                    chunk_current: Some(c_num),
                    chunk_total: Some(total_chunks),
                });
                continue;
            }

            let extract_prompt = crate::evidence::build_evidence_extract_prompt(
                &chunk_text,
                c_num,
                total_chunks,
                &rolling_state,
                req.context.as_deref(),
                false,
            );
            let extract_tokens = self
                .model
                .vocab()
                .tokenize(extract_prompt.as_bytes(), false, true);

            // Rensa cached_tokens inför fristående avsnitt så KV-cachen hålls minimal i RAM
            self.cached_tokens.clear();
            let mut silent_chunk = |_c: SummarizeChunkPayload| {};
            let part_stats = self.generate_from_tokens(
                &extract_tokens,
                false,
                0.05,
                1.08,
                0.0,
                StepLabel::EvidenceExtract {
                    idx: c_num,
                    total: total_chunks,
                },
                1_280,
                cancel_flag,
                on_progress,
                &mut silent_chunk,
            )?;

            total_gen_tokens += part_stats.generated_tokens;
            total_think_tokens += part_stats.thinking_tokens;

            let pkt = crate::evidence::parse_evidence_packet(&part_stats.text, c_num, total_chunks);
            store.save_chunk(&pkt)?;
            rolling_state.update_from_packet(&pkt);
        }

        let all_packets = store.load_all_chunks()?;
        let has_any_substance = all_packets.iter().any(|p| p.has_substance());
        if !has_any_substance {
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
                prompt_tokens: total_prompt_tokens,
                kv_reused_tokens: disk_reused_chunks * EVIDENCE_CHUNK_TOKENS,
                generated_tokens: total_gen_tokens,
                thinking_tokens: total_think_tokens,
                duration_ms: t0.elapsed().as_millis() as u64,
                tokens_per_sec: 0.0,
                strategy: "evidence".to_string(),
                used_temperature: temperature,
            });
        }

        let consolidated_register =
            crate::evidence::format_consolidated_decisions_and_actions(&all_packets);

        // För ren Åtgärdspunkter- eller Beslutslogg-mall räcker det konsoliderade registret + kort kontext
        // nästan alltid för ett enda blixtsnabbt syntespass även på 150 000-tokensmöten!
        let mut full_evidence_text = String::new();
        if (req.template_id == "action-items" || req.template_id == "decision-log")
            && !consolidated_register.is_empty()
            && all_packets.len() > 10
        {
            for pkt in &all_packets {
                if !pkt.decisions.is_empty() || !pkt.actions.is_empty() {
                    full_evidence_text.push_str(&pkt.format_for_synthesis());
                    full_evidence_text.push('\n');
                }
            }
        }
        if full_evidence_text.is_empty() {
            for pkt in &all_packets {
                let s = pkt.format_for_synthesis();
                if !s.is_empty() {
                    full_evidence_text.push_str(&s);
                    full_evidence_text.push('\n');
                }
            }
        }

        let direct_prompt =
            build_evidence_synthesis_prompt(&full_evidence_text, &consolidated_register, req);
        let direct_tokens = self
            .model
            .vocab()
            .tokenize(direct_prompt.as_bytes(), false, true);
        let evidence_only_tokens = self.count_tokens(&full_evidence_text);
        let max_direct_ctx = if self.gpu_active {
            self.safe_ceiling.min(24_576)
        } else {
            self.safe_ceiling.min(CPU_SINGLE_PASS_MAX_CTX)
        };
        let needed_direct = (direct_tokens.len() as u32).saturating_add(margin);

        // ── Pass 2 Alternativ A: Samlad Direktsyntes från Bevispaket (`evidence`) ──
        if evidence_only_tokens <= DIRECT_SYNTHESIS_MAX_EVIDENCE_TOKENS
            && needed_direct <= max_direct_ctx
        {
            self.ensure_context_capacity(needed_direct)?;
            if needed_direct <= self.current_n_ctx {
                let synth_label = if disk_reused_chunks == total_chunks {
                    format!(
                        "Skapar {} direkt ur sparade bevispaket på disk…",
                        req.template_label.to_lowercase()
                    )
                } else {
                    format!(
                        "Steg 2/2: Skapar {} utifrån samtliga bevispaket…",
                        req.template_label.to_lowercase()
                    )
                };
                on_progress(SummarizeProgressPayload {
                    phase: "reducing".to_string(),
                    label: synth_label,
                    prefill_percent: 0,
                    kv_reused_tokens: disk_reused_chunks * EVIDENCE_CHUNK_TOKENS,
                    prompt_tokens: direct_tokens.len(),
                    thinking_tokens: total_think_tokens,
                    generated_tokens: total_gen_tokens,
                    tokens_per_sec: 0.0,
                    chunk_current: Some(total_chunks),
                    chunk_total: Some(total_chunks),
                });

                self.cached_tokens.clear();
                let synth_stats = self.generate_from_tokens(
                    &direct_tokens,
                    req.thorough,
                    temperature,
                    repeat_penalty,
                    presence_penalty,
                    StepLabel::SingleDocument,
                    MAX_ANSWER_TOKENS,
                    cancel_flag,
                    on_progress,
                    on_chunk,
                )?;

                total_gen_tokens += synth_stats.generated_tokens;
                total_think_tokens += synth_stats.thinking_tokens;

                return Ok(SummarizeResult {
                    markdown: synth_stats.text,
                    model_id: self.model_id.clone(),
                    model_label: self.model_label.clone(),
                    prompt_tokens: total_prompt_tokens,
                    kv_reused_tokens: disk_reused_chunks * EVIDENCE_CHUNK_TOKENS,
                    generated_tokens: total_gen_tokens,
                    thinking_tokens: total_think_tokens,
                    duration_ms: t0.elapsed().as_millis() as u64,
                    tokens_per_sec: synth_stats.tokens_per_sec,
                    strategy: "evidence".to_string(),
                    used_temperature: temperature,
                });
            }
        }

        // ── Pass 2 Alternativ B: Tvåstegskodning från disk (`evidence-2pass` för 60k–200k+ möten / workshops) ──
        crate::logger::log_info(&format!(
            "Bevisvolym {} tokens överstiger direktsyntesgräns — kör tvåstegskodning (Temamatris -> Sektionssyntes från disk)",
            evidence_only_tokens
        ));

        self.ensure_context_capacity(WEAK_HW_EVIDENCE_CTX)?;
        on_progress(SummarizeProgressPayload {
            phase: "thematic_coding".to_string(),
            label: format!(
                "Steg 2/3: Kartlägger gemensamma teman över alla {total_chunks} mötesavsnitt…"
            ),
            prefill_percent: 0,
            kv_reused_tokens: disk_reused_chunks * EVIDENCE_CHUNK_TOKENS,
            prompt_tokens: total_prompt_tokens,
            thinking_tokens: total_think_tokens,
            generated_tokens: total_gen_tokens,
            tokens_per_sec: 0.0,
            chunk_current: None,
            chunk_total: Some(total_chunks),
        });

        let matrix_prompt = crate::evidence::build_theme_matrix_prompt(
            &all_packets,
            req.agenda.as_deref(),
            req.context.as_deref(),
        );
        let matrix_tokens = self
            .model
            .vocab()
            .tokenize(matrix_prompt.as_bytes(), false, true);
        let needed_matrix = (matrix_tokens.len() as u32).saturating_add(1024);
        self.ensure_context_capacity(needed_matrix.min(self.safe_ceiling))?;

        self.cached_tokens.clear();
        let mut silent_matrix_chunk = |_c: SummarizeChunkPayload| {};
        let matrix_stats = self.generate_from_tokens(
            &matrix_tokens,
            false,
            0.05,
            1.05,
            0.0,
            StepLabel::ThematicMatrix,
            768,
            cancel_flag,
            on_progress,
            &mut silent_matrix_chunk,
        )?;
        total_gen_tokens += matrix_stats.generated_tokens;

        let clusters =
            crate::evidence::parse_theme_clusters_with_full_coverage(&matrix_stats.text, &all_packets);
        // Max ~11 500 tecken bevis per batch (~3 000–3 400 tokens -> ryms alltid i ett 8 192-fönster på svag hårdvara!)
        let theme_batches =
            crate::evidence::batch_theme_clusters_for_budget(&clusters, &all_packets, 11_500);
        let total_batches = theme_batches.len().max(1);

        let mut accumulated_doc = String::new();
        let mut completed_titles: Vec<String> = Vec::new();
        let mut last_tps = 0.0f32;

        for (b_idx, batch_clusters) in theme_batches.into_iter().enumerate() {
            if cancel_flag.load(Ordering::Relaxed) {
                return Err(CANCELLED_ERR.to_string());
            }
            let step_no = b_idx + 1;
            let first_title = batch_clusters
                .first()
                .map(|c| c.title.clone())
                .unwrap_or_else(|| format!("Del {step_no}"));

            let sec_prompt = build_section_synthesis_prompt(
                &batch_clusters,
                &all_packets,
                &completed_titles,
                req,
            );
            let sec_tokens = self
                .model
                .vocab()
                .tokenize(sec_prompt.as_bytes(), false, true);
            let needed_sec = (sec_tokens.len() as u32).saturating_add(margin);
            self.ensure_context_capacity(needed_sec.min(self.safe_ceiling))?;

            let prefix_so_far = accumulated_doc.clone();
            let gen_so_far = total_gen_tokens;
            let mut stream_sec = |payload: SummarizeChunkPayload| {
                let merged = if prefix_so_far.is_empty() {
                    payload.full_text
                } else {
                    format!("{prefix_so_far}\n\n{}", payload.full_text)
                };
                on_chunk(SummarizeChunkPayload {
                    full_text: merged,
                    generated_tokens: gen_so_far + payload.generated_tokens,
                    tokens_per_sec: payload.tokens_per_sec,
                });
            };

            self.cached_tokens.clear();
            let sec_stats = self.generate_from_tokens(
                &sec_tokens,
                req.thorough,
                temperature,
                repeat_penalty,
                presence_penalty,
                StepLabel::SynthesisSection {
                    idx: step_no,
                    total: total_batches,
                    title: &first_title,
                },
                2_560,
                cancel_flag,
                on_progress,
                &mut stream_sec,
            )?;

            total_gen_tokens += sec_stats.generated_tokens;
            total_think_tokens += sec_stats.thinking_tokens;
            last_tps = sec_stats.tokens_per_sec;

            if !sec_stats.text.trim().is_empty() {
                if !accumulated_doc.is_empty() {
                    accumulated_doc.push_str("\n\n");
                }
                accumulated_doc.push_str(sec_stats.text.trim());
            }
            for c in batch_clusters {
                completed_titles.push(c.title);
            }
        }

        // Avslutande sammanslagning av samtliga beslut & åtgärder ur kontrollregistret så noll åtgärder tappas!
        if !consolidated_register.is_empty() && req.template_id != "summary" {
            let app_prompt = build_decisions_appendix_prompt(&consolidated_register, false);
            let app_tokens = self
                .model
                .vocab()
                .tokenize(app_prompt.as_bytes(), false, true);
            let needed_app = (app_tokens.len() as u32).saturating_add(2048);
            if needed_app <= self.safe_ceiling {
                self.ensure_context_capacity(needed_app)?;
                let prefix_so_far = accumulated_doc.clone();
                let gen_so_far = total_gen_tokens;
                let mut stream_app = |payload: SummarizeChunkPayload| {
                    let merged = if prefix_so_far.is_empty() {
                        payload.full_text
                    } else {
                        format!("{prefix_so_far}\n\n---\n\n{}", payload.full_text)
                    };
                    on_chunk(SummarizeChunkPayload {
                        full_text: merged,
                        generated_tokens: gen_so_far + payload.generated_tokens,
                        tokens_per_sec: payload.tokens_per_sec,
                    });
                };

                self.cached_tokens.clear();
                if let Ok(app_stats) = self.generate_from_tokens(
                    &app_tokens,
                    false,
                    0.05,
                    1.08,
                    0.0,
                    StepLabel::DecisionsAppendix,
                    1_536,
                    cancel_flag,
                    on_progress,
                    &mut stream_app,
                ) {
                    total_gen_tokens += app_stats.generated_tokens;
                    last_tps = app_stats.tokens_per_sec;
                    if !app_stats.text.trim().is_empty() {
                        if !accumulated_doc.is_empty() {
                            accumulated_doc.push_str("\n\n---\n\n");
                        }
                        accumulated_doc.push_str(app_stats.text.trim());
                    }
                }
            }
        }

        on_chunk(SummarizeChunkPayload {
            full_text: accumulated_doc.clone(),
            generated_tokens: total_gen_tokens,
            tokens_per_sec: last_tps,
        });

        Ok(SummarizeResult {
            markdown: accumulated_doc,
            model_id: self.model_id.clone(),
            model_label: self.model_label.clone(),
            prompt_tokens: total_prompt_tokens,
            kv_reused_tokens: disk_reused_chunks * EVIDENCE_CHUNK_TOKENS,
            generated_tokens: total_gen_tokens,
            thinking_tokens: total_think_tokens,
            duration_ms: t0.elapsed().as_millis() as u64,
            tokens_per_sec: last_tps,
            strategy: "evidence-2pass".to_string(),
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
            StepLabel::SingleDocument,
            MAX_ANSWER_TOKENS,
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
        step_label: StepLabel<'_>,
        max_answer_tokens: usize,
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
        let chunk_pair = step_label.chunk_pair();

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

        let prefill_batch = if self.gpu_active {
            GPU_PREFILL_BATCH
        } else {
            CPU_PREFILL_BATCH
        };
        let to_decode = &prompt_tokens[keep_len..];
        let total_to_decode = to_decode.len();
        crate::logger::log_info(&format!(
            "generate_from_tokens: total_prompt={}, kv_keep={}, to_decode={}, batch_size={}",
            prompt_tokens.len(),
            keep_len,
            total_to_decode,
            prefill_batch
        ));
        let mut batch = LlamaBatch::new(prefill_batch, 1);
        let mut last_chunk_len = 0usize;

        for (step_idx, chunk_start) in (0..total_to_decode).step_by(prefill_batch).enumerate() {
            if cancel_flag.load(Ordering::Relaxed) {
                return Err(CANCELLED_ERR.to_string());
            }

            let chunk_end = (chunk_start + prefill_batch).min(total_to_decode);
            last_chunk_len = chunk_end - chunk_start;

            let pct = (((keep_len + chunk_start) as f64 / (prompt_tokens.len() as f64)) * 100.0)
                .round()
                .clamp(0.0, 99.0) as u8;
            let label = match step_label {
                StepLabel::EvidenceExtract { idx, total } => format!(
                    "Steg 1/2: Läser in avsnitt {idx} av {total} ({pct} %) — minnessnålt läge…"
                ),
                StepLabel::ThematicMatrix => {
                    format!("Steg 2/3: Läser avsnittsöversikt för temakodning ({pct} %)…")
                }
                StepLabel::SynthesisSection { idx, total, title } => {
                    format!("Steg 3/3: Läser bevispaket för sektion {idx}/{total} – {title} ({pct} %)…")
                }
                StepLabel::DecisionsAppendix => {
                    format!("Sammanställer fullständigt besluts- och åtgärdsregister ({pct} %)…")
                }
                StepLabel::CleanupChunk { idx, total } => {
                    format!("Läser in del {idx} av {total} för korrekturläsning ({pct} %)…")
                }
                StepLabel::SingleDocument => {
                    if keep_len > 500 && step_idx == 0 {
                        format!("Återanvänder mötet ur varm cache ({keep_len} tokens)…")
                    } else {
                        format!("Läser in mötestexten ({pct} %)…")
                    }
                }
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
                chunk_current: chunk_pair.map(|x| x.0),
                chunk_total: chunk_pair.map(|x| x.1),
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

            let t_batch = Instant::now();
            ctx.decode(&mut batch).map_err(|e| {
                let msg = format!("Kunde inte läsa in mötesunderlaget i modellen: {e}");
                crate::logger::log_error(&msg);
                msg
            })?;
            if step_idx == 0 {
                crate::logger::log_info(&format!(
                    "Första prefill-batch ({} tokens) klar på {} ms",
                    last_chunk_len,
                    t_batch.elapsed().as_millis()
                ));
            }
            self.cached_tokens
                .extend_from_slice(&to_decode[chunk_start..chunk_end]);
        }

        // Konfigurera sampler: ren greedy eller lätt kedja med temperatur + repetitionsstraff
        let mut sampler = build_sampler(n_vocab, temperature, repeat_penalty, presence_penalty);

        // Identifiera Gemma 4 `<|channel>` och `<channel|>` för tankekanalen
        let channel_open_tok = vocab.tokenize(b"<|channel>", false, true).first().copied();
        let channel_end_tok = vocab.tokenize(b"<channel|>", false, true).first().copied();
        let turn_end_tok = vocab.tokenize(b"<turn|>", false, true).first().copied();
        let forced_close_seq = vocab.tokenize(b"\n<channel|>", false, true);

        let mut in_thought = false;
        let mut thinking_tokens = 0usize;
        let thought_budget = if thorough { MAX_THOUGHT_TOKENS } else { 2 };
        let mut answer_tokens: Vec<LlamaToken> = Vec::with_capacity(1024);
        let mut step_batch = LlamaBatch::new(16, 1);
        let mut cur_pos = prompt_tokens.len() as i32;
        let mut logits_idx = (last_chunk_len as i32) - 1;

        let gen_start = Instant::now();
        let mut last_ui_emit = Instant::now()
            .checked_sub(Duration::from_millis(50))
            .unwrap_or_else(Instant::now);

        let max_total_steps = if thorough {
            MAX_THOUGHT_TOKENS + max_answer_tokens
        } else {
            max_answer_tokens + 16
        };

        for _ in 0..max_total_steps {
            if cancel_flag.load(Ordering::Relaxed) {
                return Err(CANCELLED_ERR.to_string());
            }

            // Om tankebudgeten nås (eller direkt i snabbläge om modellen ändå öppnar `<|channel>thought`):
            // injicera `\n<channel|>` i KV-cachen så modellen omedelbart börjar skriva svaret!
            if in_thought && thinking_tokens >= thought_budget && !forced_close_seq.is_empty() {
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
            if vocab.is_eog(next_token) || Some(next_token) == turn_end_tok {
                break;
            }

            if Some(next_token) == channel_open_tok {
                in_thought = true;
            } else if in_thought {
                if Some(next_token) == channel_end_tok {
                    in_thought = false;
                } else {
                    thinking_tokens += 1;
                    if thorough && last_ui_emit.elapsed() >= Duration::from_millis(80) {
                        last_ui_emit = Instant::now();
                        let secs = gen_start.elapsed().as_secs_f32().max(0.001);
                        let tps = (thinking_tokens as f32) / secs;
                        let lbl = match step_label {
                            StepLabel::SynthesisSection { idx, total, title } => format!(
                                "Resonerar kring sektion {idx}/{total} – {title} ({thinking_tokens} tanketokens)…"
                            ),
                            _ => format!(
                                "Tänker igenom struktur och detaljer ({thinking_tokens} tanketokens)…"
                            ),
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
                            chunk_current: chunk_pair.map(|x| x.0),
                            chunk_total: chunk_pair.map(|x| x.1),
                        });
                    }
                }
            } else if Some(next_token) != channel_end_tok {
                answer_tokens.push(next_token);

                if answer_tokens.len() >= max_answer_tokens {
                    break;
                }

                if last_ui_emit.elapsed() >= Duration::from_millis(45) {
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

                    let (phase_str, lbl) = match step_label {
                        StepLabel::EvidenceExtract { idx, total } => (
                            "extracting_evidence",
                            format!("Steg 1/2: Extraherar bevispaket {idx} av {total} till lokal disk…"),
                        ),
                        StepLabel::ThematicMatrix => (
                            "thematic_coding",
                            "Steg 2/3: Bygger gemensam temamatris över mötets avsnitt…".to_string(),
                        ),
                        StepLabel::SynthesisSection { idx, total, title } => (
                            "generating",
                            format!("Steg 3/3: Skriver sektion {idx}/{total} ({title})…"),
                        ),
                        StepLabel::DecisionsAppendix => (
                            "generating",
                            "Sammanställer avslutande beslut och åtgärder…".to_string(),
                        ),
                        StepLabel::CleanupChunk { idx, total } => (
                            "generating",
                            format!("Korrekturläser del {idx} av {total}…"),
                        ),
                        StepLabel::SingleDocument => {
                            ("generating", "Skriver mötesdokument…".to_string())
                        }
                    };
                    on_progress(SummarizeProgressPayload {
                        phase: phase_str.to_string(),
                        label: lbl,
                        prefill_percent: 100,
                        kv_reused_tokens: keep_len,
                        prompt_tokens: prompt_tokens.len(),
                        thinking_tokens,
                        generated_tokens: answer_tokens.len(),
                        tokens_per_sec: tps,
                        chunk_current: chunk_pair.map(|x| x.0),
                        chunk_total: chunk_pair.map(|x| x.1),
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

#[derive(Clone, Copy)]
enum StepLabel<'a> {
    SingleDocument,
    EvidenceExtract { idx: usize, total: usize },
    ThematicMatrix,
    SynthesisSection { idx: usize, total: usize, title: &'a str },
    DecisionsAppendix,
    CleanupChunk { idx: usize, total: usize },
}

impl StepLabel<'_> {
    fn chunk_pair(&self) -> Option<(usize, usize)> {
        match *self {
            StepLabel::EvidenceExtract { idx, total }
            | StepLabel::SynthesisSection { idx, total, .. }
            | StepLabel::CleanupChunk { idx, total } => Some((idx, total)),
            _ => None,
        }
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
            deep_evidence: None,
            model_id: Some("e2b".to_string()),
            sampling: None,
        }
    }

    #[test]
    fn kv_friendly_prompt_shares_exact_transcript_prefix_across_all_settings() {
        let base_req = sample_req(
            "meeting-notes",
            "Minnesanteckning",
            &format!("{COMMON_PREAMBLE}\n\nSkapa en kronologisk minnesanteckning."),
        );
        let p_notes = build_kv_friendly_prompt(&base_req, false);

        // 1. Byte av mall till Åtgärdspunkter
        let p_actions = build_kv_friendly_prompt(
            &sample_req(
                "action-items",
                "Åtgärdspunkter",
                &format!("{COMMON_PREAMBLE}\n\nExtrahera enbart konkreta åtgärder."),
            ),
            false,
        );

        // 2. Byte till Rätta transkriberingen (CLEANUP_PROMPT)
        let p_cleanup = build_kv_friendly_prompt(
            &sample_req(
                "transcription-cleanup",
                "Rätta transkriberingen",
                CLEANUP_PROMPT,
            ),
            false,
        );

        // 3. Ändring av Kreativare språk, Sammanhang och Agenda
        let mut creative_req = base_req.clone();
        creative_req.creative = true;
        creative_req.context = Some("Sjukhusledning i Region Jönköpings län".to_string());
        creative_req.agenda = Some("1. Budget\n2. Bemanning".to_string());
        let p_creative_agenda = build_kv_friendly_prompt(&creative_req, false);

        let mut no_ctx_req = base_req.clone();
        no_ctx_req.context = None;
        let p_no_ctx = build_kv_friendly_prompt(&no_ctx_req, false);

        let expected_prefix = build_transcript_prefix(&base_req.transcript, false);

        for (name, p) in [
            ("notes", &p_notes),
            ("actions", &p_actions),
            ("cleanup", &p_cleanup),
            ("creative+context+agenda", &p_creative_agenda),
            ("no_context", &p_no_ctx),
        ] {
            assert!(
                p.starts_with(&expected_prefix),
                "Variant '{name}' delar inte det exakta transkript-prefixet!"
            );
            assert!(p.ends_with("<|turn>model\n"));
            assert!(!p.contains("<|think|>"));
        }

        let mut thorough_req = base_req;
        thorough_req.thorough = true;
        let p_thorough = build_kv_friendly_prompt(&thorough_req, false);
        assert!(p_thorough.starts_with("<bos><|turn>system\n<|think|>"));
        assert!(p_thorough.starts_with(&build_transcript_prefix(&thorough_req.transcript, true)));
    }

    #[test]
    fn safe_ctx_ceiling_scales_with_ram_and_respects_bounds() {
        assert_eq!(safe_ctx_ceiling(8.0, MODEL_E2B.expected_bytes), MAX_CTX);
        assert_eq!(safe_ctx_ceiling(8.0, MODEL_E4B.expected_bytes), 90_112);
        assert_eq!(safe_ctx_ceiling(16.0, MODEL_E4B.expected_bytes), MAX_CTX);
        assert_eq!(safe_ctx_ceiling(4.0, MODEL_E4B.expected_bytes), 8192);
    }

    #[test]
    fn splits_long_text_cleanly_at_turn_and_sentence_boundaries() {
        let sentence = "Anna: Detta är ett talarinlägg om projektets tidsplan och leveranser.\n";
        let long_text = sentence.repeat(80);
        let chunks = crate::evidence::split_transcript_on_turns(&long_text, 300, 3.5, 30);
        assert!(chunks.len() >= 4);
        assert!(chunks[0].ends_with('.'));
    }

    #[test]
    #[ignore]
    fn real_gemma4_end_to_end_smoke_test() {
        let gguf = std::env::var("LOKI_SMOKE_GGUF").unwrap_or_else(|_| {
            "/Users/patriksvensk/Library/Application Support/com.loki.app/models/gemma-4-E4B-it-qat-UD-Q4_K_XL.gguf".to_string()
        });
        let p = Path::new(&gguf);
        assert!(p.exists(), "GGUF saknas på {}", p.display());

        println!("[SMOKE] Laddar SummarizerEngine från {}...", p.display());
        let mut eng = SummarizerEngine::load(&MODEL_E4B, p).expect("SummarizerEngine::load");
        println!(
            "[SMOKE] Modell laddad! gpu={}, kv_q8={}, current_n_ctx={}, safe_ceiling={}",
            eng.gpu_active, eng.kv_q8_active, eng.current_n_ctx, eng.safe_ceiling
        );
        assert!(eng.gpu_active, "Förväntade Metal GPU aktiv!");

        // Bygg ett realistiskt svenskt mötesunderlag på ~7 150 tokens
        let turn_block = "\
Anna Lindqvist: Hej alla, kul att ni kunde koppla upp! Hörs mikrofonen bra? Ja vad fint. Då kör vi igång veckans styrgruppsmöte för e-tjänsteplattformen.\n\
Erik Sjöberg: Vi har testat den nya inloggningen via Freja och BankID i testmiljön under veckan. Svarstiden ligger på 180 millisekunder och inga avbrott har noterats.\n\
Maria Holm: Vad gäller informationssäkerhetsklassningen så landade juristen och CISO i att personuppgifterna måste gallras automatiskt efter 90 dagar. Vi behöver fatta beslut om det idag.\n\
Anna Lindqvist: Bra, då beslutar vi att införa automatisk gallring efter 90 dagar i produktionsmiljön senast den 15 december. Erik, tar du ansvar för att lägga in gallringsjobbet i sprint 14?\n\
Erik Sjöberg: Absolut, jag lägger in det i sprint 14 och återkopplar på fredag.\n\n";
        let mut long_transcript = String::new();
        while eng.count_tokens(&long_transcript) < 7150 {
            long_transcript.push_str(turn_block);
        }
        let tok_count = eng.count_tokens(&long_transcript);
        println!("[SMOKE] Transkript skapat: {} exakta Gemma 4-tokens", tok_count);

        // Verifiera tyst bakgrundsförvärmning!
        let cancel = AtomicBool::new(false);
        let t_warm = Instant::now();
        let warmed = eng
            .prewarm_transcript(&long_transcript, false, &cancel)
            .expect("prewarm_transcript");
        println!(
            "[SMOKE-PREWARM] Förvärmde {} tokens i KV-cachen på {} ms!",
            warmed,
            t_warm.elapsed().as_millis()
        );
        assert!(warmed > 7000, "Förväntade >7000 förvärmda tokens, fick {warmed}");

        let mut req1 = sample_req(
            "meeting-notes",
            "Minnesanteckning",
            &format!("{COMMON_PREAMBLE}\n\nSkapa en kort och tydlig minnesanteckning med Beslut och Åtgärder."),
        );
        req1.transcript = long_transcript.clone();

        let res1 = eng
            .run_summarize(
                &req1,
                None,
                &cancel,
                |prog| {
                    if prog.phase == "prefill" {
                        println!(
                            "[SMOKE-1] {} ({}% / kv_reused={} / prompt={})",
                            prog.label, prog.prefill_percent, prog.kv_reused_tokens, prog.prompt_tokens
                        );
                    }
                },
                |_chunk| {},
            )
            .expect("run_summarize pass 1");

        println!(
            "[SMOKE-1] KLAR på {} ms ({:.1} tok/s, kv_reused={}, gen_tokens={}):\n---\n{}\n---",
            res1.duration_ms, res1.tokens_per_sec, res1.kv_reused_tokens, res1.generated_tokens, res1.markdown
        );
        assert!(!res1.markdown.trim().is_empty(), "Pass 1 gav tomt svar!");
        assert!(
            res1.kv_reused_tokens >= warmed,
            "Pass 1 skulle återanvända förvärmda tokens ({warmed}), men fick {}",
            res1.kv_reused_tokens
        );

        // Pass 2: Ändra BÅDE mall, kreativt läge OCH sammanhang -> verifiera 99,9 % varm KV-cache!
        let mut req2 = sample_req(
            "action-items",
            "Åtgärdspunkter",
            &format!("{COMMON_PREAMBLE}\n\nLista enbart konkreta åtgärdspunkter med ansvarig och deadline."),
        );
        req2.transcript = long_transcript;
        req2.creative = true;
        req2.context = Some("Digitaliseringsenheten och informationssäkerhet".to_string());

        let res2 = eng
            .run_summarize(
                &req2,
                None,
                &cancel,
                |prog| {
                    if prog.phase == "prefill" {
                        println!(
                            "[SMOKE-2] {} (kv_reused={})",
                            prog.label, prog.kv_reused_tokens
                        );
                    }
                },
                |_chunk| {},
            )
            .expect("run_summarize pass 2 (warm KV)");

        println!(
            "[SMOKE-2] KLAR på {} ms ({:.1} tok/s, kv_reused={}, gen_tokens={}):\n---\n{}\n---",
            res2.duration_ms, res2.tokens_per_sec, res2.kv_reused_tokens, res2.generated_tokens, res2.markdown
        );
        assert!(!res2.markdown.trim().is_empty(), "Pass 2 gav tomt svar!");
        assert!(
            res2.kv_reused_tokens > 7000,
            "Förväntade återanvändning av >7000 tokens trots ändrat kreativt läge + sammanhang, fick {}",
            res2.kv_reused_tokens
        );
    }

    #[test]
    #[ignore]
    fn real_disk_evidence_pipeline_smoke_test() {
        let e2b_path = std::path::PathBuf::from(
            "/Users/patriksvensk/Library/Application Support/com.loki.app/models/gemma-4-E2B-it-qat-UD-Q4_K_XL.gguf",
        );
        let e4b_path = std::path::PathBuf::from(
            "/Users/patriksvensk/Library/Application Support/com.loki.app/models/gemma-4-E4B-it-qat-UD-Q4_K_XL.gguf",
        );
        let (spec, p) = if e2b_path.exists() {
            (&MODEL_E2B, e2b_path)
        } else {
            (&MODEL_E4B, e4b_path)
        };
        assert!(p.exists(), "GGUF saknas på {}", p.display());

        let mut eng = SummarizerEngine::load(spec, &p).expect("load Gemma 4");
        let tmp_cache = std::env::temp_dir().join("loki_smoke_evidence_disk");
        crate::evidence::cleanup_all_evidence_cache(&tmp_cache);

        let part_a = "\
Anna Lindqvist: Välkomna till projektmötet om e-tjänsteplattformen. Första punkten är logglagring och informationssäkerhet.\n\
Maria Holm: Juristen och CISO har slagit fast att personuppgifter i felsökningsloggar måste gallras automatiskt efter 90 dagar.\n\
Johan Berg (Kundtjänst): Jag måste invända här! Rensar vi efter 90 dagar står kundtjänst helt utan spårbarhet när kvartalsfakturorna överklagas efter tre månader.\n\
Anna Lindqvist: Vi noterar Johans invändning som en risk att bevaka, men vi beslutar ändå att införa 90 dagars gallring senast 15 december, tillsammans med en anonymiserad fakturalogg.\n\
Erik Sjöberg: Jag tar på mig att bygga gallringsjobbet och den anonymiserade fakturaloggen i sprint 14 till på fredag.\n\n";

        let part_b = "\
Anna Lindqvist: Nu går vi vidare till nästa fråga som är budget för licenser under Q1 och införande av kortkort — förlåt, SITHS-kort — för administratörerna.\n\
Sara Ek: Kostnaden för SITHS-integrationen landar på 240 000 kronor. Henrik menade dock att vi borde avvakta till Q2 på grund av stram driftbudget.\n\
Anna Lindqvist: Vi beslutar att godkänna de 240 000 kronorna ur reservpotten redan i Q1 eftersom säkerhetskravet inte kan vänta. Sara beställer certifikaten senast den 20 november.\n\n";

        let mut multi_chunk_transcript = String::new();
        while eng.count_tokens(&multi_chunk_transcript) < 5200 {
            multi_chunk_transcript.push_str(part_a);
            multi_chunk_transcript.push_str(part_b);
        }

        let mut req_deep = sample_req(
            "thematic-workshop",
            "Tematisk djupanalys",
            "Skapa en tematisk djupanalys med teman, oenigheter/friktionspunkter, risker, citat samt beslut och åtgärder.",
        );
        req_deep.transcript = multi_chunk_transcript.clone();
        req_deep.deep_evidence = Some(true);

        let cancel = AtomicBool::new(false);
        let res1 = eng
            .run_summarize(
                &req_deep,
                Some(&tmp_cache),
                &cancel,
                |prog| {
                    println!("[EVIDENCE-1] [{}] {}", prog.phase, prog.label);
                },
                |_c| {},
            )
            .expect("evidence run 1");

        println!(
            "[EVIDENCE-1] KLAR på {} ms (strategi={}):\n---\n{}\n---",
            res1.duration_ms, res1.strategy, res1.markdown
        );
        assert_eq!(res1.strategy, "evidence");
        assert!(res1.markdown.contains("90") || res1.markdown.contains("240"));

        // Kör nu direkt ett mallbyte till Beslutslogg -> ska återanvända 100 % av bevispaketen från disk på 0 ms i Pass 1!
        let mut req_decisions = sample_req(
            "decision-log",
            "Beslutslogg",
            "Lista alla beslut som fattas i kronologisk ordning med motivering.",
        );
        req_decisions.transcript = multi_chunk_transcript;
        req_decisions.deep_evidence = Some(true);

        let t_switch = Instant::now();
        let res2 = eng
            .run_summarize(
                &req_decisions,
                Some(&tmp_cache),
                &cancel,
                |prog| {
                    println!("[EVIDENCE-2-DISK-HIT] [{}] {}", prog.phase, prog.label);
                },
                |_c| {},
            )
            .expect("evidence run 2 from disk cache");
        let switch_ms = t_switch.elapsed().as_millis();
        println!(
            "[EVIDENCE-2-DISK-HIT] Mallbyte från sparade bevispaket klart på {} ms (kv_reused={}):\n---\n{}\n---",
            switch_ms, res2.kv_reused_tokens, res2.markdown
        );
        assert!(res2.kv_reused_tokens >= 3400, "Förväntade träff i disk-cachen!");
        assert!(!res2.markdown.trim().is_empty());

        crate::evidence::cleanup_all_evidence_cache(&tmp_cache);
    }
}
