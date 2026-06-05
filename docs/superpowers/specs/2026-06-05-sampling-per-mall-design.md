# Sampling-inställningar per mall — design

**Datum:** 2026-06-05
**Status:** Godkänd design, redo för implementationsplan

## Bakgrund & syfte

Sampling-värdena i sammanfattningsläget är idag hårdkodade per modell
([model-sampling.ts](../../../webui/src/lib/constants/model-sampling.ts)) och system-prompten
styrs av mallen. Användaren har inget sätt att experimentera. Konkret upptäckt: **Gemma 4 med
temperatur 0.5 + en enklare prompt** ("du är expert på att summera möten, ge en kort
minnesanteckning") gav märkbart bättre mötessammanfattningar än de hårdkodade defaults
(Gemma temp 1.0) + den utförliga `COMMON_PREAMBLE`-prompten.

**Mål (bekräftat med användaren):** kunna *labba* med temperatur, system-prompt och
upprepningsstraff för att hitta rätt inställningar per modell — och behålla möjligheten att
finjustera. Labbandet är både ett medel (hitta bra defaults) och en kvarvarande funktion.

## Beslut (alla bekräftade)

- **Parametrar att exponera:** temperatur, system-prompt (= mallens instruktion), samt
  presence_penalty + repeat_penalty. (top_p/top_k exponeras INTE — mindre tydlig effekt,
  modell-defaults är rimliga.)
- **Inställningar hör till mallen.** En mall blir ett komplett "recept": prompt + sampling.
  Återanvändbart och jämförbart.
- **Ett värde per mall** (inte per modell i mallen). Vill man ha olika per modell skapar man
  två mallar.
- **Saknas värde → modellens auto-default** (`samplingForModel`). Bakåtkompatibelt: inget
  ändras för befintliga/inbyggda mallar förrän användaren aktivt sätter värden.
- **Modellbyte:** sker som idag via avancerat läge (ingen modellväljare i sammanfattningsvyn).
- **Resultat-etiketten** visar modell + temperatur (t.ex. "Skapad med Gemma 4 · temp 0.5").

## Arkitektur

### Datamodell
Utöka `SummaryTemplate` ([summary-templates.ts](../../../webui/src/lib/constants/summary-templates.ts))
med ett valfritt `sampling`-fält:
```ts
interface SummaryTemplate {
  // befintligt: id, label, description, systemPrompt, icon?, builtin?
  sampling?: {
    temperature?: number;
    presence_penalty?: number;
    repeat_penalty?: number;
  };
}
```
Allt valfritt. Varje utelämnad parameter ärver modellens default.

### Sammanslagning av sampling
I `summarizeStore.stream()` ([summarize.svelte.ts](../../../webui/src/lib/stores/summarize.svelte.ts))
ersätts `samplingForModel(modell, thinking)` med en sammanslagning:
```
effektiv = { ...samplingForModel(modell, thinking), ...rensa(template.sampling) }
```
`rensa()` tar bort undefined-fält så bara satta värden överstyr. `repeat_penalty` läggs till
i `SamplingProfile`-typen och skickas vidare i `sendMessage`-options. `repeat_penalty` och
`presence_penalty` mappas redan till requestBody i ChatService (verifierat:
[chat.service.ts:236-237](../../../webui/src/lib/services/chat.service.ts#L236-L237)).

### Persistens
Egna mallars `sampling` sparas redan via JSON-serialiseringen i
[summary-templates.svelte.ts](../../../webui/src/lib/stores/summary-templates.svelte.ts)
`persist()`. Utöka `persist()` att inkludera `sampling`-fältet, och `create`/`update` att ta
emot det.

## UI

### Mall-editorn (`TemplateEditorDialog`)
Lägg ett **kollapsat** avsnitt "Avancerat (sampling)" längst ner, efter prompt-fältet:
- **Temperatur** — slider 0–1.5 + sifferfält. Hjälptext: "Lägre = mer förutsägbart, högre = mer kreativt".
- **Upprepningsstraff** (presence_penalty) — 0–2. Hjälptext: "Högre motverkar att modellen upprepar sig".
- **Repetitionsstraff** (repeat_penalty) — 1–1.5. Hjälptext kort.
- Varje reglage har ett "använd modellens standard"-läge (ostyrt = ärver default).
- **"Återställ till modellens standard"**-knapp som tömmer mallens sampling.

Avsnittet är kollapsat som standard (Collapsible-komponenten finns i ui/) så vanliga
användare inte störs.

### Resultat-etikett (`SummarizeScreen`)
Statusraden när klar visar modell + temp: "Skapad med {modell} · temp {värde}". Hämta från den
effektiva sampling som användes (spara den i store-state vid körning, t.ex. `usedTemperature`).

## Labb-arbetsflöde (för verifiering)
1. Duplicera en inbyggd mall (knappen finns) → kopia.
2. Redigera kopian: enklare prompt + öppna Avancerat + sätt temp 0.5.
3. Kör samma transkribering. Etiketten visar "temp 0.5".
4. Justera → kör igen → jämför.
5. Behåll den bästa mallen som standardval.

## Verifiering
1. **Default oförändrat:** en mall utan `sampling` ger exakt samma värden som idag
   (modell-default). Befintliga mallar/historik opåverkade.
2. **Överstyrning:** en mall med `sampling.temperature = 0.5` → requesten skickar temp 0.5
   (verifiera i `llama_server.log` eller via diag), övriga värden = modell-default.
3. **Partiell överstyrning:** bara temperatur satt → presence/repeat = modell-default.
4. **Persistens:** skapa mall med sampling → ladda om appen → värdena finns kvar.
5. **Etikett:** resultatraden visar rätt temp.
6. **Enhetstester:** sampling-sammanslagningen (modell-default + mall-överstyrning + rensa
   undefined). Utöka `tests/unit/model-sampling.test.ts` eller ny testfil.
7. `npm run check && npm run lint && npm run test:unit`; `npm run build`.

## YAGNI / avgränsning
- INTE per-modell-värden i en mall.
- INTE top_p/top_k i UI (default räcker).
- INTE modellväljare i sammanfattningsvyn.
- INTE en separat "avancerat"-panel skild från mallen — allt hör till mallen.
