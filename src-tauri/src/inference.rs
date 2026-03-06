use serde::Serialize;
use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

#[derive(Clone, Serialize)]
pub struct ChatToken {
    pub session_id: String,
    pub token: String,
    pub done: bool,
}

pub struct InferenceEngine {
    server_binary: Option<PathBuf>,
    server_process: Option<Child>,
    pub port: Option<u16>,
    model_path: Option<String>,
}

// Safety: process Child is owned by InferenceEngine and only accessed through Mutex
unsafe impl Send for InferenceEngine {}

impl Drop for InferenceEngine {
    fn drop(&mut self) {
        self.kill_server();
    }
}

impl InferenceEngine {
    pub fn new() -> Self {
        Self {
            server_binary: None,
            server_process: None,
            port: None,
            model_path: None,
        }
    }

    pub fn set_server_binary(&mut self, path: PathBuf) {
        self.server_binary = Some(path);
    }

    fn kill_server(&mut self) {
        if let Some(mut child) = self.server_process.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.port = None;
    }

    pub fn load_model(&mut self, path: &str) -> Result<(), String> {
        // Don't restart if same model already loaded
        if self.model_path.as_deref() == Some(path) && self.server_process.is_some() {
            if self.server_is_alive() {
                return Ok(());
            }
        }

        self.kill_server();

        let binary = self
            .server_binary
            .as_ref()
            .ok_or("llama-server binary saknas – ladda ner den först")?
            .clone();

        if !binary.exists() {
            return Err(format!(
                "llama-server binary hittades inte: {}",
                binary.display()
            ));
        }

        let port = free_port()?;

        let mut cmd = Command::new(&binary);
        cmd.arg("--model").arg(path);
        cmd.arg("--port").arg(port.to_string());
        cmd.arg("--host").arg("127.0.0.1");
        cmd.arg("--ctx-size").arg("8192");
        cmd.arg("--n-predict").arg("1024");
        // Log to stderr; suppress stdout noise
        cmd.stdout(Stdio::null());
        cmd.stderr(Stdio::null());

        // On Windows: hide the console window
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let child = cmd
            .spawn()
            .map_err(|e| format!("Kunde inte starta llama-server: {}", e))?;

        self.server_process = Some(child);
        self.port = Some(port);
        self.model_path = Some(path.to_string());

        // Wait for the server to become ready (up to 120 seconds for large models)
        wait_for_server(port, Duration::from_secs(120))?;

        Ok(())
    }

    pub fn is_loaded(&self) -> bool {
        self.model_path.is_some() && self.port.is_some() && self.server_is_alive()
    }

    fn server_is_alive(&self) -> bool {
        let Some(port) = self.port else {
            return false;
        };
        // Quick TCP probe
        std::net::TcpStream::connect_timeout(
            &format!("127.0.0.1:{}", port).parse().unwrap(),
            Duration::from_millis(200),
        )
        .is_ok()
    }

    pub fn generate(
        &self,
        messages: &[serde_json::Value],
        max_tokens: u32,
        app: &AppHandle,
        session_id: &str,
    ) -> Result<String, String> {
        let port = self.port.ok_or("Ingen modell laddad")?;
        let url = format!("http://127.0.0.1:{}/v1/chat/completions", port);

        let body = serde_json::json!({
            "model": "local",
            "messages": messages,
            "max_tokens": max_tokens,
            "temperature": 0.7,
            "stream": true
        });

        // Use blocking HTTP with SSE parsing (runs in spawn_blocking context)
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(300))
            .build()
            .map_err(|e| format!("HTTP client fel: {}", e))?;

        let resp = client
            .post(&url)
            .header("Content-Type", "application/json")
            .body(body.to_string())
            .send()
            .map_err(|e| format!("Inference-anrop misslyckades: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("llama-server svarade HTTP {}", resp.status()));
        }

        let mut full_output = String::new();
        let reader = BufReader::new(resp);

        for line in reader.lines() {
            let line = line.map_err(|e| format!("Läsfel: {}", e))?;
            if line.is_empty() || line == "data: [DONE]" {
                continue;
            }
            let data = line.strip_prefix("data: ").unwrap_or(&line);
            if data.is_empty() {
                continue;
            }

            let Ok(json) = serde_json::from_str::<serde_json::Value>(data) else {
                continue;
            };

            let token_text = json["choices"][0]["delta"]["content"]
                .as_str()
                .unwrap_or("");

            if !token_text.is_empty() {
                full_output.push_str(token_text);
                let _ = app.emit(
                    "chat-token",
                    ChatToken {
                        session_id: session_id.to_string(),
                        token: token_text.to_string(),
                        done: false,
                    },
                );
            }

            // Check for finish reason
            if json["choices"][0]["finish_reason"].is_string() {
                break;
            }
        }

        let _ = app.emit(
            "chat-token",
            ChatToken {
                session_id: session_id.to_string(),
                token: String::new(),
                done: true,
            },
        );

        Ok(full_output)
    }
}

/// Find a free local TCP port by letting the OS assign one.
fn free_port() -> Result<u16, String> {
    TcpListener::bind("127.0.0.1:0")
        .map(|l| l.local_addr().unwrap().port())
        .map_err(|e| format!("Kunde inte hitta ledig port: {}", e))
}

/// Poll until the server accepts TCP connections and responds to /health, or timeout.
fn wait_for_server(port: u16, timeout: Duration) -> Result<(), String> {
    let addr: std::net::SocketAddr = format!("127.0.0.1:{}", port).parse().unwrap();
    let deadline = Instant::now() + timeout;

    // Phase 1: wait for TCP
    loop {
        if Instant::now() > deadline {
            return Err(format!(
                "llama-server startade inte inom {}s",
                timeout.as_secs()
            ));
        }
        if std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(300));
    }

    // Phase 2: wait for /health to return 200
    let url = format!("http://127.0.0.1:{}/health", port);
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .map_err(|e| format!("HTTP client fel: {}", e))?;

    loop {
        if Instant::now() > deadline {
            return Err("llama-server hälsa svarade inte i tid".to_string());
        }
        if let Ok(r) = client.get(&url).send() {
            if r.status().is_success() {
                return Ok(());
            }
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// Thread-safe wrapper
pub type SharedEngine = Arc<Mutex<InferenceEngine>>;

pub fn create_engine() -> SharedEngine {
    Arc::new(Mutex::new(InferenceEngine::new()))
}
