# Sampling-inställningar per mall — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Låt en sammanfattningsmall bära egna sampling-värden (temperatur, presence_penalty, repeat_penalty) som överstyr modellens defaults, så användaren kan labba fram bästa inställningar per mall.

**Architecture:** Utöka `SummaryTemplate` med ett valfritt `sampling`-fält. I `summarizeStore.stream()` slås modellens default (`samplingForModel`) ihop med mallens överstyrningar. Mall-editorn får ett kollapsat "Avancerat"-avsnitt med reglage. Resultat-etiketten visar använd temperatur. Bakåtkompatibelt: mallar utan `sampling` beter sig exakt som idag.

**Tech Stack:** SvelteKit (Svelte 5 runes), TypeScript, Vitest, Tailwind, shadcn-svelte (Collapsible, Input, Label, Button).

**Arbetskatalog:** `webui/` (alla npm-kommandon körs där: `cd webui`).

---

## Filöversikt

- **Modify** `webui/src/lib/constants/model-sampling.ts` — lägg `repeat_penalty` i `SamplingProfile` + alla profiler; ny `mergeSampling()`.
- **Modify** `webui/src/lib/constants/summary-templates.ts` — `sampling`-fält + `TemplateSampling`-typ på `SummaryTemplate`.
- **Modify** `webui/src/lib/stores/summary-templates.svelte.ts` — `create`/`update`/`persist` hanterar `sampling`.
- **Modify** `webui/src/lib/stores/summarize.svelte.ts` — `stream()` slår ihop sampling; spara `usedTemperature`; skicka mallens sampling.
- **Modify** `webui/src/lib/components/app/summarize/TemplateEditorDialog.svelte` — Avancerat-avsnitt med reglage.
- **Modify** `webui/src/lib/components/app/summarize/SummarizeScreen.svelte` — etikett visar "· temp X".
- **Test** `webui/tests/unit/model-sampling.test.ts` — utöka med `mergeSampling`-tester.

---

## Task 1: Lägg repeat_penalty + mergeSampling i model-sampling.ts

**Files:**
- Modify: `webui/src/lib/constants/model-sampling.ts`
- Test: `webui/tests/unit/model-sampling.test.ts`

- [ ] **Step 1: Skriv de felande testerna**

Uppdatera FÖRST den befintliga importraden överst i `webui/tests/unit/model-sampling.test.ts`
så `mergeSampling` ingår:

```ts
import {
	samplingForModel,
	mergeSampling,
	QWEN_INSTRUCT,
	QWEN_THINKING,
	GEMMA_DEFAULT,
	GENERIC_DEFAULT
} from '$lib/constants/model-sampling';
```

Lägg sedan till detta `describe`-block efter befintlig `describe('samplingForModel'...)`:

```ts
describe('mergeSampling', () => {
	it('returnerar modell-defaulten oförändrad när mallen saknar sampling', () => {
		const base = samplingForModel('Qwen3.5-4B.gguf', false);
		expect(mergeSampling(base, undefined)).toEqual(base);
		expect(mergeSampling(base, {})).toEqual(base);
	});

	it('låter mallens satta värden överstyra', () => {
		const base = samplingForModel('google_gemma-4.gguf', false); // temp 1.0
		const merged = mergeSampling(base, { temperature: 0.5 });
		expect(merged.temperature).toBe(0.5);
		expect(merged.top_p).toBe(base.top_p); // oförändrat
	});

	it('ignorerar undefined-fält i mallens sampling', () => {
		const base = samplingForModel('Qwen3.5-4B.gguf', false);
		const merged = mergeSampling(base, { temperature: undefined, presence_penalty: 0.5 });
		expect(merged.temperature).toBe(base.temperature);
		expect(merged.presence_penalty).toBe(0.5);
	});

	it('hanterar repeat_penalty-överstyrning', () => {
		const base = samplingForModel('google_gemma-4.gguf', false);
		const merged = mergeSampling(base, { repeat_penalty: 1.2 });
		expect(merged.repeat_penalty).toBe(1.2);
	});
});
```

Korrigera importraden överst i testfilen så `mergeSampling` importeras från `$lib/constants/model-sampling` (INTE från summarize.service — det var ett fel i exempelkommentaren ovan).

- [ ] **Step 2: Kör testet, verifiera att det failar**

Run: `cd webui && npm run test:unit -- model-sampling`
Expected: FAIL — `mergeSampling is not a function` / `repeat_penalty` saknas.

- [ ] **Step 3: Implementera i model-sampling.ts**

I `webui/src/lib/constants/model-sampling.ts`, lägg `repeat_penalty` i interfacet och alla profiler, samt `TemplateSampling` + `mergeSampling`:

```ts
export interface SamplingProfile {
	temperature: number;
	top_p: number;
	top_k: number;
	min_p: number;
	presence_penalty: number;
	repeat_penalty: number;
}
```

Lägg `repeat_penalty` i varje profil:
- `QWEN_INSTRUCT`, `QWEN_THINKING`: `repeat_penalty: 1.0`
- `GEMMA_DEFAULT`: `repeat_penalty: 1.0`
- `GENERIC_DEFAULT`: `repeat_penalty: 1.0`

Lägg sist i filen:

```ts
/** Per-mall överstyrningar av sampling. Alla fält valfria; utelämnade ärver modell-default. */
export interface TemplateSampling {
	temperature?: number;
	presence_penalty?: number;
	repeat_penalty?: number;
}

/**
 * Slår ihop modellens default-profil med en malls valfria överstyrningar.
 * Endast definierade fält i `override` vinner; undefined ignoreras.
 */
export function mergeSampling(
	base: SamplingProfile,
	override?: TemplateSampling
): SamplingProfile {
	if (!override) return base;
	const clean: Partial<SamplingProfile> = {};
	if (override.temperature !== undefined) clean.temperature = override.temperature;
	if (override.presence_penalty !== undefined) clean.presence_penalty = override.presence_penalty;
	if (override.repeat_penalty !== undefined) clean.repeat_penalty = override.repeat_penalty;
	return { ...base, ...clean };
}
```

- [ ] **Step 4: Kör testet, verifiera PASS**

Run: `cd webui && npm run test:unit -- model-sampling`
Expected: PASS (alla mergeSampling- och samplingForModel-tester gröna).

- [ ] **Step 5: Commit**

```bash
git add webui/src/lib/constants/model-sampling.ts webui/tests/unit/model-sampling.test.ts
git commit -m "feat(2.0): mergeSampling + repeat_penalty i sampling-profiler"
```

---

## Task 2: Lägg sampling-fält på SummaryTemplate

**Files:**
- Modify: `webui/src/lib/constants/summary-templates.ts`

- [ ] **Step 1: Lägg fältet på interfacet**

I `webui/src/lib/constants/summary-templates.ts`, lägg överst en import och fält i `SummaryTemplate`:

```ts
import type { TemplateSampling } from '$lib/constants/model-sampling';
```

Lägg i interfacet `SummaryTemplate` (efter `builtin?`):

```ts
	/** Valfria sampling-överstyrningar (temperatur m.m.). Saknas → modellens default. */
	sampling?: TemplateSampling;
```

- [ ] **Step 2: Verifiera typkontroll**

Run: `cd webui && npm run check`
Expected: Inga NYA fel i summary-templates.ts (6 pre-existerande fel i andra filer är OK).

- [ ] **Step 3: Commit**

```bash
git add webui/src/lib/constants/summary-templates.ts
git commit -m "feat(2.0): sampling-fält på SummaryTemplate"
```

---

## Task 3: Persistera sampling i mall-storen

**Files:**
- Modify: `webui/src/lib/stores/summary-templates.svelte.ts`

- [ ] **Step 1: Uppdatera create/update/persist att hantera sampling**

I `webui/src/lib/stores/summary-templates.svelte.ts`:

Importera typen överst:
```ts
import type { TemplateSampling } from '$lib/constants/model-sampling';
```

Ändra `persist` så `sampling` tas med:
```ts
	private persist(templates: SummaryTemplate[]): void {
		const plain = templates.map((t) => ({
			id: t.id,
			label: t.label,
			description: t.description,
			systemPrompt: t.systemPrompt,
			builtin: false,
			sampling: t.sampling
		}));
		settingsStore.updateConfig('summaryTemplates', JSON.stringify(plain));
	}
```

Ändra `create`-signaturen och kroppen:
```ts
	create(data: {
		label: string;
		description: string;
		systemPrompt: string;
		sampling?: TemplateSampling;
	}): string {
		const id = newId();
		const template: SummaryTemplate = {
			id,
			label: data.label.trim() || 'Egen mall',
			description: data.description.trim(),
			systemPrompt: data.systemPrompt.trim(),
			builtin: false,
			sampling: data.sampling
		};
		this.persist([...this.custom, template]);
		return id;
	}
```

Ändra `update`-signaturen och kroppen:
```ts
	update(
		id: string,
		data: {
			label: string;
			description: string;
			systemPrompt: string;
			sampling?: TemplateSampling;
		}
	): void {
		const next = this.custom.map((t) =>
			t.id === id
				? {
						...t,
						label: data.label.trim() || t.label,
						description: data.description.trim(),
						systemPrompt: data.systemPrompt.trim(),
						sampling: data.sampling
					}
				: t
		);
		this.persist(next);
	}
```

I `duplicate`, ta med källans sampling:
```ts
	duplicate(id: string): string | null {
		const source = this.get(id);
		if (!source) return null;
		return this.create({
			label: `${source.label} (kopia)`,
			description: source.description,
			systemPrompt: source.systemPrompt,
			sampling: source.sampling
		});
	}
```

I getter `custom`, säkerställ att inlästa `sampling` följer med (mappningen där sätter `builtin: false` – lägg till `sampling: t.sampling`):
```ts
			return parsed
				.filter((t) => t && typeof t.id === 'string' && typeof t.systemPrompt === 'string')
				.map((t) => ({ ...t, builtin: false }) as SummaryTemplate);
```
(`{ ...t }` bevarar redan `sampling` – ingen ändring krävs om spread används. Verifiera att raden använder `...t`.)

- [ ] **Step 2: Verifiera typkontroll**

Run: `cd webui && npm run check`
Expected: Inga nya fel i summary-templates.svelte.ts.

- [ ] **Step 3: Commit**

```bash
git add webui/src/lib/stores/summary-templates.svelte.ts
git commit -m "feat(2.0): persistera mallars sampling-överstyrningar"
```

---

## Task 4: Använd mallens sampling i stream()

**Files:**
- Modify: `webui/src/lib/stores/summarize.svelte.ts`

- [ ] **Step 1: Lägg usedTemperature-state + skicka sampling till stream**

I `webui/src/lib/stores/summarize.svelte.ts`:

Uppdatera importen:
```ts
import { samplingForModel, mergeSampling } from '$lib/constants/model-sampling';
```

Lägg ett state-fält (nära `resultModel`):
```ts
	/** Temperatur som faktiskt användes (för resultat-etiketten). */
	usedTemperature = $state<number | null>(null);
```

Nollställ i `reset()` (lägg till raden):
```ts
		this.usedTemperature = null;
```

Ändra `stream()`-signaturen att ta emot mallens sampling och använd `mergeSampling`:
```ts
	private async stream(
		messages: ApiChatMessageData[],
		opts: { enableThinking: boolean; sampling?: TemplateSampling }
	): Promise<{ text: string; hitThinkingCap: boolean }> {
		const base = samplingForModel(serverStore.currentModelPath, opts.enableThinking);
		const sampling = mergeSampling(base, opts.sampling);
		this.usedTemperature = sampling.temperature;
		this.resetProgress();
		// ... resten oförändrad, men lägg till repeat_penalty i sendMessage-options:
```

I `ChatService.sendMessage`-options inuti `stream()`, lägg till efter `presence_penalty`-raden:
```ts
				presence_penalty: sampling.presence_penalty,
				repeat_penalty: sampling.repeat_penalty,
```

Importera typen överst:
```ts
import { samplingForModel, mergeSampling, type TemplateSampling } from '$lib/constants/model-sampling';
```

- [ ] **Step 2: Skicka mallens sampling från run()**

I `run()`, där `template` slagits upp, skicka `template.sampling` till båda stream-anropen:
```ts
			const first = await this.stream(messages, {
				enableThinking: this.thorough,
				sampling: template.sampling
			});
			let text = first.text;
			if (first.hitThinkingCap) {
				toast.info('Modellen tänkte för länge – kör om utan tankeläge.');
				text = (
					await this.stream(messages, {
						enableThinking: false,
						sampling: template.sampling
					})
				).text;
			}
```

I `refineLanguage()` lämnas stream-anropet utan `sampling` (Gemma-rättning använder modell-default):
```ts
			const { text: refined } = await this.stream(messages, { enableThinking: false });
```
(Ingen ändring krävs där – `sampling` är valfri och utelämnas.)

- [ ] **Step 3: Verifiera typkontroll + befintliga tester**

Run: `cd webui && npm run check && npm run test:unit`
Expected: Inga nya check-fel; 149 tester passerar (befintliga + Task 1).

- [ ] **Step 4: Commit**

```bash
git add webui/src/lib/stores/summarize.svelte.ts
git commit -m "feat(2.0): stream() använder mallens sampling-överstyrningar"
```

---

## Task 5: Avancerat-avsnitt i mall-editorn

**Files:**
- Modify: `webui/src/lib/components/app/summarize/TemplateEditorDialog.svelte`

- [ ] **Step 1: Lägg sampling-state och fyll-i-logik**

I `<script>` i `webui/src/lib/components/app/summarize/TemplateEditorDialog.svelte`:

Lägg importer:
```ts
	import * as Collapsible from '$lib/components/ui/collapsible';
	import { ChevronDown } from '@lucide/svelte';
```

Lägg state (efter `systemPrompt`):
```ts
	// Sampling-överstyrningar. Tomt fält = ärv modellens default.
	let temperature = $state('');
	let presencePenalty = $state('');
	let repeatPenalty = $state('');
	let advancedOpen = $state(false);
```

I `$effect`-blocket som fyller fälten när dialogen öppnas, lägg:
```ts
		if (open) {
			const existing = editId ? summaryTemplatesStore.get(editId) : undefined;
			label = existing?.label ?? '';
			description = existing?.description ?? '';
			systemPrompt = existing?.systemPrompt ?? '';
			temperature = existing?.sampling?.temperature?.toString() ?? '';
			presencePenalty = existing?.sampling?.presence_penalty?.toString() ?? '';
			repeatPenalty = existing?.sampling?.repeat_penalty?.toString() ?? '';
			advancedOpen = !!existing?.sampling;
		}
```

Lägg en hjälpfunktion som bygger sampling-objektet (tomma/ogiltiga fält → utelämnas):
```ts
	function buildSampling(): import('$lib/constants/model-sampling').TemplateSampling | undefined {
		const num = (s: string) => {
			const n = parseFloat(s.replace(',', '.'));
			return Number.isFinite(n) ? n : undefined;
		};
		const s = {
			temperature: num(temperature),
			presence_penalty: num(presencePenalty),
			repeat_penalty: num(repeatPenalty)
		};
		const hasAny =
			s.temperature !== undefined ||
			s.presence_penalty !== undefined ||
			s.repeat_penalty !== undefined;
		return hasAny ? s : undefined;
	}

	function resetSampling() {
		temperature = '';
		presencePenalty = '';
		repeatPenalty = '';
	}
```

Uppdatera `save()` att skicka sampling:
```ts
	function save() {
		if (!systemPrompt.trim()) {
			toast.error('Skriv en instruktion för mallen.');
			return;
		}
		const data = { label, description, systemPrompt, sampling: buildSampling() };
		const id = editId
			? (summaryTemplatesStore.update(editId, data), editId)
			: summaryTemplatesStore.create(data);
		open = false;
		onSaved?.(id);
	}
```

- [ ] **Step 2: Lägg Avancerat-avsnittet i markupen**

I `webui/src/lib/components/app/summarize/TemplateEditorDialog.svelte`, efter prompt-fältets `<div class="space-y-1.5">...</div>` (det med Textarea) och före `</div>` som stänger `space-y-4 py-2`:

```svelte
				<Collapsible.Root bind:open={advancedOpen}>
					<Collapsible.Trigger
						class="text-muted-foreground hover:text-foreground flex items-center gap-1.5 text-sm"
					>
						<ChevronDown
							class={'size-4 transition-transform ' + (advancedOpen ? 'rotate-180' : '')}
						/>
						Avancerat (sampling)
					</Collapsible.Trigger>
					<Collapsible.Content class="space-y-4 pt-3">
						<div class="space-y-1.5">
							<Label for="tpl-temp">Temperatur</Label>
							<Input id="tpl-temp" bind:value={temperature} placeholder="modellens standard" />
							<p class="text-muted-foreground text-xs">
								Lägre = mer förutsägbart och troget källan, högre = mer kreativt. Lämna tomt för
								modellens standard. Typiskt 0.3–0.8 för protokoll.
							</p>
						</div>
						<div class="space-y-1.5">
							<Label for="tpl-presence">Upprepningsstraff</Label>
							<Input
								id="tpl-presence"
								bind:value={presencePenalty}
								placeholder="modellens standard"
							/>
							<p class="text-muted-foreground text-xs">
								Högre värde (0–2) motverkar att modellen upprepar sig.
							</p>
						</div>
						<div class="space-y-1.5">
							<Label for="tpl-repeat">Repetitionsstraff</Label>
							<Input id="tpl-repeat" bind:value={repeatPenalty} placeholder="modellens standard" />
							<p class="text-muted-foreground text-xs">
								Alternativt straff mot upprepning (typiskt 1.0–1.3).
							</p>
						</div>
						<Button variant="ghost" size="sm" class="text-xs" onclick={resetSampling}>
							Återställ till modellens standard
						</Button>
					</Collapsible.Content>
				</Collapsible.Root>
```

- [ ] **Step 3: Verifiera typkontroll + lint + bygg**

Run: `cd webui && npm run check && npm run lint && npm run build`
Expected: Inga nya check-fel; lint rent på filen; bygget lyckas.

- [ ] **Step 4: Commit**

```bash
git add webui/src/lib/components/app/summarize/TemplateEditorDialog.svelte
git commit -m "feat(2.0): avancerat sampling-avsnitt i mall-editorn"
```

---

## Task 6: Visa temperatur i resultat-etiketten

**Files:**
- Modify: `webui/src/lib/components/app/summarize/SummarizeScreen.svelte`

- [ ] **Step 1: Lägg temp i "Skapad med"-raden**

I `webui/src/lib/components/app/summarize/SummarizeScreen.svelte`, hitta raden i statusrubriken:
```svelte
						Skapad med {summarizeStore.resultModel}{summarizeStore.refined ? ' · språkrättad' : ''}
```
Ersätt med (visar temp om känd):
```svelte
						Skapad med {summarizeStore.resultModel}{summarizeStore.usedTemperature !== null
							? ` · temp ${summarizeStore.usedTemperature}`
							: ''}{summarizeStore.refined ? ' · språkrättad' : ''}
```

- [ ] **Step 2: Verifiera bygge**

Run: `cd webui && npm run check && npm run build`
Expected: Inga nya fel; bygget lyckas.

- [ ] **Step 3: Commit**

```bash
git add webui/src/lib/components/app/summarize/SummarizeScreen.svelte
git commit -m "feat(2.0): resultat-etikett visar använd temperatur"
```

---

## Task 7: Slutverifiering + testbygge

- [ ] **Step 1: Full verifiering**

Run: `cd webui && npm run check && npm run lint && npm run test:unit`
Expected: 6 pre-existerande check-fel (ChatSettings + TauriModelSelector), lint rent på ändrade filer, alla unit-tester gröna.

- [ ] **Step 2: Bumpa version till 2.4.0**

Uppdatera versionen på fyra ställen (`package.json`, `webui/package.json`, `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`) + `cargo update -p loki --precise 2.4.0`.

- [ ] **Step 3: Commit + tagg → trigga testbygge**

```bash
git add -A
git commit -m "chore: version 2.4.0 (sampling per mall)"
git push
git tag v2.4.0
git push origin v2.4.0
```

- [ ] **Step 4: Manuell verifiering (användaren, i byggd app)**

1. Duplicera "Mötesprotokoll" → öppna kopian → Avancerat → sätt Temperatur 0.5 → Spara.
2. Kör en transkribering med kopian. Verifiera att resultat-etiketten visar "· temp 0.5".
3. Kör en INBYGGD mall (utan sampling) → etiketten visar ingen temp-överstyrning (eller modellens default-temp), och resultatet är som tidigare.
4. Ladda om appen → den egna mallens temp-värde finns kvar.
5. (Valfritt) Verifiera i `llama_server.log`/diag att temp 0.5 faktiskt skickades.

---

## Självgranskning (ifylld av planförfattaren)

- **Spec-täckning:** Datamodell (Task 2), sammanslagning (Task 1+4), persistens (Task 3), UI-avsnitt
  (Task 5), etikett (Task 6), verifiering (Task 7). Alla spec-krav täckta.
- **Inga top_p/top_k i UI** (spec YAGNI) — bekräftat, bara temp + 2 penalties exponeras.
- **Bakåtkompatibelt:** `mergeSampling(base, undefined)` returnerar `base` (Task 1-test), mallar utan
  `sampling` oförändrade.
- **Typkonsistens:** `TemplateSampling` definieras i Task 1, används i Task 2/3/4/5. `mergeSampling`,
  `repeat_penalty`, `usedTemperature` konsekvent namngivna genom planen.
