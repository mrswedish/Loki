import { conversationsStore } from '$lib/stores/conversations.svelte';
import { config } from '$lib/stores/settings.svelte';
import { serverStore } from '$lib/stores/server.svelte';
import { isTauriEnv } from '$lib/server-url';
import { startServer, listAvailableModels, listLocalModels } from '$lib/tauri-bridge';
import { processFilesToChatUploaded } from '$lib/utils/process-uploaded-files';
import { ChatService } from '$lib/services/chat.service';
import { DatabaseService } from '$lib/services/database.service';
import {
	countTokens,
	pickStrategy,
	buildSystemPrompt,
	splitIntoChunks,
	buildMapPrompt,
	isEmptyChunkResponse,
	MAX_CTX,
	RESPONSE_MARGIN,
	RESPONSE_MARGIN_THINKING
} from '$lib/services/summarize.service';
import { LANGUAGE_REFINE_PROMPT, REDUCE_PROMPT } from '$lib/constants/summary-templates';
import {
	samplingForModel,
	mergeSampling,
	type TemplateSampling
} from '$lib/constants/model-sampling';
import type { SummaryTemplate } from '$lib/constants/summary-templates';
import { summaryTemplatesStore } from '$lib/stores/summary-templates.svelte';
import { MessageRole, MessageType } from '$lib/enums';
import type { ApiChatMessageData } from '$lib/types/api';
import type { ChatMessageTimings, ChatMessagePromptProgress } from '$lib/types/chat';
import { toast } from 'svelte-sonner';

/**
 * SummarizeStore – orkestrerar Loki 2.0 sammanfattningsläget.
 *
 * Läser transkriberingen, räknar tokens proaktivt, startar servern med exakt rätt
 * kontext, och streamar sammanfattningen DIREKT via ChatService till egen state
 * (resultatpanelen i sammanfattningsvyn). Sparar resultatet i historiken (Dexie).
 *
 * Två-modell-pipeline: efter Qwen-sammanfattningen kan användaren välja att
 * "Förbättra svenskan" – då byts servern till Gemma 4 E2B som språkrättar och
 * lätt polerar resultatet (utan att skriva om innehållet).
 */

export type SummarizeState = 'idle' | 'reading' | 'preparing' | 'running' | 'done' | 'error';

/** Vilket steg i pipelinen som körs (för UI-text). */
export type SummarizePhase = 'summarizing' | 'refining' | null;

/** Uppskattning som visas för användaren innan körning. */
export interface TranscriptInfo {
	fileName: string;
	chars: number;
	tokens: number;
	strategy: 'single' | 'chunked';
	/** Grov uppskattning av taltid i minuter. */
	approxMinutes: number;
}

/** Gemma-modellens id i registret – används för språkrättningssteget. */
const REFINE_MODEL_ID = 'gemma-4-e2b';

/**
 * Tak för antal reasoning-tokens (thinking) innan vi avbryter och kör om utan
 * thinking. Små modeller (Qwen 4B) kan annars resonera i oändlighet utan att nå
 * svaret. ~2500 ger gott om tankeutrymme men garanterar att vi inte fastnar.
 */
const MAX_THINKING_TOKENS = 2500;

/** Token-budget per bit vid chunkad sammanfattning (Geminis "~20 min"-råd). */
const CHUNK_TOKEN_BUDGET = 8000;

class SummarizeStore {
	state = $state<SummarizeState>('idle');
	phase = $state<SummarizePhase>(null);
	error = $state<string | null>(null);

	transcript = $state<string>('');
	agenda = $state<string>('');
	meetingName = $state<string>('');
	/** "Noggrannare": thinking på (av som standard). När på reserveras större ctx. */
	thorough = $state<boolean>(false);

	info = $state<TranscriptInfo | null>(null);

	/** Streamande/färdigt resultat (markdown). */
	result = $state<string>('');
	/** Namn på modellen som producerade nuvarande result (visas i UI). */
	resultModel = $state<string>('');
	/** Temperatur som faktiskt användes (för resultat-etiketten). */
	usedTemperature = $state<number | null>(null);
	/** True om resultatet redan språkrättats (döljer knappen). */
	refined = $state<boolean>(false);
	/**
	 * Förloppsinfo under körning (för progress-indikatorn i resultatvyn).
	 * promptPercent: prompt-processing 0–100 (innan första token).
	 * generatedTokens + tokensPerSec: under genereringen.
	 */
	promptPercent = $state<number | null>(null);
	generatedTokens = $state<number>(0);
	tokensPerSec = $state<number>(0);
	/** Antal reasoning-tokens (thinking) hittills – visas som "Tänker… X". */
	thinkingTokens = $state<number>(0);
	/** True medan modellen är i thinking-fasen (inget svar ännu). */
	isThinking = $state<boolean>(false);
	/** Förlopp vid chunkad sammanfattning: { current, total } eller null. */
	chunkProgress = $state<{ current: number; total: number } | null>(null);
	/** Konversations-id i historiken för denna körning. */
	private convId: string | null = null;
	/** Db-id för assistant-meddelandet (uppdateras vid språkrättning). */
	private resultMessageId: string | null = null;
	/**
	 * Modellsökväg som användes för sammanfattningen (t.ex. Qwen). Sparas så att vi
	 * kan byta tillbaka till den om språkrättningen bytt servern till Gemma – annars
	 * skulle nästa sammanfattning köras med fel modell.
	 */
	private summaryModelPath: string | null = null;

	get busy(): boolean {
		return this.state === 'reading' || this.state === 'preparing' || this.state === 'running';
	}

	reset(): void {
		this.state = 'idle';
		this.phase = null;
		this.error = null;
		this.transcript = '';
		this.agenda = '';
		this.meetingName = '';
		this.info = null;
		this.result = '';
		this.resultModel = '';
		this.refined = false;
		this.usedTemperature = null;
		this.convId = null;
		this.resultMessageId = null;
		this.resetProgress();
	}

	private resetProgress(): void {
		this.promptPercent = null;
		this.generatedTokens = 0;
		this.tokensPerSec = 0;
		this.thinkingTokens = 0;
		this.isThinking = false;
		this.chunkProgress = null;
	}

	/** Läser en transkriberingsfil, räknar tokens och beräknar uppskattning. */
	async loadTranscript(file: File): Promise<void> {
		this.state = 'reading';
		this.error = null;
		try {
			const [uploaded] = await processFilesToChatUploaded([file]);
			const text = uploaded?.textContent?.trim() ?? '';
			if (!text) throw new Error('Filen verkar tom eller kunde inte läsas som text.');
			this.transcript = text;
			this.meetingName = file.name.replace(/\.[^.]+$/, '').trim();

			let tokens: number;
			try {
				tokens = await countTokens(text);
			} catch {
				tokens = Math.ceil(text.length / 4);
			}

			const margin = this.thorough ? RESPONSE_MARGIN_THINKING : RESPONSE_MARGIN;
			const { strategy } = pickStrategy(tokens, serverStore.contextSize ?? undefined, margin);

			this.info = {
				fileName: file.name,
				chars: text.length,
				tokens,
				strategy,
				approxMinutes: Math.round(tokens / 1.3 / 150)
			};
			this.state = 'idle';
		} catch (e) {
			this.state = 'error';
			this.error = e instanceof Error ? e.message : 'Kunde inte läsa filen.';
			throw e;
		}
	}

	/** Läser en valfri agendafil. */
	async loadAgenda(file: File): Promise<void> {
		try {
			const [uploaded] = await processFilesToChatUploaded([file]);
			this.agenda = uploaded?.textContent?.trim() ?? '';
			if (!this.agenda) toast.info('Agendan verkar tom eller kunde inte läsas.');
		} catch {
			toast.error('Kunde inte läsa agendafilen.');
		}
	}

	clearAgenda(): void {
		this.agenda = '';
	}

	/** Kör sammanfattningen med vald mall och streamar resultatet till `result`. */
	async run(templateId: string, customTemplate?: SummaryTemplate): Promise<void> {
		if (!this.transcript || !this.info) {
			toast.error('Ladda upp en transkribering först.');
			return;
		}
		// Slå upp mallen bland både inbyggda och användarens egna mallar.
		const template = customTemplate ?? summaryTemplatesStore.get(templateId);
		if (!template) {
			toast.error('Okänd mall.');
			return;
		}

		this.state = 'preparing';
		this.phase = 'summarizing';
		this.error = null;
		this.result = '';
		this.refined = false;

		const systemPrompt = buildSystemPrompt(template, this.agenda || undefined);
		const margin = this.thorough ? RESPONSE_MARGIN_THINKING : RESPONSE_MARGIN;

		// Säkerställ rätt sammanfattningsmodell. Om en tidigare körning språkrättades
		// kör servern nu Gemma – byt då tillbaka till sammanfattningsmodellen så att
		// nästa sammanfattning inte oavsiktligt körs med Gemma. Första körningen sparar
		// den modell servern redan kör (användarens val).
		await this.ensureSummaryModel();
		this.resultModel = this.currentModelName();

		// Proaktiv ctx: starta servern med exakt rätt storlek innan anropet.
		await this.ensureContextFor(`${systemPrompt}\n\n${this.transcript}`, margin);

		const messages: ApiChatMessageData[] = [
			{ role: MessageRole.SYSTEM, content: systemPrompt },
			{ role: MessageRole.USER, content: this.transcript }
		];

		try {
			this.state = 'running';
			if (this.info.strategy === 'chunked') {
				await this.runChunked(template);
			} else {
				// Befintligt single-flöde (behåll exakt som det är):
				const first = await this.stream(messages, {
					enableThinking: this.thorough,
					sampling: template.sampling
				});
				let text = first.text;
				// Modellen fastnade i oändligt resonemang – kör om utan thinking så att ett
				// resultat garanteras (annars väntar användaren för evigt).
				if (first.hitThinkingCap) {
					toast.info('Modellen tänkte för länge – kör om utan tankeläge.');
					text = (
						await this.stream(messages, {
							enableThinking: false,
							sampling: template.sampling
						})
					).text;
				}
				await this.saveToHistory(text);
			}
			this.state = 'done';
		} catch (e) {
			this.state = 'error';
			this.error = e instanceof Error ? e.message : 'Sammanfattningen misslyckades.';
			toast.error(this.error);
		} finally {
			this.phase = null;
			this.chunkProgress = null;
		}
	}

	/**
	 * Chunkad map-reduce för transkriberingar som inte ryms i modellens kontext.
	 * Map: dela texten, sammanfatta varje bit. Reduce: slå ihop kronologiskt.
	 * Thinking är alltid AV i map-steget (stabilare/snabbare över många bitar).
	 */
	private async runChunked(template: SummaryTemplate): Promise<void> {
		// Mät tecken-per-token för exakt delning (fallback ~4).
		let charsPerToken = 4;
		try {
			const tokens = await countTokens(this.transcript);
			if (tokens > 0) charsPerToken = this.transcript.length / tokens;
		} catch {
			// behåll fallback
		}

		const chunks = splitIntoChunks(this.transcript, CHUNK_TOKEN_BUDGET, charsPerToken, 200);
		const mapPrompt = buildMapPrompt(template);
		const summaries: string[] = [];

		// MAP: sammanfatta varje bit.
		for (let i = 0; i < chunks.length; i++) {
			this.chunkProgress = { current: i + 1, total: chunks.length };
			this.result = ''; // visa progress, inte föregående bit
			await this.ensureCtxForText(`${mapPrompt}\n\n${chunks[i]}`);
			const messages: ApiChatMessageData[] = [
				{ role: MessageRole.SYSTEM, content: mapPrompt },
				{ role: MessageRole.USER, content: chunks[i] }
			];
			const { text } = await this.stream(messages, {
				enableThinking: false,
				sampling: template.sampling
			});
			const trimmed = text.trim();
			// Hoppa över avsnitt utan relevant innehåll. Modellen kan formulera sig
			// olika ("Inget relevant...", *kursivt*, med citattecken, eller omvänt
			// "...finns inget relevant"), så matcha brett men bara korta svar.
			if (trimmed && !isEmptyChunkResponse(trimmed)) {
				summaries.push(trimmed);
			}
		}

		// Om inget relevant alls hittades.
		if (summaries.length === 0) {
			this.chunkProgress = null;
			this.result = 'Inget relevant innehåll hittades i transkriberingen.';
			await this.saveToHistory(this.result);
			return;
		}

		// REDUCE: slå ihop delsammanfattningarna kronologiskt.
		this.chunkProgress = null;
		this.phase = 'summarizing';
		this.result = '';
		const joined = summaries.join('\n\n---\n\n');
		// Lägg agendan i reduce-steget om sådan finns (map utelämnar den).
		const reduceSystem = this.agenda
			? `${REDUCE_PROMPT}\n\n--- AGENDA ---\n${this.agenda}\n--- SLUT AGENDA ---`
			: REDUCE_PROMPT;

		// Skydd: om delsammanfattningarna tillsammans inte ryms i taket finns ingen
		// reaktiv auto-expand här. Varna användaren (hierarkisk reduce är framtida arbete).
		try {
			const reduceTokens = await countTokens(`${reduceSystem}\n\n${joined}`);
			if (reduceTokens + RESPONSE_MARGIN > MAX_CTX) {
				toast.warning(
					'Mycket lång transkribering – sammanslagningen kan bli ofullständig. ' +
						'Överväg att dela upp inspelningen.'
				);
			}
		} catch {
			// tokenräkning är best-effort
		}

		await this.ensureCtxForText(`${reduceSystem}\n\n${joined}`);
		const reduceMessages: ApiChatMessageData[] = [
			{ role: MessageRole.SYSTEM, content: reduceSystem },
			{ role: MessageRole.USER, content: joined }
		];
		const { text: final } = await this.stream(reduceMessages, {
			enableThinking: false,
			sampling: template.sampling
		});
		await this.saveToHistory(final);
	}

	/**
	 * Säkrar ctx för en given prompt-text (används per bit i chunkad körning).
	 * Liknar ensureContextFor men utan strategy-guarden (chunkad gör egen sizing).
	 */
	private async ensureCtxForText(promptText: string): Promise<void> {
		if (!isTauriEnv()) return;
		try {
			let promptTokens: number;
			try {
				promptTokens = await countTokens(promptText);
			} catch {
				promptTokens = Math.ceil(promptText.length / 4);
			}
			const { ctx } = pickStrategy(
				promptTokens,
				serverStore.contextSize ?? undefined,
				RESPONSE_MARGIN
			);
			const modelPath = serverStore.currentModelPath;
			const gpuIndex = (config().gpuIndex as number) ?? -1;
			if (modelPath && ctx > (serverStore.contextSize ?? 0)) {
				await startServer(modelPath, ctx, gpuIndex);
				serverStore.currentModelPath = modelPath;
				await serverStore.fetch();
			}
		} catch (e) {
			console.warn('[summarize] ctx-sizing (chunk) misslyckades:', e);
		}
	}

	/**
	 * Språkrättar nuvarande resultat med Gemma 4 E2B (bäst på svenska). Byter
	 * server till Gemma, kör en rättnings-prompt på resultattexten (kort input →
	 * snabbt), och ersätter resultatet med den rättade versionen.
	 */
	async refineLanguage(): Promise<void> {
		if (!this.result.trim() || this.busy) return;
		const original = this.result;

		this.state = 'running';
		this.phase = 'refining';
		this.error = null;

		try {
			// Byt till Gemma (om vi inte redan kör den) innan rättningen.
			if (isTauriEnv()) {
				const gemmaPath = await this.resolveModelPath(REFINE_MODEL_ID);
				if (gemmaPath && gemmaPath !== serverStore.currentModelPath) {
					const gpuIndex = (config().gpuIndex as number) ?? -1;
					await startServer(gemmaPath, serverStore.contextSize ?? 4096, gpuIndex);
					serverStore.currentModelPath = gemmaPath;
					await serverStore.fetch();
				}
				// Rättningen läser bara det korta protokollet – säkra ctx för det.
				await this.ensureContextFor(`${LANGUAGE_REFINE_PROMPT}\n\n${original}`, RESPONSE_MARGIN);
			}

			this.result = '';
			this.resultModel = 'Gemma 4 E2B';
			const messages: ApiChatMessageData[] = [
				{ role: MessageRole.SYSTEM, content: LANGUAGE_REFINE_PROMPT },
				{ role: MessageRole.USER, content: original }
			];
			// Gemma är ingen thinking-modell – kör utan.
			const { text: refined } = await this.stream(messages, { enableThinking: false });
			this.refined = true;
			await this.updateHistory(refined);
			this.state = 'done';
		} catch (e) {
			// Återställ originalet om rättningen misslyckas.
			this.result = original;
			this.state = 'done';
			this.error = e instanceof Error ? e.message : 'Språkrättningen misslyckades.';
			toast.error(this.error);
		} finally {
			this.phase = null;
		}
	}

	/**
	 * Streamar en chat completion till `this.result` och returnerar full text.
	 * Strippar råa think-taggar löpande (vissa modeller läcker dem).
	 */
	private async stream(
		messages: ApiChatMessageData[],
		opts: { enableThinking: boolean; sampling?: TemplateSampling }
	): Promise<{ text: string; hitThinkingCap: boolean }> {
		// Välj modell-anpassad sampling. En enda hårdkodad temperatur fungerar dåligt
		// (Qwen utan presence_penalty upprepar sig och spårar ur). Profilen väljs
		// utifrån vilken modell servern kör + thinking-läge.
		const base = samplingForModel(serverStore.currentModelPath, opts.enableThinking);
		const sampling = mergeSampling(base, opts.sampling);
		this.usedTemperature = sampling.temperature;
		this.resetProgress();

		// Tak för thinking: små modeller (Qwen 4B) kan annars resonera i oändlighet
		// och aldrig nå svaret. Når reasoning-tokens taket avbryter vi och anroparen
		// kör om utan thinking så ett resultat garanteras.
		const controller = new AbortController();
		let reasoningChars = 0;
		let hitThinkingCap = false;
		let acc = '';

		try {
			await ChatService.sendMessage(
				messages,
				{
					stream: true,
					enableThinking: opts.enableThinking,
					max_tokens: -1,
					temperature: sampling.temperature,
					top_p: sampling.top_p,
					top_k: sampling.top_k,
					min_p: sampling.min_p,
					presence_penalty: sampling.presence_penalty,
					repeat_penalty: sampling.repeat_penalty,
					onChunk: (chunk: string) => {
						// Första content-token → tänkandet är klart.
						this.isThinking = false;
						acc += chunk;
						this.result = ChatService.stripRawThinkTags(acc);
					},
					onReasoningChunk: (chunk: string) => {
						this.isThinking = true;
						reasoningChars += chunk.length;
						// Uppskatta tokens (~4 tecken/token) för statusvisningen.
						this.thinkingTokens = Math.round(reasoningChars / 4);
						if (this.thinkingTokens >= MAX_THINKING_TOKENS && !controller.signal.aborted) {
							hitThinkingCap = true;
							controller.abort();
						}
					},
					onTimings: (timings?: ChatMessageTimings, promptProgress?: ChatMessagePromptProgress) => {
						if (promptProgress && promptProgress.total > 0) {
							this.promptPercent = Math.min(
								100,
								Math.round((promptProgress.processed / promptProgress.total) * 100)
							);
						}
						if (timings?.predicted_n) {
							this.promptPercent = null;
							this.generatedTokens = timings.predicted_n;
							if (timings.predicted_ms) {
								this.tokensPerSec = (timings.predicted_n / timings.predicted_ms) * 1000;
							}
						}
					},
					onError: (err: Error) => {
						throw err;
					}
				},
				undefined,
				controller.signal
			);
		} catch (e) {
			// Abort pga thinking-taket är förväntat – svälj det, anroparen kör om.
			if (!hitThinkingCap) throw e;
		}

		this.isThinking = false;
		const final = ChatService.stripRawThinkTags(acc).trim();
		this.result = final;
		return { text: final, hitThinkingCap };
	}

	/**
	 * Sparar körningen som en konversation i historiken (transkribering + resultat).
	 * Använder DatabaseService direkt (INTE conversationsStore.createConversation, som
	 * navigerar till /chat/[id]) så att vi stannar kvar i sammanfattningsvyn.
	 */
	private async saveToHistory(output: string): Promise<void> {
		try {
			const title = this.meetingName.trim() || 'Sammanfattning';
			const conv = await DatabaseService.createConversation(title);
			this.convId = conv.id;
			const rootId = await DatabaseService.createRootMessage(conv.id);
			const userMsg = await DatabaseService.createMessageBranch(
				{
					convId: conv.id,
					role: MessageRole.USER,
					content: this.transcript,
					type: MessageType.TEXT,
					timestamp: Date.now(),
					toolCalls: '',
					children: [],
					extra: undefined
				},
				rootId
			);
			const assistantMsg = await DatabaseService.createMessageBranch(
				{
					convId: conv.id,
					role: MessageRole.ASSISTANT,
					content: output,
					type: MessageType.TEXT,
					timestamp: Date.now(),
					toolCalls: '',
					children: [],
					extra: undefined
				},
				userMsg.id
			);
			this.resultMessageId = assistantMsg.id;
			// Peka konversationens currNode på resultatet så den öppnas korrekt från historiken.
			await DatabaseService.updateConversation(conv.id, { currNode: assistantMsg.id });
			// Uppdatera sidofältet utan att navigera.
			await conversationsStore.loadConversations();
		} catch (e) {
			console.warn('[summarize] kunde inte spara i historik:', e);
		}
	}

	/** Uppdaterar det sparade resultatmeddelandet (efter språkrättning). */
	private async updateHistory(output: string): Promise<void> {
		if (!this.resultMessageId) return;
		try {
			await DatabaseService.updateMessage(this.resultMessageId, { content: output });
		} catch (e) {
			console.warn('[summarize] kunde inte uppdatera historik:', e);
		}
	}

	/**
	 * Säkerställer att servern kör sammanfattningsmodellen (inte Gemma från ett
	 * tidigare språkrättningssteg). Första körningen sparar användarens valda modell.
	 */
	private async ensureSummaryModel(): Promise<void> {
		if (!isTauriEnv()) return;
		const current = serverStore.currentModelPath;
		// Första körningen: kom ihåg den modell servern redan kör (användarens val).
		if (!this.summaryModelPath) {
			this.summaryModelPath = current;
			return;
		}
		// Servern kör en annan modell (t.ex. Gemma efter rättning) – byt tillbaka.
		if (current !== this.summaryModelPath) {
			try {
				const gpuIndex = (config().gpuIndex as number) ?? -1;
				await startServer(this.summaryModelPath, serverStore.contextSize ?? 4096, gpuIndex);
				serverStore.currentModelPath = this.summaryModelPath;
				await serverStore.fetch();
			} catch (e) {
				console.warn('[summarize] kunde inte byta tillbaka till sammanfattningsmodellen:', e);
			}
		}
	}

	/** Säkrar att servern kör med tillräcklig kontext för en given prompt-text. */
	private async ensureContextFor(promptText: string, margin: number): Promise<void> {
		if (!isTauriEnv() || this.info?.strategy === 'chunked') return;
		try {
			let promptTokens: number;
			try {
				promptTokens = await countTokens(promptText);
			} catch {
				promptTokens = Math.ceil(promptText.length / 4);
			}
			const { ctx } = pickStrategy(promptTokens, serverStore.contextSize ?? undefined, margin);
			const modelPath = serverStore.currentModelPath;
			const gpuIndex = (config().gpuIndex as number) ?? -1;
			if (modelPath && ctx > (serverStore.contextSize ?? 0)) {
				await startServer(modelPath, ctx, gpuIndex);
				serverStore.currentModelPath = modelPath;
				await serverStore.fetch();
			}
		} catch (e) {
			// Proaktiv sizing är en optimering – faller tillbaka på reaktiv auto-expand.
			console.warn('[summarize] proaktiv ctx-sizing misslyckades:', e);
		}
	}

	/** Hittar den lokala sökvägen för en registermodell (via filnamn). */
	private async resolveModelPath(modelId: string): Promise<string | null> {
		try {
			const [registry, local] = await Promise.all([listAvailableModels(), listLocalModels()]);
			const entry = registry.find((m) => m.id === modelId);
			if (!entry) return null;
			return local.find((l) => l.filename === entry.filename)?.path ?? null;
		} catch {
			return null;
		}
	}

	/** Namn på modellen som servern just nu kör (för resultatetikett). */
	private currentModelName(): string {
		const path = serverStore.currentModelPath ?? '';
		const file = path.split(/[\\/]/).pop() ?? '';
		if (/qwen/i.test(file)) return 'Qwen3.5';
		if (/gemma/i.test(file)) return 'Gemma 4';
		return file.replace(/\.gguf$/i, '') || 'Modell';
	}
}

export const summarizeStore = new SummarizeStore();
