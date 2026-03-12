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

/// Returns the path where the llama-server binary should live.
pub fn server_binary_path(app: &AppHandle) -> PathBuf {
    let key = platform_key();
    crate::get_app_dir(app).join("bin").join(key).join(BINARY_NAME)
}

/// Returns the path to the version file stored alongside the binary.
fn version_file_path(app: &AppHandle) -> PathBuf {
    let key = platform_key();
    crate::get_app_dir(app).join("bin").join(key).join("version.txt")
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

    let (url, tag) = find_release_asset().await?;
    let bytes = download_asset(&url).await?;

    if url.ends_with(".zip") {
        extract_zip(&bytes, &bin_dir)?;
    } else if url.ends_with(".tar.gz") {
        extract_tgz(&bytes, &bin_dir)?;
    } else {
        return Err(format!("Okänt arkivformat för URL: {}", url));
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
    std::fs::write(version_file_path(app), &tag)
        .map_err(|e| format!("Kunde inte spara version: {}", e))?;

    Ok(bin_path)
}


/// Fetch latest release JSON from GitHub, find the right asset URL and release tag.
/// Returns `(download_url, tag_name)`.
async fn find_release_asset() -> Result<(String, String), String> {
    let client = reqwest::Client::builder()
        .user_agent("loki-app")
        .timeout(std::time::Duration::from_secs(30))
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

    let tag = json["tag_name"]
        .as_str()
        .unwrap_or("unknown")
        .to_string();

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
                .map(|n: &str| {
                    n.starts_with("llama-") && n.contains(platform_key) && n.ends_with(ASSET_EXTENSION)
                })
                .unwrap_or(false)
        })
        .ok_or_else(|| format!("Hittade ingen tillgång ({}) för platform '{}'", ASSET_EXTENSION, platform_key))?;

    let url = asset["browser_download_url"]
        .as_str()
        .map(|s: &str| s.to_string())
        .ok_or("Ingen download URL i asset".to_string())?;

    Ok((url, tag))
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
