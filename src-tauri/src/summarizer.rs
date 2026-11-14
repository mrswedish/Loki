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
/// Tak för enkelpass på ren CPU utan GPU-offload (undviker kvadratisk 30k-sirap på kontors-PC utan GPU).
const CPU_SINGLE_PASS_MAX_CTX: u32 = 16_384;
/// Batchstorlek vid prompt-inläsning på GPU (Metal/Vulkan) för maximal matrisgenomströmning.
const GPU_PREFILL_BATCH: usize = 2048;
const GPU_PREFILL_UBATCH: u32 = 2048;
/// Batchstorlek på ren CPU (responsivt avbrott och lägre cache-tryck).
const CPU_PREFILL_BATCH: usize = 512;
const CPU_PREFILL_UBATCH: u32 = 512;
/// Stegstorlek vid tyst bakgrundsförvärmning på GPU (hög matrisgenomströmning men lämnar över låset snabbt vid klick på "Skapa").
const PREWARM_BATCH: usize = 1024;
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

    let think_prefix = if req.thorough { "<|think|>" } else { "" };

    format!(
        "<bos><|turn>system\n{think_prefix}{REDUCE_PROMPT}<turn|>\n<|turn>user\n--- DELSAMMANFATTNINGAR I KRONOLOGISK ORDNING ---\n{joined_parts}\n--- SLUT PÅ DELSAMMANFATTNINGAR ---\n\n{task_tail}<turn|>\n<|turn>model\n"
    )
}

pub fn build_adjust_prompt(current_markdown: &str, directive: &str) -> String {
    format!(
        "<bos><|turn>system\n{ADJUST_PROMPT}<turn|>\n<|turn>user\n--- NUVARANDE MÖTESDOKUMENT ---\n{}\n--- SLUT PÅ DOKUMENT ---\n\n--- JUSTERINGSDIREKTIV FRÅN ANVÄNDAREN ---\n{}\n--- SLUT PÅ DIREKTIV ---<turn|>\n<|turn>model\n",
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
    pub gpu_active: bool,
    pub kv_q8_active: bool,
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
                let mut mparams = LlamaModelParams::default().with_use_mmap(true);
                if use_gpu {
                    crate::logger::log_info("Aktiverar GPU-offload (n_gpu_layers=999)");
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
                let base_init = if use_gpu && total_ram_gb >= 22.0 {
                    HIGH_RAM_INIT_CTX
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
        let min_floor = DEFAULT_MIN_CTX.min(self.safe_ceiling);
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
            Err(err) if grown_target > DEFAULT_MIN_CTX => {
                crate::logger::log_warn(&format!(
                    "Kunde inte växa kontextfönstret till {grown_target} tokens ({err}) — begränsar till {DEFAULT_MIN_CTX} och använder delsammanfattning."
                ));
                let fallback = DEFAULT_MIN_CTX.min(self.safe_ceiling);
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
        crate::logger::log_info(&format!(
            "run_summarize: mall='{}', prompt_tokens={}, needed_total={}, single_ceiling={}, safe_ceiling={}, gpu={}, kv_q8={}, thorough={}, creative={}",
            req.template_id,
            prompt_tokens.len(),
            needed_total,
            effective_single_ceiling,
            self.safe_ceiling,
            self.gpu_active,
            self.kv_q8_active,
            req.thorough,
            req.creative
        ));

        if needed_total <= effective_single_ceiling {
            self.ensure_context_capacity(needed_total)?;
            if needed_total <= self.current_n_ctx {
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
        }

        // Map-Reduce-fallback för möten som överskrider kontexttaket (eller >16K på ren CPU utan GPU)
        let chunk_ctx = if self.gpu_active {
            self.safe_ceiling.min(32_768)
        } else {
            self.safe_ceiling.min(CPU_SINGLE_PASS_MAX_CTX)
        };
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
            MAX_THOUGHT_TOKENS + MAX_ANSWER_TOKENS
        } else {
            MAX_ANSWER_TOKENS + 16
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
            } else if Some(next_token) != channel_end_tok {
                answer_tokens.push(next_token);

                if answer_tokens.len() >= MAX_ANSWER_TOKENS {
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
    fn splits_long_text_cleanly_at_sentence_boundaries() {
        let sentence = "Detta är en mening om projektets tidsplan och leveranser. ";
        let long_text = sentence.repeat(80);
        let chunks = split_into_chunks(&long_text, 300, 3.5, 30);
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
}
