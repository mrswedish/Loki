# CLAUDE.md — Loki 3.0 Arkitekturguide

Detta dokument ger sammanhang och riktlinjer för vidareutveckling av **Loki 3.0**.

## Syfte

Loki är **inte** en allmän AI-chatt. Applikationens enda uppdrag är att omvandla råa mötestranskriberingar och mötesunderlag till rena, välstrukturerade svenska mötesanteckningar och protokoll – **100 % lokalt, snabbt och säkert utan molntjänster**.

Arkitekturen delar ramverk, designsystem och in-process `llama-cpp-2`-mönster med systerapplikationen **Maskera**, medan ljudinspelning/tal-till-text ligger i systerapplikationen **Transkribera**.

---

## Teknikstack & Projektstruktur

- **Frontend (`src/`):** `React 19` + `TypeScript` + `Vite 7`, tillgänglighetsanpassad CSS med `light-dark()` (WCAG 2.2 AA), native `<dialog>`, `<progress>` och `role="toolbar"`.
  - `src/App.tsx`: Hela arbetsflödet (modellhantering, inmatning/drag-och-släpp, mallval, live-streaming, 1-klicks mallväxling, direktredigering, iterativ *Justera resultatet* med *Ångra*, lokal möteshistorik och export).
  - `src/templates.ts`: De 6 svenska dokumentmallarna (*Minnesanteckning*, *Mötesprotokoll*, *Kort sammanfattning*, *Åtgärdspunkter*, *Beslutslogg*, *Rätta transkriberingen*) samt `COMMON_PREAMBLE`.
  - `src/markdownUtils.tsx`: XSS-säker React-rendering av Markdown samt `markdownToCleanHtml` (för `ClipboardItem` `text/html` till Word/Outlook) och `markdownToCleanPlainText`.
- **Backend (`src-tauri/src/`):** `Tauri 2` + `Rust` (inga externa sidovagnar eller localhost-HTTP-servrar).
  - `summarizer.rs`: In-process LLM-inferens via `llama-cpp-2` (`0.1.161`, Metal på macOS, portabel SIMD/AVX2 CPU på Windows). Hanterar `mmap`, delad CPU-trådning (`optimal_cpu_threads`), RAM-medvetet kontextfönster (`sysinfo`), **Varm KV-cache** över mallbyten (`clear_kv_cache_seq` på längsta gemensamma token-prefix), Gemma 4 native `<|channel>thought`-styrning (med budgettak som injicerar `\n<channel|>` direkt i KV-cachen), `< 50 ms` `AtomicBool`-avbrott och automatisk Map-Reduce-fallback för jättemöten.
  - `transcript.rs`: Deterministisk Lager 0-städning (`< 5 ms`) av `.vtt`, `.srt` och Teams/Zoom-transkript (tar bort tidsstämplar, UUID/cue-ID och slår ihop repliker från samma talare).
  - `documents.rs`: Portabel dokumentinläsning helt i Rust (`.docx`, `.odt`, `.xlsx`/`.xls`/`.xlsm`/`.xlsb`/`.ods`, `.pdf`, `.vtt`, `.srt`, `.txt`/`.md`/`.csv` med svensk Windows-1252-fallback) samt äkta OpenXML `.docx`-export (`export_markdown_to_docx_bytes`).
  - `lib.rs`: Tauri-kommandon, modellnedladdare med `reqwest` (`native-tls` + `.part`), automatisk modellåteranvändning från Maskera (`com.patriksvensk.maskera`), offline-import samt lokal JSON-persistens (`loki_state.json`).

---

## Språkmodeller (QAT GGUF)

1. **Standard (`e2b`, 2,62 GB):** `unsloth/gemma-4-E2B-it-qat-GGUF` (`gemma-4-E2B-it-qat-UD-Q4_K_XL.gguf`)
2. **Fördjupad (`e4b`, 4,22 GB, valfri):** `unsloth/gemma-4-E4B-it-qat-GGUF` (`gemma-4-E4B-it-qat-UD-Q4_K_XL.gguf`)
3. **Maximal (`12b`, 6,72 GB, valfri):** `unsloth/gemma-4-12B-it-qat-GGUF` (`gemma-4-12B-it-qat-UD-Q4_K_XL.gguf`) — rekommenderas för datorer med mer än 16 GB RAM.

Om användaren redan har laddat ner `gemma-4-E2B-it-qat-UD-Q4_K_XL.gguf` i **Maskera** upptäcker Loki filen automatiskt utan ny nedladdning.

---

## Utvecklingskommandon

```bash
# Installera frontendberoenden
npm install

# Kör i utvecklingsläge (kompilerar C++/llama.cpp med O3 via Cargo.toml)
npm run tauri dev

# Kontrollera frontend & kör alla Rust-enhetstester
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
```
