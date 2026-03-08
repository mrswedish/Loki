/**
 * Manages the base URL for llama-server HTTP API.
 *
 * In normal web mode (served by llama-server itself), all API calls use
 * relative URLs and this returns ''.
 *
 * In Tauri mode, llama-server runs as a subprocess on a dynamic port.
 * The Tauri backend exposes get_server_url() which returns
 * "http://127.0.0.1:{port}". We store that here so all services use it.
 */

let _serverBase = '';

export function getServerBase(): string {
	return _serverBase;
}

export function setServerBase(url: string): void {
	_serverBase = url.replace(/\/$/, ''); // strip trailing slash
}

export function isTauriEnv(): boolean {
	return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}
