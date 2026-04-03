# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Overview

**Loki** är en Tauri 2-skrivbordsapp för lokal AI-chatt (utan moln). Backend i Rust (`src-tauri/`), frontend i SvelteKit (`webui/`). Appen bäddar in en `llama-server`-binär (llama.cpp) som laddas ner vid behov och körs lokalt på port 127.0.0.1:{dynamic}.

## Kommandon

### Utveckling & bygge (rot)
```bash
npm run dev        # Startar Tauri dev (startar även webui-devservern automatiskt)
npm run build      # Bygger hela appen (webui + Rust)
```

### Frontend (webui/)
```bash
cd webui
npm run dev        # Vite dev-server på :5173 (fristående, utan Tauri)
npm run lint       # prettier + eslint
npm run check      # svelte-check (TypeScript)
npm run format     # prettier --write

# Tester
npm run test              # Kör alla tester (unit + client + ui + e2e)
npm run test:unit         # Vitest, Node-miljö (tests/unit/)
npm run test:client       # Vitest, browser/Chromium (tests/client/)
npm run test:ui           # Storybook stories som tester (tests/stories/)
npm run test:e2e          # Playwright e2e (tests/e2e/, bygger först)
```

### Rust-backend
```bash
cd src-tauri
cargo build                        # Standard build (GPU/Vulkan)
cargo build --features cpu-only    # CPU-only build (ingen Vulkan)
cargo test
```

## Arkitektur

### Frontend (`webui/src/lib/`)

| Mapp | Syfte |
|------|-------|
| `services/` | Stateless klasser: `ChatService` (streaming/tokenize), `DatabaseService` (Dexie), `ModelsService`, `McpService` |
| `stores/` | Reaktiv Svelte 5-state: `agenticStore`, `serverStore`, `modelsStore`, `settings`, `mcpStore`, m.fl. |
| `components/` | UI-komponenter grupperade under `app/`, `chat/`, `models/`, `mcp/`, osv. |
| `types/` | TypeScript-typer; `api.ts` = llama.cpp-API, `agentic.ts` = agentiskt flöde |
| `constants/` | Regexar (AGENTIC_REGEX), taggar, gränsvärden |
| `enums/` | Delade enums (MessageRole, ContentPartType, etc.) |

**Mönster:** Services är stateless, stores hanterar reaktiv state + affärslogik. `agenticStore` orkestrar multi-turn agentic loops med MCP-verktyg.

**Databas:** Dexie (IndexedDB), schema i `database.service.ts` – tabeller `conversations` och `messages`.

**Routing:** SvelteKit – enda väsentliga route är `routes/chat/`.

### Backend (`src-tauri/src/`)

| Fil | Syfte |
|-----|-------|
| `lib.rs` | Tauri-kommandon (exponerade till frontend), appinitiering |
| `inference.rs` | `InferenceEngine` – hanterar llama-server-processen; GPU-fallback till CPU vid OOM |
| `llama_server.rs` | Binärnedladdning från llama.cpp GitHub Releases, extraktion, versionsspårning |
| `model_download.rs` | GGUF-modellregister, nedladdning från Hugging Face |

**Binär- och modellsökvägar:**
- Binär: `{app_data}/bin/{platform_key}/llama-server[.exe]`
- Modeller: `{app_data}/models/*.gguf`
- Serverlogg: `{app_log}/llama_server.log`

**CPU-only-build:** Kompilatorflaggan `--features cpu-only` sätter `GGML_VK_VISIBLE_DEVICES=""` och hoppar över GPU-start helt.

### Frontend ↔ Tauri-kommunikation

`tauri-bridge.ts` och `$lib/server-url.ts` kapslar Tauri `invoke()`-anrop. Under `vite dev` proxyas `/v1`, `/props`, `/models`, `/cors-proxy` till `localhost:8080`.

## Viktiga detaljer

- **Svelte 5** används – syntax med runes (`$state`, `$derived`, `$effect`), inte Svelte 4-syntax.
- **Dokumentanalys (Map-Reduce):** Tre lägen – *Auto*, *Extrahera* (extrahering), *Sammanfatta* (progressiv summering). Logiken lever i `agenticStore` + `ChatService`.
- **Versionering:** Versions-nummret finns på tre ställen: `package.json` (rot), `webui/package.json`, `src-tauri/Cargo.toml` och `src-tauri/tauri.conf.json` – håll dem i synk vid release.
- **Playwright e2e** kräver att appen byggs och serveras på port 8181 (`npm run build && http-server ../public -p 8181`). Körs aldrig mot Tauri direkt.
