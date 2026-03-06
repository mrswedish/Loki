/// Downloads and manages the llama-server binary from llama.cpp GitHub releases.
use std::path::PathBuf;
use tauri::AppHandle;

#[cfg(target_os = "windows")]
const BINARY_NAME: &str = "llama-server.exe";
#[cfg(not(target_os = "windows"))]
const BINARY_NAME: &str = "llama-server";

/// Returns the path where the llama-server binary should live.
pub fn server_binary_path(app: &AppHandle) -> PathBuf {
    crate::get_app_dir(app).join("bin").join(BINARY_NAME)
}

/// Ensure the llama-server binary exists; download + extract if not.
pub async fn ensure_server_binary(app: &AppHandle) -> Result<PathBuf, String> {
    let bin_path = server_binary_path(app);
    if bin_path.exists() {
        return Ok(bin_path);
    }

    let bin_dir = bin_path.parent().unwrap().to_path_buf();
    std::fs::create_dir_all(&bin_dir)
        .map_err(|e| format!("Kunde inte skapa bin-katalog: {}", e))?;

    let url = find_release_asset_url().await?;
    let zip_bytes = download_zip(&url).await?;
    extract_binary(&zip_bytes, &bin_dir)?;

    if !bin_path.exists() {
        return Err(format!(
            "Binären '{}' hittades inte i ZIP:en",
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

    Ok(bin_path)
}

/// Picks the right ZIP asset name pattern for the current platform.
#[cfg(target_os = "windows")]
fn asset_pattern() -> &'static str {
    "llama-b*-bin-win-vulkan-x64.zip"
}

#[cfg(target_os = "macos")]
fn asset_pattern() -> &'static str {
    // Apple Silicon – metal build
    "llama-b*-bin-macos-arm64.zip"
}

#[cfg(target_os = "linux")]
fn asset_pattern() -> &'static str {
    "llama-b*-bin-ubuntu-x64.zip"
}

/// Fetch latest release JSON from GitHub, find the right asset URL.
async fn find_release_asset_url() -> Result<String, String> {
    let client = reqwest::Client::builder()
        .user_agent("loke-app")
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    let resp = client
        .get("https://api.github.com/repos/ggerganov/llama.cpp/releases/latest")
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

    let assets = json["assets"]
        .as_array()
        .ok_or("Inga assets i GitHub-svaret")?;

    // Find the matching asset by platform pattern prefix
    let platform_key = platform_key();
    let asset = assets
        .iter()
        .find(|a| {
            a["name"]
                .as_str()
                .map(|n: &str| n.contains(platform_key) && n.ends_with(".zip"))
                .unwrap_or(false)
        })
        .ok_or_else(|| format!("Hittade ingen ZIP för platform '{}'", platform_key))?;

    asset["browser_download_url"]
        .as_str()
        .map(|s: &str| s.to_string())
        .ok_or("Ingen download URL i asset".to_string())
}

#[cfg(target_os = "windows")]
fn platform_key() -> &'static str {
    "bin-win-vulkan-x64"
}

#[cfg(target_os = "macos")]
fn platform_key() -> &'static str {
    "bin-macos-arm64"
}

#[cfg(target_os = "linux")]
fn platform_key() -> &'static str {
    "bin-ubuntu-x64"
}

/// Download the ZIP from `url` and return raw bytes.
async fn download_zip(url: &str) -> Result<Vec<u8>, String> {
    let client = reqwest::Client::builder()
        .user_agent("loke-app")
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
fn extract_binary(zip_bytes: &[u8], dest_dir: &std::path::Path) -> Result<(), String> {
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
