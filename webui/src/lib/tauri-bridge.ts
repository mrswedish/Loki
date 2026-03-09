/**
 * Tauri bridge – kommunikation med Rust-backendet.
 *
 * Används BARA när appen kör i Tauri (window.__TAURI_INTERNALS__ finns).
 * I vanlig webbläsare är alla funktioner no-ops / returnerar null.
 */
import { setServerBase, isTauriEnv } from '$lib/server-url';

// Lazy-import av Tauri API så att builden inte kraschar i icke-Tauri miljöer
async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
	const { invoke: tauriInvoke } = await import('@tauri-apps/api/core');
	return tauriInvoke<T>(cmd, args);
}

export interface ModelInfo {
	name: string;
	filename: string;
	path: string;
	size_bytes: number;
}

export interface ModelStatus {
	id: string;
	name: string;
	filename: string;
	size_bytes: number;
	description: string;
	flavor: string;
	ram_required_gb: number;
	downloaded: boolean;
}

export interface SystemInfo {
	total_ram_gb: number;
	available_ram_gb: number;
}

/**
 * Försöker hämta den aktiva llama-server URL från Rust.
 * Returnerar null om servern inte är startad än.
 */
export async function fetchServerUrl(): Promise<string | null> {
	if (!isTauriEnv()) return null;
	try {
		const url = await invoke<string | null>('get_server_url');
		if (url) {
			setServerBase(url);
			return url;
		}
		return null;
	} catch {
		return null;
	}
}

/** Lista lokalt nedladdade GGUF-modeller */
export async function listLocalModels(): Promise<ModelInfo[]> {
	if (!isTauriEnv()) return [];
	return invoke<ModelInfo[]>('list_models');
}

/** Lista alla tillgängliga modeller (nedladdade + ej nedladdade) */
export async function listAvailableModels(): Promise<ModelStatus[]> {
	if (!isTauriEnv()) return [];
	return invoke<ModelStatus[]>('list_available_models');
}

/** Starta llama-server med vald modell. Returnerar server-URL. */
export async function startServer(modelPath: string, contextSize?: number): Promise<string> {
	const url = await invoke<string>('start_server', { modelPath, contextSize });
	setServerBase(url);
	return url;
}

/** Stäng av llama-server */
export async function stopServer(): Promise<void> {
	if (!isTauriEnv()) return;
	await invoke<void>('stop_server');
}

/** Ladda ner en modell. Returnerar sökvägen till den nedladdade filen. */
export async function downloadModel(modelId: string): Promise<string> {
	return invoke<string>('download_model_cmd', { modelId });
}

/** Radera en nedladdad modell */
export async function deleteModel(modelId: string): Promise<void> {
	return invoke<void>('delete_model_cmd', { modelId });
}

/** Hämta systeminformation (RAM) */
export async function getSystemInfo(): Promise<SystemInfo> {
	if (!isTauriEnv()) return { total_ram_gb: 0, available_ram_gb: 0 };
	return invoke<SystemInfo>('get_system_info');
}

export interface DownloadProgress {
	model_id: string;
	percent: number;
	downloaded_bytes: number;
	total_bytes: number;
}

/** Lyssna på nedladdningsprogress. Returnerar unlisten-funktion. */
export async function onDownloadProgress(
	cb: (p: DownloadProgress) => void
): Promise<() => void> {
	const { listen } = await import('@tauri-apps/api/event');
	return listen<DownloadProgress>('download-progress', (e) => cb(e.payload));
}
