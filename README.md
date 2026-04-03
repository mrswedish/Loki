# LOKI – Lokal, Oberoende, Konfidentiell Intelligens

**LOKI** står för **L**okal, **O**beroende, **K**onfidentiell **I**ntelligens. Det är en stensäker, integritetsfokuserad AI-chatt som körs till 100 % på din egen maskin. Inga molntjänster. Inga prenumerationer. Dina ord lämnar aldrig din enhet.

Under huven drivs appen av [llama.cpp](https://github.com/ggml-org/llama.cpp) via en inbäddad server, med fullt stöd för ett brett urval av GGUF-modeller och blixtsnabb hårdvaruacceleration (Vulkan på Windows, Metal på macOS).

![Loki skärmdump](assets/screenshot.png)

---

## Varför välja Loki?

De flesta moderna AI-assistenter gör dig beroende av en tredjepart – du skickar dina frågor till ett externt datacenter där de bearbetas, loggas och analyseras innan du får ett svar. Dina privata konversationer, arbetsdokument och idéer passerar genom system du helt saknar kontroll över.

Loki ger dig makten tillbaka. Modellen bor hos dig. Beräkningen sker lokalt. Du är helt oberoende av uppkoppling och allt du gör förblir strikt konfidentiellt.

**Lokis huvudsyfte:**

**Mötesanteckningar från transkriberingar** – Klistra in eller bifoga din transkribering (t.ex. exporterad från Teams, Zoom eller liknande) och låt Loki skapa strukturerade mötesanteckningar med beslut, action points och nyckelinformation. Allt sker lokalt – transkriberingen lämnar aldrig din enhet.

---

## Säkerhet och integritet på riktigt

Att köra AI lokalt handlar inte bara om noll driftkostnader – det är den enda garantin för total digital integritet.

| Vad Loki *inte* gör | Varför det spelar roll |
| --- | --- |
| **Inga API-anrop till externa servrar** | Dina frågor och svar lämnar aldrig ditt eget nätverk. |
| **Inget konto, ingen inloggning** | Det finns inga användaruppgifter som kan läcka, säljas eller kapas. |
| **Ingen telemetri eller loggning** | Loki spionerar inte på dig och samlar inte in någon användningsdata. |
| **Inget internetkrav (efter nedladdning)** | Helt oberoende. Fungerar perfekt offline, på isolerade nätverk eller bakom strikta brandväggar. |
| **Allt sparas lokalt (IndexedDB)** | Du äger din konfidentiella historik fullt ut – ingen annan kan komma åt den. |

Loki är det perfekta verktyget för att bolla känsliga ämnen, effektivisera interna arbetsflöden, granska konfidentiella dokument och hantera alla situationer där du vägrar låta en tredje part tjuvlyssna.

> **Obs:** Modellfilerna laddas ner från Hugging Face första gången du använder dem. Därefter krävs ingen som helst internetuppkoppling för att använda appen.

---

## Nyckelfunktioner

* **100 % lokal AI** – All tankekraft genereras av din egen hårdvara, helt oberoende av molnet.
* **Färdiga "smaker"** – Ett kurerat urval av optimerade modeller (GGUF) som laddas ner direkt i appen. Inkluderar modeller från Google, Microsoft och Mistral AI.
* **Modeller som ingår (förval)**:
  | Modell | Storlek | RAM-krav | Fokus |
  | :--- | :--- | :--- | :--- |
  | **Gemma 4 E2B** | ~3.5 GB | 5 GB+ | Blixtsnabb, multimodal, 256K kontext |
  | **Ministral 3B** | ~2.3 GB | 5 GB+ | Kompakt Expert |
  | **Gemma 4 E4B** | ~6.9 GB | 9 GB+ | Analytisk, multimodal, 256K kontext |
  | **Ministral 8B** | ~5.6 GB | 10 GB+ | Allround & Kraftfull |

> [!TIP]
> **Upplever du att en modell inte startar?** Om du har ett grafikkort med begränsat VRAM (t.ex. 6 GB), prova att sänka **Context Size** i inställningarna till 4096 eller 2048. Detta minskar minneskravet avsevärt vid start.

> [!TIP]
> **Problem med GPU på Windows (t.ex. AMD Radeon)?** Testa CPU-versionen av Loki (`loki-cpu`) som inte använder Vulkan alls. Den är något långsammare men fungerar på i stort sett all hårdvara.

* **Intelligent dokumentanalys (Map-Reduce)** – Appen känner automatiskt av om en text är för stor för kontextfönstret och väljer då rätt strategi (extrahering eller sammanfattning) utan att du behöver göra något. Under bearbetningen visas realtidsstatus direkt i chattvyn ("Analyserar del 2 av 9…").
* **Resonemang (Thinking)** – Modeller som stödjer intern tankeprocess (som Gemma 4) kan resonera igenom svaret innan de svarar. Ger djupare och mer genomtänkta svar på komplexa frågor. Kan slås av i inställningarna för snabbare konversation.

* **Hårdvaruacceleration med kontroll** – Drar nytta av Vulkan (Windows) eller Metal (macOS). Möjlighet att manuellt välja GPU-index för att tvinga fram rätt grafikkort på t.ex. bärbara datorer.
* **CPU-version för AMD och äldre hårdvara** – En separat CPU-build (utan Vulkan) finns tillgänglig för maskiner där GPU-versionen inte fungerar, t.ex. vissa AMD Radeon-konfigurationer.
* **Dynamiskt kontextstöd** – Justera storleken på AI-minnet (tokens) med en enkel slider för att optimera prestanda vs. RAM.
* **Smart RAM-varning** – Appen beräknar minnesbehovet i realtid och varnar om inställningarna riskerar att överstiga din dators tillgängliga RAM.
* **Sömlösa modellbyten** – Byt AI-modell i farten från sidomenyn, utan att behöva starta om appen.
* **Konfidentiell datahantering** – Bifoga textfiler, PDF:er och bilder direkt i din chatt utan risk för dataläckage.
* **Lokal historik** – Alla konversationer sparas tryggt och krypterat i webbläsarens IndexedDB.
* **Anpassningsbar systemprompt** – Skräddarsy AI:ns personlighet och beteende för varje unik uppgift.
* **Import & Export** – Säkerhetskopiera eller flytta dina konversationer smidigt mellan dina egna enheter.
* **Visuella teman** – Välj mellan ljust, mörkt eller ett terminalinspirerat grönt retro-tema med scanlines.
* **Helt på svenska** – Gränssnittet är skapat och fullt översatt för svenska användare.
* **Portabelt läge** – Kan köras direkt från mappen utan installation (kräver WebView2 på Windows).
* **Inbyggd binärhantering** – Under *Inställningar → System* visas vilken version av llama.cpp-servern som är installerad. Du kan uppdatera till senaste release med ett klick, direkt inifrån appen.

---

## Installation & Felsökning

### macOS: "Appen är skadad och kan inte öppnas"
Eftersom Loki inte är digitalt signerad via Apples betalda utvecklingsprogram kan macOS visa ett felmeddelande om att appen är skadad när den laddas ner via en webbläsare. Detta är en säkerhetsfunktion i Gatekeeper.

För att fixa detta, öppna **Terminalen** och kör följande kommando:

```bash
sudo xattr -rd com.apple.quarantine /Applications/Loki.app
```

*(Om du har flyttat appen till en annan mapp än Applications, justera sökvägen i kommandot).*

### Windows: "SmartScreen förhindrade att en okänd app startades"
Klicka på **"Mer information"** och sedan **"Kör ändå"**. Appen kräver även [WebView2 Runtime](https://developer.microsoft.com/en-us/microsoft-edge/webview2/) (finns oftast redan installerat i Windows 10/11).
