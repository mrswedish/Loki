/// Manages the llama-server subprocess lifecycle.
use std::path::PathBuf;
use std::net::TcpListener;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub struct InferenceEngine {
	server_binary: Option<PathBuf>,
	server_process: Option<Child>,
	pub port: Option<u16>,
	model_path: Option<String>,
}

// Safety: Child is owned and only accessed through Mutex
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
			eprintln!("Killing llama-server process...");
			let _ = child.kill();
			let _ = child.wait();
			eprintln!("llama-server process killed.");
		}
		self.port = None;
		self.model_path = None;
	}

	pub fn start(&mut self, path: &str, ctx_size: Option<u32>, gpu_index: Option<i32>) -> Result<u16, String> {
		// Don't restart if same model already loaded and server alive
		if self.model_path.as_deref() == Some(path) && self.server_is_alive() {
			return Ok(self.port.unwrap());
		}

		self.kill_server();

		let binary = self
			.server_binary
			.as_ref()
			.ok_or("llama-server binary saknas")?
			.clone();

		if !binary.exists() {
			return Err(format!("llama-server binary hittades inte: {}", binary.display()));
		}

		let port = free_port()?;

		let mut cmd = Command::new(&binary);
		cmd.arg("--model").arg(path);
		cmd.arg("--port").arg(port.to_string());
		cmd.arg("--host").arg("127.0.0.1");
		let ctx = ctx_size.unwrap_or(8192);
		cmd.arg("--ctx-size").arg(ctx.to_string());
		cmd.arg("--n-gpu-layers").arg("99");
		cmd.arg("--jinja");

		#[cfg(target_os = "windows")]
		{
			let index = gpu_index.unwrap_or(-1);
			if index >= 0 {
				let index_str = index.to_string();
				// For Vulkan:
				cmd.env("GGML_VULKAN_DEVICE", &index_str);
				// For CUDA:
				cmd.env("CUDA_VISIBLE_DEVICES", &index_str);
			}
		}
		
		// Pipe output in debug mode so we can see what's happening
		#[cfg(debug_assertions)]
		{
			cmd.stdout(Stdio::inherit());
			cmd.stderr(Stdio::inherit());
		}
		#[cfg(not(debug_assertions))]
		{
			cmd.stdout(Stdio::null());
			cmd.stderr(Stdio::null());
		}

		eprintln!("Starting llama-server: {} --model {} --port {}", binary.display(), path, port);

		#[cfg(target_os = "windows")]
		{
			use std::os::windows::process::CommandExt;
			const CREATE_NO_WINDOW: u32 = 0x0800_0000;
			cmd.creation_flags(CREATE_NO_WINDOW);
		}

		let child = cmd.spawn()
			.map_err(|e| format!("Kunde inte starta llama-server: {}", e))?;

		self.server_process = Some(child);
		self.port = Some(port);
		self.model_path = Some(path.to_string());

		wait_for_server(port, Duration::from_secs(120))?;

		Ok(port)
	}

	pub fn stop(&mut self) {
		self.kill_server();
		self.model_path = None;
	}

	pub fn server_url(&self) -> Option<String> {
		self.port.map(|p| format!("http://127.0.0.1:{}", p))
	}


	fn server_is_alive(&self) -> bool {
		let Some(port) = self.port else { return false; };
		std::net::TcpStream::connect_timeout(
			&format!("127.0.0.1:{}", port).parse().unwrap(),
			Duration::from_millis(200),
		).is_ok()
	}
}

fn free_port() -> Result<u16, String> {
	TcpListener::bind("127.0.0.1:0")
		.map(|l| l.local_addr().unwrap().port())
		.map_err(|e| format!("Kunde inte hitta ledig port: {}", e))
}

fn wait_for_server(port: u16, timeout: Duration) -> Result<(), String> {
	let addr: std::net::SocketAddr = format!("127.0.0.1:{}", port).parse().unwrap();
	let deadline = Instant::now() + timeout;

	loop {
		if Instant::now() > deadline {
			return Err(format!("llama-server startade inte inom {}s", timeout.as_secs()));
		}
		if std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(500)).is_ok() {
			break;
		}
		std::thread::sleep(Duration::from_millis(500));
	}

	let url = format!("http://127.0.0.1:{}/health", port);
	let client = reqwest::blocking::Client::builder()
		.timeout(Duration::from_secs(2))
		.build()
		.map_err(|e| format!("HTTP client fel: {}", e))?;

	loop {
		if Instant::now() > deadline {
			return Err("llama-server /health svarade inte i tid".to_string());
		}
		if let Ok(r) = client.get(&url).send() {
			if r.status().is_success() {
				return Ok(());
			}
		}
		std::thread::sleep(Duration::from_millis(1000));
	}
}

pub type SharedEngine = Arc<Mutex<InferenceEngine>>;

pub fn create_engine() -> SharedEngine {
	Arc::new(Mutex::new(InferenceEngine::new()))
}
