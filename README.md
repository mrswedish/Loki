# Loki 3.0 — Lokala mötesanteckningar på din dator

Loki förvandlar råa mötestranskriberingar och underlag till rena, välstrukturerade svenska **minnesanteckningar, mötesprotokoll, beslutsloggar och åtgärdslistor** – **100 % lokalt på din egen dator** utan molntjänster eller externa API-anrop.

---

## Höjdpunkter i Loki 3.0

- **100 % lokalt & integritetssäkert:** All AI-bearbetning, dokumentläsning och historik stannar på datorn. Noll telemetri och noll externa AI-tjänster.
- **Inbyggd C++/Rust-motor (`llama-cpp-2`):** Ingen extern webbserver eller krånglig installation. Kör direkt med Apple Silicon Metal på Mac och portabel flerkärnig CPU på Windows (`Loki-portabel.exe`).
- **Varm mötescache – byt mall på under en sekund:** När ett möte har lästs in ligger underlaget kvar varmt i minnet. Växla direkt mellan *Minnesanteckning*, *Mötesprotokoll*, *Åtgärdspunkter*, *Beslutslogg* och egna mallar utan att modellen behöver läsa om mötestexten.
- **Bred dokumentimport (utan installerat Office):**
  - **Mötestranskript:** `.vtt`, `.srt` samt inklistrade Teams-/Zoom-transkript (rensar automatiskt bort tidsstämplar och slår ihop repliker från samma talare).
  - **Dokument & Kalkylblad:** Word (`.docx`), OpenDocument (`.odt`), Excel (`.xlsx`, `.xls`, `.ods`), PDF (`.pdf`) samt `.txt`, `.md` och `.csv` (med svensk Windows-1252-fallback).
- **Två optimerade QAT-språkmodeller:**
  - **Gemma 4 E2B QAT (`2,62 GB`, standard):** Snabb och träffsäker på vanliga bärbara datorer. *(Delas automatiskt om du redan har Maskera installerad!)*
  - **Gemma 4 E4B QAT (`4,22 GB`, valfri):** För extra nyanserade protokoll och krävande fackterminologi.
- **Färdigt för Word & Outlook:** Kopiera med riktiga rubriker och punktlistor direkt till urklipp, eller exportera en formaterad **Word-fil (`.docx`)**, `.md` eller `.txt`.

---

## Bygga från källkod

### Förutsättningar
- **Node.js** 20+
- **Rust** (`rustup`, stable)
- På Windows vid eget bygge: LLVM (`LIBCLANG_PATH`) och Visual Studio C++ Build Tools. På macOS: Xcode Command Line Tools.

```bash
npm install
npm run tauri dev
```

För produktionsbygge:

```bash
npm run tauri build
```
