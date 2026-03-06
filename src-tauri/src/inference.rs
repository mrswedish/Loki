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

/// On Windows, llama.cpp's C code calls fopen() which interprets the path as ANSI (Windows-1252),
/// not UTF-8. If the path contains non-ASCII characters (e.g. Swedish letters in the username),
/// fopen() silently fails even though Rust can open the file fine (Rust uses wide Win32 APIs).
///
/// Fix: call GetShortPathNameW to get the 8.3 short form of the path, which is always ASCII-safe.
/// On other platforms, just normalise slashes.
#[cfg(target_os = "windows")]
fn to_llama_safe_path(path: &str) -> String {
    if path.is_ascii() {
        return path.replace('\\', "/");
    }

    // Declare GetShortPathNameW directly – no extra crate needed.
    extern "system" {
        fn GetShortPathNameW(
            lpsz_long_path: *const u16,
            lpsz_short_path: *mut u16,
            cch_buffer: u32,
        ) -> u32;
    }

    use std::os::windows::ffi::OsStrExt;
    let wide: Vec<u16> = std::ffi::OsStr::new(path)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    // First call: get required buffer size.
    let needed = unsafe { GetShortPathNameW(wide.as_ptr(), std::ptr::null_mut(), 0) };
    if needed == 0 {
        return path.replace('\\', "/");
    }

    let mut buf = vec![0u16; needed as usize];
    let len = unsafe { GetShortPathNameW(wide.as_ptr(), buf.as_mut_ptr(), needed) };
    if len == 0 || len >= needed {
        return path.replace('\\', "/");
    }

    String::from_utf16_lossy(&buf[..len as usize]).replace('\\', "/")
}

#[cfg(not(target_os = "windows"))]
fn to_llama_safe_path(path: &str) -> String {
    path.replace('\\', "/")
}

/// Redirects the C runtime's stderr (fd 2) to `capture_path` for the duration of `f`,
/// then restores it. Returns the result of `f` and the captured text.
/// This lets us see what llama.cpp prints when it fails.
#[cfg(target_os = "windows")]
fn with_captured_stderr<F: FnOnce() -> R, R>(capture_path: &std::path::Path, f: F) -> (R, String) {
    extern "C" {
        fn _dup(fd: i32) -> i32;
        fn _dup2(fd1: i32, fd2: i32) -> i32;
        fn _close(fd: i32) -> i32;
        fn _open_osfhandle(osfhandle: isize, flags: i32) -> i32;
        fn _flushall() -> i32;
    }

    let file = match std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(capture_path)
    {
        Ok(f) => f,
        Err(_) => return (f(), String::new()),
    };

    use std::os::windows::io::IntoRawHandle;
    let handle = file.into_raw_handle();
    // _O_WRONLY | _O_TEXT = 0x0001 | 0x4000
    let new_fd = unsafe { _open_osfhandle(handle as isize, 0x4001) };
    if new_fd < 0 {
        return (f(), String::new());
    }

    let saved = unsafe { _dup(2) };
    unsafe {
        _flushall();
        _dup2(new_fd, 2);
        _close(new_fd);
    }

    let result = f();

    unsafe {
        _flushall();
        _dup2(saved, 2);
        _close(saved);
    }

    let captured = std::fs::read_to_string(capture_path).unwrap_or_default();
    (result, captured)
}

#[derive(Clone, Serialize)]
pub struct ChatToken {
    pub session_id: String,
    pub token: String,
    pub done: bool,
}

use serde::Serialize;

pub struct InferenceEngine {
    model_path: Option<String>,
    model: Option<LlamaModel>,
    backend: LlamaBackend,
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

        // Free old model from memory (especially important for Metal GPU backend) before loading the new one
        self.model = None;
        self.model_path = None;

        // On Windows, llama.cpp's mmap (CreateFileMapping/MapViewOfFile) is silently blocked by
        // Windows Defender and other security software scanning the model file, which causes
        // NullResult without any error message. Disable mmap on Windows; use normal file I/O instead.
        #[cfg(target_os = "windows")]
        let mut params = LlamaModelParams::default().with_use_mmap(false);
        #[cfg(not(target_os = "windows"))]
        let mut params = LlamaModelParams::default();

        #[cfg(feature = "vulkan")]
        {
            params = params.with_n_gpu_layers(1000); // Offload allt till GPU
        }
        #[cfg(feature = "metal")]
        {
            params = params.with_n_gpu_layers(1000); // Offload allt till GPU
        }

        let path_obj = Path::new(path);
        
        // --- DEBUG LOGGING FÖR WINDOWS ---
        let desktop_path = dirs::desktop_dir().unwrap_or_else(|| std::path::PathBuf::from("C:\\"));
        let log_file = desktop_path.join("loke_debug_log.txt");
        
        let file_size = std::fs::metadata(&path_obj).map(|m| m.len()).unwrap_or(0);
        let mut log_msgs = vec![
            format!("=== LADDAR MODELL ==="),
            format!("Path_str: {}", path),
            format!("Path exists: {}", path_obj.exists()),
            format!("Path is_file: {}", path_obj.is_file()),
            format!("File size: {} bytes", file_size),
        ];

        // 1. Verify Rust can actually read the file natively (Checks for Windows Defender locks)
        match std::fs::File::open(&path_obj) {
            Ok(mut f) => {
                use std::io::Read;
                let mut magic = [0u8; 4];
                if let Err(e) = f.read_exact(&mut magic) {
                    log_msgs.push(format!("Kunde inte läsa GGUF-hörfilen: {}", e));
                    return Err(format!("Kunde inte läsa GGUF-hörfilen: {}", e));
                }
                log_msgs.push(format!("Magic bytes: {:?}", magic));
                log_msgs.push(format!("Magic bytes som string: {:?}", String::from_utf8_lossy(&magic)));
                // Check for GGUF magic standard
                if &magic != b"GGUF" {
                    log_msgs.push(format!("VARNING: Filen är inte GGUF!"));
                }
            }
            Err(e) => {
                return Err(format!("Rust OS Fel: Filen kan inte öppnas av operativsystemet (förmodligen låst av Windows Defender eller fel sökväg): {:?}", e));
            }
        }

        // 2. Convert path to a form that llama.cpp's C fopen() can handle.
        // See `to_llama_safe_path` above for full explanation.
        let safe_path_str = to_llama_safe_path(path);
        log_msgs.push(format!("Safe path for llama.cpp: {}", safe_path_str));
        let safe_path = Path::new(&safe_path_str);

        // First attempt: with GPU layers.
        // Capture llama.cpp's own stderr so it ends up in the debug log.
        #[cfg(target_os = "windows")]
        let stderr_file = desktop_path.join("loke_llama_stderr.txt");

        #[cfg(target_os = "windows")]
        let (mut model_res, gpu_stderr) = with_captured_stderr(&stderr_file, || {
            LlamaModel::load_from_file(&self.backend, safe_path, &params)
        });
        #[cfg(not(target_os = "windows"))]
        let mut model_res = LlamaModel::load_from_file(&self.backend, safe_path, &params);

        #[cfg(target_os = "windows")]
        log_msgs.push(format!("llama.cpp GPU stderr:\n{}", gpu_stderr.trim()));

        log_msgs.push(format!("GPU res is_err: {}", model_res.is_err()));
        // Fallback: If GPU load fails, retry on CPU
        if let Err(e) = &model_res {
            log_msgs.push(format!("GPU Error: {:?}", e));

            #[cfg(target_os = "windows")]
            let cpu_params = LlamaModelParams::default().with_use_mmap(false).with_n_gpu_layers(0);
            #[cfg(not(target_os = "windows"))]
            let cpu_params = LlamaModelParams::default().with_n_gpu_layers(0);

            #[cfg(target_os = "windows")]
            let (cpu_res, cpu_stderr) = with_captured_stderr(&stderr_file, || {
                LlamaModel::load_from_file(&self.backend, safe_path, &cpu_params)
            });
            #[cfg(not(target_os = "windows"))]
            let cpu_res = LlamaModel::load_from_file(&self.backend, safe_path, &cpu_params);

            #[cfg(target_os = "windows")]
            log_msgs.push(format!("llama.cpp CPU stderr:\n{}", cpu_stderr.trim()));

            if let Err(cpu_e) = &cpu_res {
                log_msgs.push(format!("CPU Error: {:?}", cpu_e));
            } else {
                log_msgs.push(format!("CPU load succeeded!"));
            }
            model_res = cpu_res;
        } else {
            log_msgs.push(format!("GPU load succeeded!"));
        }

            let final_log = log_msgs.join("\n");
            let _ = std::fs::write(&log_file, final_log);

            let model = model_res
                .map_err(|e| format!("Kunde inte ladda modell från '{}': {:?}", path, e))?;

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

        let n_ctx = 8192;
        let ctx_params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(n_ctx));

        let mut ctx = model
            .new_context(&self.backend, ctx_params)
            .map_err(|e| format!("Kontext-fel: {:?}", e))?;

        // Tokenize input
        let tokens = model
            .str_to_token(prompt, AddBos::Always)
            .map_err(|e| format!("Tokeniseringsfel: {:?}", e))?;

        // If the prompt is too long, we truncate it for safety to fit in context window
        let max_prompt_tokens = (n_ctx - max_tokens - 10) as usize;
        let tokens = if tokens.len() > max_prompt_tokens {
            tokens[tokens.len() - max_prompt_tokens..].to_vec()
        } else {
            tokens
        };

        // Create batch with prompt tokens
        // Default n_batch in llama.cpp is often 512 or 2048. We decode in chunks of 512 to be safe for Metal OOM and asserts.
        let mut batch = LlamaBatch::new(512, 1);
        let mut n_past = 0;
        
        for chunk in tokens.chunks(512) {
            batch.clear();
            for (i, token) in chunk.iter().enumerate() {
                let is_last = (n_past + i) == tokens.len() - 1;
                batch.add(*token, (n_past + i) as i32, &[0], is_last)
                    .map_err(|e| format!("Batch-fel: {:?}", e))?;
            }
            
            // Evaluate prompt chunk
            ctx.decode(&mut batch)
                .map_err(|e| format!("Avkodningsfel: {:?}", e))?;
                
            n_past += chunk.len();
        }

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
