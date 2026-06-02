/// Downloads and manages the llama-server binary from llama.cpp GitHub releases.
use std::path::PathBuf;
use tauri::AppHandle;

#[cfg(target_os = "windows")]
const BINARY_NAME: &str = "llama-server.exe";
#[cfg(not(target_os = "windows"))]
const BINARY_NAME: &str = "llama-server";

#[cfg(target_os = "windows")]
const ASSET_EXTENSION: &str = ".zip";
#[cfg(not(target_os = "windows"))]
const ASSET_EXTENSION: &str = ".tar.gz";

/// Känd-god llama.cpp-release som installeras som standard. Att alltid hämta
/// `releases/latest` är en risk: en framtida release kan byta asset-namn eller
/// bryta serverns API och slå ut alla installationer samtidigt. Genom att pinna
/// en verifierad tag får nya installationer ett känt-gott bygge. Användaren kan
/// fortfarande uppdatera medvetet (clear_server_binary), och om den pinnade taggen
/// av någon anledning saknas faller vi tillbaka till latest.
const PINNED_TAG: &str = "b9467";

/// CUDA-versionen vi riktar oss mot. 12.4 har bredast drivrutinskompatibilitet
/// (NVIDIA R550+) och är säkrast på IT-kontrollerade maskiner med ev. äldre
/// drivrutiner. Vi buntar dessutom cudart-DLL:erna så att ingen CUDA-toolkit
/// behöver vara installerad – endast NVIDIA-drivrutinen (som följer med kortet).
#[cfg(all(target_os = "windows", not(feature = "cpu-only")))]
const CUDA_KEY: &str = "bin-win-cuda-12.4-x64";

/// Returnerar true om en NVIDIA-GPU med fungerande drivrutin finns.
/// Vi kör `nvidia-smi -L`: verktyget medföljer NVIDIA-drivrutinen, så att det
/// går att köra betyder både att kortet finns OCH att drivern fungerar – exakt
/// det vi behöver veta innan vi väljer CUDA-backend. Resultatet cachas.
#[cfg(all(target_os = "windows", not(feature = "cpu-only")))]
fn has_nvidia_gpu() -> bool {
    use std::sync::OnceLock;
    static DETECTED: OnceLock<bool> = OnceLock::new();
    *DETECTED.get_or_init(|| {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        std::process::Command::new("nvidia-smi")
            .arg("-L")
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map(|o| o.status.success() && !o.stdout.is_empty())
            .unwrap_or(false)
    })
}

/// Returns the primary platform key used for directory naming.
/// På Windows GPU-build väljs CUDA-katalogen om en NVIDIA-GPU upptäcks, annars
/// Vulkan. Detta håller CUDA- och Vulkan-installationerna i skilda kataloger så
/// att binär + DLL:er aldrig blandas ihop mellan backends.
fn primary_platform_key() -> &'static str {
    platform_keys()[0]
}

/// Returns the path where the llama-server binary should live.
pub fn server_binary_path(app: &AppHandle) -> PathBuf {
    crate::get_app_dir(app).join("bin").join(primary_platform_key()).join(BINARY_NAME)
}

/// Returns the path to the version file stored alongside the binary.
fn version_file_path(app: &AppHandle) -> PathBuf {
    crate::get_app_dir(app).join("bin").join(primary_platform_key()).join("version.txt")
}

/// Returns the installed llama-server release tag, if known.
pub fn get_installed_version(app: &AppHandle) -> Option<String> {
    std::fs::read_to_string(version_file_path(app))
        .ok()
        .map(|s| s.trim().to_string())
}

/// Removes the binary and its version file so the next call to
/// `ensure_server_binary` will re-download the latest release.
pub fn clear_server_binary(app: &AppHandle) -> Result<(), String> {
    let bin_path = server_binary_path(app);
    let ver_path = version_file_path(app);
    if bin_path.exists() {
        std::fs::remove_file(&bin_path)
            .map_err(|e| format!("Kunde inte ta bort binär: {}", e))?;
    }
    if ver_path.exists() {
        let _ = std::fs::remove_file(&ver_path);
    }
    Ok(())
}

/// Ensure the llama-server binary exists; download + extract if not.
/// A `version.txt` file next to the binary tracks which release is installed.
pub async fn ensure_server_binary(app: &AppHandle) -> Result<PathBuf, String> {
    let bin_path = server_binary_path(app);
    if bin_path.exists() && version_file_path(app).exists() {
        return Ok(bin_path);
    }

    let bin_dir = bin_path.parent().unwrap().to_path_buf();
    std::fs::create_dir_all(&bin_dir)
        .map_err(|e| format!("Kunde inte skapa bin-katalog: {}", e))?;

    let asset = find_release_asset().await?;
    let bytes = download_asset(&asset.binary_url).await?;

    if asset.binary_url.ends_with(".zip") {
        extract_zip(&bytes, &bin_dir)?;
    } else if asset.binary_url.ends_with(".tar.gz") {
        extract_tgz(&bytes, &bin_dir)?;
    } else {
        return Err(format!("Okänt arkivformat för URL: {}", asset.binary_url));
    }

    // CUDA-binären länkar dynamiskt mot runtime-DLL:er (cudart/cublas) som ligger
    // i ett separat cudart-paket. Hämta och packa upp det bredvid binären så att
    // appen blir self-contained (ingen CUDA-toolkit krävs på maskinen). extract_zip
    // plockar redan ut alla .dll på Windows.
    if let Some(cudart_url) = &asset.cudart_url {
        match download_asset(cudart_url).await {
            Ok(cudart_bytes) => {
                if let Err(e) = extract_zip(&cudart_bytes, &bin_dir) {
                    eprintln!("[llama_server] Kunde inte packa upp cudart-DLL:er: {}", e);
                }
            }
            Err(e) => eprintln!("[llama_server] Kunde inte hämta cudart-paket: {}", e),
        }
    }

    if !bin_path.exists() {
        return Err(format!(
            "Binären '{}' hittades inte i arkivet",
            BINARY_NAME
        ));
    }

    // Make executable on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bin_path, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("chmod fail: {}", e))?;
    }

    // Persist installed version so we can show it in the UI and skip re-downloads
    std::fs::write(version_file_path(app), &asset.tag)
        .map_err(|e| format!("Kunde inte spara version: {}", e))?;

    Ok(bin_path)
}


/// Resultatet av att slå upp rätt llama-server-asset för denna plattform.
struct ReleaseAsset {
    /// URL till själva binär-arkivet (zip/tgz).
    binary_url: String,
    /// URL till cudart-paketet med runtime-DLL:er – endast satt när binären är
    /// ett CUDA-bygge på Windows.
    cudart_url: Option<String>,
    /// Release-taggen (t.ex. "b9467").
    tag: String,
}

/// Find the right asset URL and release tag for this platform.
/// Försöker först den pinnade taggen (PINNED_TAG) för ett känt-gott bygge och
/// faller tillbaka till `releases/latest` om den taggen saknas eller inte har
/// någon matchande asset.
async fn find_release_asset() -> Result<ReleaseAsset, String> {
    let client = reqwest::Client::builder()
        .user_agent("loki-app")
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    // 1) Pinnad release
    let pinned_url = format!(
        "https://api.github.com/repos/ggml-org/llama.cpp/releases/tags/{}",
        PINNED_TAG
    );
    if let Ok(found) = fetch_asset_from(&client, &pinned_url).await {
        return Ok(found);
    }
    eprintln!(
        "[llama_server] Pinnad tag {} otillgänglig, faller tillbaka till latest",
        PINNED_TAG
    );

    // 2) Fallback: senaste release
    fetch_asset_from(
        &client,
        "https://api.github.com/repos/ggml-org/llama.cpp/releases/latest",
    )
    .await
}

/// Hämtar en release-JSON från `api_url` och plockar ut binär-URL (+ ev. cudart-URL)
/// och tag för den första platform-nyckeln som matchar.
async fn fetch_asset_from(
    client: &reqwest::Client,
    api_url: &str,
) -> Result<ReleaseAsset, String> {
    let resp = client
        .get(api_url)
        .send()
        .await
        .map_err(|e| format!("GitHub API fel: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("GitHub API svarade HTTP {}", resp.status()));
    }

    let json: serde_json::Value = resp
        .json::<serde_json::Value>()
        .await
        .map_err(|e| format!("JSON parse-fel: {}", e))?;

    let tag = json["tag_name"]
        .as_str()
        .unwrap_or("unknown")
        .to_string();

    let assets = json["assets"]
        .as_array()
        .ok_or("Inga assets i GitHub-svaret")?;

    // Try each candidate key in priority order
    for key in platform_keys() {
        if let Some(asset) = assets.iter().find(|a| {
            a["name"]
                .as_str()
                .map(|n| n.starts_with("llama-") && n.contains(key) && n.ends_with(ASSET_EXTENSION))
                .unwrap_or(false)
        }) {
            let binary_url = asset["browser_download_url"]
                .as_str()
                .map(|s| s.to_string())
                .ok_or("Ingen download URL i asset")?;

            // För CUDA-bygget: hitta matchande cudart-paket (runtime-DLL:er) i
            // samma release. Namnformat: cudart-llama-bin-win-cuda-12.4-x64.zip.
            let cudart_url = if key.contains("cuda") {
                assets
                    .iter()
                    .find(|a| {
                        a["name"]
                            .as_str()
                            .map(|n| n.starts_with("cudart-") && n.contains(key))
                            .unwrap_or(false)
                    })
                    .and_then(|a| a["browser_download_url"].as_str())
                    .map(|s| s.to_string())
            } else {
                None
            };

            return Ok(ReleaseAsset { binary_url, cudart_url, tag });
        }
    }

    Err(format!(
        "Hittade ingen tillgång ({}) för platform (testad: {})",
        ASSET_EXTENSION,
        platform_keys().join(", ")
    ))
}

/// Returns candidate platform keys in priority order.
/// For CPU-only builds on Windows we try several CPU binary names since
/// the exact naming varies between llama.cpp releases.
fn platform_keys() -> Vec<&'static str> {
    #[cfg(all(target_os = "windows", feature = "cpu-only"))]
    return vec![
        "bin-win-cpu-x64",      // nytt namn från b8000+
        "bin-win-avx2-x64",     // äldre releases
        "bin-win-openblas-x64",
        "bin-win-avx-x64",
        "bin-win-noavx-x64",
    ];

    #[cfg(all(target_os = "windows", not(feature = "cpu-only")))]
    {
        // NVIDIA → prova CUDA först, med Vulkan som fallback. Annars bara Vulkan.
        if has_nvidia_gpu() {
            return vec![CUDA_KEY, "bin-win-vulkan-x64"];
        }
        return vec!["bin-win-vulkan-x64"];
    }

    #[cfg(target_os = "macos")]
    return vec!["bin-macos-arm64"];

    #[cfg(target_os = "linux")]
    return vec!["bin-ubuntu-x64"];
}

/// Download the asset (ZIP or TGZ) from `url` and return raw bytes.
async fn download_asset(url: &str) -> Result<Vec<u8>, String> {
    let client = reqwest::Client::builder()
        .user_agent("loki-app")
        .timeout(std::time::Duration::from_secs(300)) // 5-minute download timeout
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Nedladdningsfel: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("HTTP {} vid nedladdning av binary", resp.status()));
    }

    resp.bytes()
        .await
        .map(|b| b.to_vec())
        .map_err(|e| format!("Läsfel: {}", e))
}

/// Extract `llama-server[.exe]` (and any DLLs needed on Windows) from ZIP bytes into `dest_dir`.
fn extract_zip(zip_bytes: &[u8], dest_dir: &std::path::Path) -> Result<(), String> {
    use std::io::Cursor;
    let cursor = Cursor::new(zip_bytes);
    let mut archive =
        zip::ZipArchive::new(cursor).map_err(|e| format!("ZIP open-fel: {}", e))?;

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("ZIP entry-fel: {}", e))?;

        let name = entry.name().to_string();
        let filename = std::path::Path::new(&name)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();

        // Extract the server binary and any DLLs that may be needed
        let should_extract = filename == BINARY_NAME
            || (cfg!(target_os = "windows") && filename.ends_with(".dll"));

        if should_extract {
            let dest = dest_dir.join(&filename);
            let mut out = std::fs::File::create(&dest)
                .map_err(|e| format!("Kunde inte skapa {}: {}", filename, e))?;
            std::io::copy(&mut entry, &mut out)
                .map_err(|e| format!("Kunde inte skriva {}: {}", filename, e))?;
        }
    }

    Ok(())
}

/// Extract `llama-server` from .tar.gz bytes into `dest_dir`.
fn extract_tgz(tgz_bytes: &[u8], dest_dir: &std::path::Path) -> Result<(), String> {
    use flate2::read::GzDecoder;
    use tar::Archive;
    use std::io::Cursor;

    let decoder = GzDecoder::new(Cursor::new(tgz_bytes));
    let mut archive = Archive::new(decoder);

    for entry_result in archive.entries().map_err(|e| format!("TGZ entries-fel: {}", e))? {
        let mut entry = entry_result.map_err(|e| format!("TGZ entry-fel: {}", e))?;
        let path = entry.path().map_err(|e| format!("TGZ path-fel: {}", e))?.to_path_buf();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

        let is_lib = name.contains("libggml") || name.contains("libllama") || name.contains("libmtmd");
        let should_extract = name == BINARY_NAME || is_lib;

        if should_extract && !name.is_empty() {
            let dest = dest_dir.join(name);
            
            // Handle symlinks or regular files
            if entry.header().entry_type().is_symlink() {
                #[cfg(unix)]
                {
                    if let Ok(Some(target)) = entry.link_name() {
                        let _ = std::fs::remove_file(&dest); // Clear if exists
                        
                        // Symlinks in the tarball might be like "libllama.0.dylib -> libllama.0.0.8247.dylib"
                        // Since we extract everything into one flat dir, we just link to the target name
                        let target_name = target.file_name().and_then(|n| n.to_str()).unwrap_or("");
                        if !target_name.is_empty() {
                            if let Err(e) = std::os::unix::fs::symlink(target_name, &dest) {
                                eprintln!("Kunde inte skapa symlink {} -> {}: {}", name, target_name, e);
                            }
                        }
                    }
                }
            } else {
                let mut out = std::fs::File::create(&dest)
                    .map_err(|e| format!("Kunde inte skapa {}: {}", name, e))?;
                std::io::copy(&mut entry, &mut out)
                    .map_err(|e| format!("Kunde inte skriva {}: {}", name, e))?;
                
                // Make executable on Unix
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755));
                }
            }
        }
    }

    Ok(())
}
