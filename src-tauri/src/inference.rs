use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{AddBos, LlamaModel};
use llama_cpp_2::sampling::LlamaSampler;
use std::num::NonZeroU32;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter};

#[derive(Clone, Serialize)]
pub struct ChatToken {
    pub session_id: String,
    pub token: String,
    pub done: bool,
}

use serde::Serialize;

pub struct InferenceEngine {
    backend: LlamaBackend,
    model: Option<LlamaModel>,
    model_path: Option<String>,
}

// Safety: LlamaBackend and LlamaModel implement Send
unsafe impl Send for InferenceEngine {}

impl InferenceEngine {
    pub fn new() -> Result<Self, String> {
        let backend = LlamaBackend::init().map_err(|e| format!("Backend init failed: {}", e))?;
        Ok(Self {
            backend,
            model: None,
            model_path: None,
        })
    }

    pub fn load_model(&mut self, path: &str) -> Result<(), String> {
        // Don't reload if same model
        if self.model_path.as_deref() == Some(path) && self.model.is_some() {
            return Ok(());
        }

        let params = LlamaModelParams::default();

        let model = LlamaModel::load_from_file(&self.backend, Path::new(path), &params)
            .map_err(|e| format!("Kunde inte ladda modell: {:?}", e))?;

        self.model = Some(model);
        self.model_path = Some(path.to_string());
        Ok(())
    }

    pub fn is_loaded(&self) -> bool {
        self.model.is_some()
    }

    pub fn generate(
        &self,
        prompt: &str,
        max_tokens: u32,
        app: &AppHandle,
        session_id: &str,
    ) -> Result<String, String> {
        let model = self
            .model
            .as_ref()
            .ok_or("Ingen modell laddad")?;

        let ctx_params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(4096));

        let mut ctx = model
            .new_context(&self.backend, ctx_params)
            .map_err(|e| format!("Kontext-fel: {:?}", e))?;

        // Tokenize input
        let tokens = model
            .str_to_token(prompt, AddBos::Always)
            .map_err(|e| format!("Tokeniseringsfel: {:?}", e))?;

        // Create batch with prompt tokens
        let mut batch = LlamaBatch::new(4096, 1);
        for (i, token) in tokens.iter().enumerate() {
            let is_last = i == tokens.len() - 1;
            batch.add(*token, i as i32, &[0], is_last)
                .map_err(|e| format!("Batch-fel: {:?}", e))?;
        }

        // Evaluate prompt
        ctx.decode(&mut batch)
            .map_err(|e| format!("Avkodningsfel: {:?}", e))?;

        // Setup sampler
        let mut sampler = LlamaSampler::chain_simple([
            LlamaSampler::temp(0.7),
            LlamaSampler::top_p(0.9, 1),
            LlamaSampler::dist(42),
        ]);

        let mut output = String::new();
        let mut n_cur = tokens.len() as i32;

        for _ in 0..max_tokens {
            // Sample next token
            let token = sampler.sample(&ctx, -1);

            // Check for end of generation
            if model.is_eog_token(token) {
                break;
            }

            // Decode to string
            #[allow(deprecated)]
            let piece = model.token_to_str(token, llama_cpp_2::model::Special::Tokenize)
                .unwrap_or_default();

            output.push_str(&piece);

            // Emit streaming token
            let _ = app.emit(
                "chat-token",
                ChatToken {
                    session_id: session_id.to_string(),
                    token: piece.clone(),
                    done: false,
                },
            );

            // Prepare next batch
            batch.clear();
            batch.add(token, n_cur, &[0], true)
                .map_err(|e| format!("Batch-fel: {:?}", e))?;

            ctx.decode(&mut batch)
                .map_err(|e| format!("Avkodningsfel: {:?}", e))?;

            n_cur += 1;
        }

        // Signal done
        let _ = app.emit(
            "chat-token",
            ChatToken {
                session_id: session_id.to_string(),
                token: String::new(),
                done: true,
            },
        );

        Ok(output)
    }
}

/// Thread-safe wrapper
pub type SharedEngine = Arc<Mutex<InferenceEngine>>;

pub fn create_engine() -> Result<SharedEngine, String> {
    let engine = InferenceEngine::new()?;
    Ok(Arc::new(Mutex::new(engine)))
}
