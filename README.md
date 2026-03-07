# Loki – Lokal AI-chat

Loki är en integritetsfokuserad AI-chattapplikation som körs helt lokalt på din dator. Inga molntjänster, inga prenumerationer och dina data lämnar aldrig din enhet.

Appen drivs av [llama.cpp](https://github.com/ggml-org/llama.cpp) via en inbäddad llama-server och stödjer ett brett urval av GGUF-modeller med hårdvaruacceleration (Vulkan på Windows, Metal på macOS).

## Funktioner

- **100% lokal AI** – All inferens sker på din hårdvara, inget skickas till externa servrar
- **Modellväljare med smaker** – Fem förkurerade modeller att ladda ner direkt i appen
- **RAM-varningar** – Appen visar om en modell kräver mer minne än du har tillgängligt
- **Vision + ljud** – Stöd för bildanalys och ljudinmatning (kräver multimodal modell)
- **Byt modell utan omstart** – Starta en annan modell direkt från sidomenyn
- **Kontext-bilagor** – Bifoga textfiler, PDF-filer och bilder direkt i chatten
- **Konversationshistorik** – All historik sparas lokalt i webbläsarens IndexedDB
- **Systemprompt** – Konfigurera AI:ns beteende per konversation
- **Importera / exportera** – Säkerhetskopiera och flytta konversationer
- **Retro-tema** – Valfritt terminalinspirerat grönt tema med scanline-effekt
- **Mörkt/ljust tema** – Välj tema i inställningarna
- **Helt på svenska** – Gränssnittet är fullt översatt till svenska

## Modeller

Loki levereras med fem rekommenderade GGUF-modeller:

| Smak | Modell | Storlek | RAM | Beskrivning |
|------|--------|---------|-----|-------------|
| Kompakt | Gemma 3n E2B | ~1.7 GB | 3 GB | Googles nya effektiva modell – vision + ljud |
| Vision | Gemma 3n E4B | ~2.3 GB | 4 GB | Starkare variant med vision + ljud |
| Analytisk | Gemma 3 4B | ~2.5 GB | 4 GB | Bäst på analys och sammanfattning |
| Balanserad | Ministral 3B | ~3.2 GB | 5 GB | Mistrals 3B – stark på instruktioner och svenska |
| Kraftfull | Mistral 7B | ~4.1 GB | 6 GB | Bäst på svenska och komplexa instruktioner |

Du kan även lägga egna GGUF-modeller i appens modellmapp och starta dem direkt.

## Teknikstack

| Komponent | Teknologi |
|-----------|-----------|
| Skrivbordsram | [Tauri 2](https://tauri.app/) (Rust) |
| Frontend | [SvelteKit 5](https://svelte.dev/) + TypeScript + Vite |
| UI-komponenter | [shadcn-svelte](https://www.shadcn-svelte.com/) + Tailwind CSS |
| AI-backend | [llama.cpp](https://github.com/ggml-org/llama.cpp) via inbäddad llama-server |
| Modellformat | [GGUF](https://github.com/ggml-org/ggml/blob/master/docs/gguf.md) |
| Hårdvaruacceleration | Vulkan (Windows) / Metal (macOS) |
| Lokal lagring | IndexedDB (konversationer), localStorage (inställningar) |

## Arkitektur

```
Tauri (Rust)
  ├── llama-server subprocess  ←  kör GGUF-modeller med Vulkan/Metal
  ├── Nedladdningshanterare    ←  hämtar modeller från Hugging Face
  └── WebView
        └── SvelteKit WebUI   ←  kommunicerar med llama-server via HTTP/SSE
```

llama-server exponerar ett OpenAI-kompatibelt API (`/v1/chat/completions`) som frontend konsumerar med streaming via Server-Sent Events (SSE).

## Kom igång (utveckling)

Krav: [Node.js](https://nodejs.org/) 20+, [Rust](https://rustup.rs/) (stable), [Vulkan SDK](https://vulkan.lunarg.com/) (Windows) eller Xcode (macOS)

```bash
# Installera beroenden
npm install

# Starta i utvecklingsläge (HMR aktiverat)
npm run dev           # Bara webgränssnittet
npx tauri dev         # Fullständig Tauri-app

# Bygg för produktion
npm run build         # Skapar installerare i src-tauri/target/release/bundle/
```

### Windows-bygge

```bash
npm run build
# Installerare skapas i:
#   src-tauri/target/release/bundle/msi/    ← MSI
#   src-tauri/target/release/bundle/nsis/   ← EXE
```

### macOS-bygge

```bash
npm run build
# App bundle skapas i:
#   src-tauri/target/release/bundle/macos/
#   src-tauri/target/release/bundle/dmg/
```

## Egna modeller

Placera valfri GGUF-fil i appens modellmapp (visas under "Egna modeller" i modellväljaren). Modellen startas direkt – inga konfigurationsfiler behövs.

Rekommenderade resurser för att hitta modeller:
- [Hugging Face GGUF-modeller](https://huggingface.co/models?library=gguf)
- [bartowski's GGUF-repon](https://huggingface.co/bartowski) – välkurerade kvantiseringar

## Relaterade projekt

- [llama.cpp](https://github.com/ggml-org/llama.cpp) – inferensmotorn som driver Loki
- [Tauri](https://github.com/tauri-apps/tauri) – skrivbordsramen
- [SvelteKit](https://github.com/sveltejs/kit) – frontend-ramverket
