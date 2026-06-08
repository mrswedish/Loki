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
	// Stored for auto-restart
	ctx_size: Option<u32>,
	gpu_index: Option<i32>,
	log_dir: Option<PathBuf>,
	/// True om senaste start föll tillbaka till CPU efter att GPU misslyckats.
	fell_back_to_cpu: bool,
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
			ctx_size: None,
			gpu_index: None,
			log_dir: None,
			fell_back_to_cpu: false,
		}
	}

	/// Returnerar true om senaste lyckade start föll tillbaka till CPU (GPU misslyckades).
	pub fn fell_back_to_cpu(&self) -> bool {
		self.fell_back_to_cpu
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

	pub fn start(&mut self, path: &str, ctx_size: Option<u32>, gpu_index: Option<i32>, log_dir: Option<PathBuf>) -> Result<u16, String> {
		// Don't restart if same model already loaded and server alive – men bara om
		// ctx_size och gpu_index är oförändrade. Auto-expand anropar start() med samma
		// modell men STÖRRE ctx_size; då MÅSTE servern startas om, annars körs den vidare
		// med den gamla (för lilla) kontexten och overflow-felet kvarstår.
		if self.model_path.as_deref() == Some(path)
			&& self.ctx_size == ctx_size
			&& self.gpu_index == gpu_index
			&& self.server_is_alive()
		{
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

		let ctx = ctx_size.unwrap_or(4096);

		// CPU-only build: skip GPU entirely
		#[cfg(feature = "cpu-only")]
		let port = {
			self.fell_back_to_cpu = false;
			self.try_spawn(&binary, path, ctx, gpu_index, log_dir.clone(), "0", true, Duration::from_secs(300))?
		};

		// GPU build: try Vulkan first, fall back to a *real* CPU run on failure
		// (OOM, missing Vulkan extension, crashing driver). Den första körningen kör
		// GPU med --n-gpu-layers auto: llama.cpp mäter tillgängligt VRAM och lägger bara
		// så många lager som ryms (partiell offload), resten på CPU. Det undviker att en
		// stor modell (t.ex. 12B) spränger en svag iGPU genom att tvinga ALLA lager dit.
		// Faller GPU-starten ändå kör vi om med force_cpu=true som tvingar bort Vulkan helt
		// (--device none, -fit off, tömd Vulkan-env) så device-init aldrig sker.
		#[cfg(not(feature = "cpu-only"))]
		let port = match self.try_spawn(&binary, path, ctx, gpu_index, log_dir.clone(), "auto", false, Duration::from_secs(60)) {
			Ok(p) => {
				self.fell_back_to_cpu = false;
				p
			}
			Err(e) => {
				eprintln!("[InferenceEngine] GPU misslyckades ({}), startar om på CPU (--device none)...", e);
				let p = self.try_spawn(&binary, path, ctx, gpu_index, log_dir.clone(), "0", true, Duration::from_secs(300))?;
				self.fell_back_to_cpu = true;
				p
			}
		};

		self.port = Some(port);
		self.model_path = Some(path.to_string());
		self.ctx_size = ctx_size;
		self.gpu_index = gpu_index;
		self.log_dir = log_dir;

		Ok(port)
	}

	/// Spawns llama-server with the given gpu_layers value and waits for it to become ready.
	/// Kills the child and returns Err if the process dies or the timeout expires.
	fn try_spawn(
		&mut self,
		binary: &PathBuf,
		path: &str,
		ctx: u32,
		gpu_index: Option<i32>,
		log_dir: Option<PathBuf>,
		gpu_layers: &str,
		force_cpu: bool,
		timeout: Duration,
	) -> Result<u16, String> {
		let port = free_port()?;

		let mut cmd = Command::new(binary);
		cmd.arg("--model").arg(path);
		cmd.arg("--port").arg(port.to_string());
		cmd.arg("--host").arg("127.0.0.1");
		cmd.arg("--ctx-size").arg(ctx.to_string());
		// Desktop-app: en chatt i taget. Utan detta väljer llama.cpp n_parallel=4 (auto)
		// och reserverar KV-cache för slots vi aldrig använder – hela ctx ska gå till den
		// enda förfrågan.
		cmd.arg("--parallel").arg("1");
		cmd.arg("--n-gpu-layers").arg(gpu_layers);
		cmd.arg("--jinja");
		// Flash Attention i auto-läge: llama.cpp aktiverar FA där hårdvaran stödjer det
		// (CPU, NVIDIA/coopmat2) och faller annars säkert tillbaka. Aldrig tvinga 'on' –
		// på Vulkan/AMD ger det CPU-offload och kraftig prestandaförlust.
		cmd.arg("--flash-attn").arg("auto");
		// Vi har en egen frontend – stäng av llama-serverns inbyggda WebUI.
		cmd.arg("--no-webui");
		// Aldrig kasta bort äldsta tokens när kontexten fylls. Context-shift är till
		// för oändlig chatt, men för sammanfattning av stora transkript skulle det
		// tyst trunkera början/mitten → ofullständig sammanfattning. Loki utökar
		// istället kontexten dynamiskt (auto-expansion i chat-storen).
		cmd.arg("--no-context-shift");

		if force_cpu {
			// Äkta CPU-körning: hoppa över all GPU/Vulkan-init. Enbart --n-gpu-layers 0
			// räcker INTE i Vulkan-bygget – `-fit on` (default) gör device-anrop för att
			// mäta minne, vilket kraschar på trasiga/inkompatibla drivrutiner
			// (t.ex. AMD iGPU: ErrorExtensionNotPresent). --device none + -fit off tar
			// bort alla device-anrop. Tömd Vulkan-env är bälte-och-hängslen på Windows.
			cmd.arg("--device").arg("none");
			cmd.arg("-fit").arg("off");
			cmd.env("GGML_VK_VISIBLE_DEVICES", "");
			cmd.env("GGML_VULKAN_DEVICE", "");
		} else {
			#[cfg(all(target_os = "windows", not(feature = "cpu-only")))]
			{
				let index = gpu_index.unwrap_or(-1);
				if index >= 0 {
					let index_str = index.to_string();
					// Sätt env för båda backends – binären använder den som gäller och
					// ignorerar den andra. Vulkan-bygget läser GGML_VK_*, CUDA-bygget
					// läser CUDA_VISIBLE_DEVICES.
					cmd.env("GGML_VK_VISIBLE_DEVICES", &index_str);
					cmd.env("GGML_VULKAN_DEVICE", &index_str);
					cmd.env("CUDA_VISIBLE_DEVICES", &index_str);
				}
			}
		}

		if let Some(ref dir) = log_dir {
			let log_path = dir.join("llama_server.log");
			if let Ok(file) = std::fs::File::create(log_path) {
				cmd.stdout(Stdio::from(file.try_clone().unwrap()));
				cmd.stderr(Stdio::from(file));
			}
		} else {
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
		}

		#[cfg(target_os = "windows")]
		{
			use std::os::windows::process::CommandExt;
			const CREATE_NO_WINDOW: u32 = 0x0800_0000;
			cmd.creation_flags(CREATE_NO_WINDOW);
		}

		eprintln!(
			"Starting llama-server: {} --model {} --port {} --n-gpu-layers {}",
			binary.display(), path, port, gpu_layers
		);

		let mut child = cmd
			.spawn()
			.map_err(|e| format!("Kunde inte starta llama-server: {}", e))?;

		match wait_for_server(port, timeout, log_dir, &mut child) {
			Ok(()) => {
				self.server_process = Some(child);
				Ok(port)
			}
			Err(e) => {
				let _ = child.kill();
				let _ = child.wait();
				Err(e)
			}
		}
	}

	/// Restarts the server if it is no longer alive, using the last-known parameters.
	/// Returns `Ok(Some(new_url))` if restarted, `Ok(None)` if still alive or nothing to restart.
	pub fn restart_if_dead(&mut self) -> Result<Option<String>, String> {
		if self.server_is_alive() {
			return Ok(None);
		}
		let Some(model_path) = self.model_path.clone() else {
			return Ok(None); // No model was loaded – nothing to restart
		};
		eprintln!("[InferenceEngine] Server dead, restarting with model: {}", model_path);
		let log_dir = self.log_dir.clone();
		let port = self.start(&model_path, self.ctx_size, self.gpu_index, log_dir)?;
		eprintln!("[InferenceEngine] Server restarted on port {}", port);
		Ok(Some(format!("http://127.0.0.1:{}", port)))
	}

	pub fn stop(&mut self) {
		self.kill_server();
		self.model_path = None;
	}

	pub fn server_url(&self) -> Option<String> {
		self.port.map(|p| format!("http://127.0.0.1:{}", p))
	}

	pub fn server_is_alive(&self) -> bool {
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

/// Waits for llama-server to become ready on `port`.
/// Returns immediately with Err if the child process dies before the server is up.
fn wait_for_server(port: u16, timeout: Duration, log_dir: Option<PathBuf>, child: &mut Child) -> Result<(), String> {
	let addr: std::net::SocketAddr = format!("127.0.0.1:{}", port).parse().unwrap();
	let deadline = Instant::now() + timeout;

	// Phase 1: wait for TCP port to open
	loop {
		if Instant::now() > deadline {
			let log_msg = log_dir.as_ref()
				.map(|d| format!(" Se logg: {}", d.join("llama_server.log").display()))
				.unwrap_or_default();
			return Err(format!("llama-server startade inte inom {}s.{}", timeout.as_secs(), log_msg));
		}
		// Detect early exit (e.g. GPU OOM, missing DLLs)
		if let Ok(Some(status)) = child.try_wait() {
			return Err(format!("llama-server avslutades direkt (exit: {:?})", status));
		}
		if std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(500)).is_ok() {
			break;
		}
		std::thread::sleep(Duration::from_millis(500));
	}

	// Phase 2: wait for /health to return 200
	let url = format!("http://127.0.0.1:{}/health", port);
	let client = reqwest::blocking::Client::builder()
		.timeout(Duration::from_secs(2))
		.build()
		.map_err(|e| format!("HTTP client fel: {}", e))?;

	loop {
		if Instant::now() > deadline {
			let log_msg = log_dir.as_ref()
				.map(|d| format!(" Se logg: {}", d.join("llama_server.log").display()))
				.unwrap_or_default();
			return Err(format!("llama-server /health svarade inte i tid.{}", log_msg));
		}
		if let Ok(Some(status)) = child.try_wait() {
			return Err(format!("llama-server dog under hälsokontroll (exit: {:?})", status));
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
