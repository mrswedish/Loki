import { chatStore } from '$lib/stores/chat.svelte';
import { conversationsStore } from '$lib/stores/conversations.svelte';
import { settingsStore, config } from '$lib/stores/settings.svelte';
import { serverStore } from '$lib/stores/server.svelte';
import { isTauriEnv } from '$lib/server-url';
import { startServer } from '$lib/tauri-bridge';
import { processFilesToChatUploaded } from '$lib/utils/process-uploaded-files';
import { countTokens, pickStrategy, buildSystemPrompt } from '$lib/services/summarize.service';
import { getBuiltinTemplate, type SummaryTemplate } from '$lib/constants/summary-templates';
import { toast } from 'svelte-sonner';

/**
 * SummarizeStore – orkestrerar Loki 2.0 sammanfattningsläget.
 *
 * Tunt lager ovanpå den befintliga chatt-motorn (chatStore.sendMessage). Det
 * läser den uppladdade transkriberingen, räknar tokens proaktivt, startar om
 * servern med exakt rätt kontextstorlek (undviker overflow), bygger system-
 * prompten från vald mall + valfri agenda, och kör sammanfattningen. Resultatet
 * visas och sparas av chatt-motorn (navigerar till /chat/[id]).
 *
 * Steg 1 stödjer "single"-strategin. Chunkad map-reduce (för texter större än
 * modellens kontext) hanteras separat i ett senare steg.
 */

export type SummarizeState = 'idle' | 'reading' | 'preparing' | 'running' | 'error';

/** Uppskattning som visas för användaren innan körning. */
export interface TranscriptInfo {
	fileName: string;
	chars: number;
	tokens: number;
	/** Strategin som kommer användas. */
	strategy: 'single' | 'chunked';
	/** Grov uppskattning av taltid i minuter (≈ tokens / 150 ord-per-min / ~1.3 tokens-per-ord). */
	approxMinutes: number;
}

class SummarizeStore {
	state = $state<SummarizeState>('idle');
	error = $state<string | null>(null);

	/** Inläst transkriberingstext. */
	transcript = $state<string>('');
	/** Inläst agendatext (valfri). */
	agenda = $state<string>('');
	/** Namn på mötet – blir konversationens titel i historiken. */
	meetingName = $state<string>('');

	info = $state<TranscriptInfo | null>(null);

	/** True medan en sammanfattning förbereds/körs. */
	get busy(): boolean {
		return this.state === 'reading' || this.state === 'preparing' || this.state === 'running';
	}

	reset(): void {
		this.state = 'idle';
		this.error = null;
		this.transcript = '';
		this.agenda = '';
		this.meetingName = '';
		this.info = null;
	}

	/**
	 * Läser en uppladdad transkriberingsfil (.txt/.pdf) till text, räknar tokens
	 * och beräknar en uppskattning som visas innan körning.
	 */
	async loadTranscript(file: File): Promise<void> {
		this.state = 'reading';
		this.error = null;
		try {
			const [uploaded] = await processFilesToChatUploaded([file]);
			const text = uploaded?.textContent?.trim() ?? '';
			if (!text) {
				throw new Error('Filen verkar tom eller kunde inte läsas som text.');
			}
			this.transcript = text;
			// Förifyll mötesnamn med filnamnet utan ändelse – användaren kan ändra.
			this.meetingName = file.name.replace(/\.[^.]+$/, '').trim();

			// Räkna tokens proaktivt (kräver att servern är igång). Faller tillbaka
			// till teckenbaserad uppskattning om /tokenize inte är tillgänglig.
			let tokens: number;
			try {
				tokens = await countTokens(text);
			} catch {
				tokens = Math.ceil(text.length / 4);
			}

			const modelMax = serverStore.contextSize ?? undefined;
			const { strategy } = pickStrategy(tokens, modelMax ?? undefined);

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

	/** Läser en valfri agendafil (.txt/.pdf) till text. */
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

	/**
	 * Kör sammanfattningen med vald mall. Sätter rätt kontext proaktivt och
	 * kör via chatt-motorn (som visar och sparar resultatet).
	 *
	 * @param templateId  Id för vald mall (inbyggd eller egen via customTemplate).
	 * @param customTemplate  En egen mall som inte finns i registret (valfri).
	 */
	async run(templateId: string, customTemplate?: SummaryTemplate): Promise<void> {
		if (!this.transcript || !this.info) {
			toast.error('Ladda upp en transkribering först.');
			return;
		}
		const template = customTemplate ?? getBuiltinTemplate(templateId);
		if (!template) {
			toast.error('Okänd mall.');
			return;
		}

		this.state = 'preparing';
		this.error = null;

		const systemPrompt = buildSystemPrompt(template, this.agenda || undefined);

		// Proaktiv ctx: starta servern med exakt rätt storlek innan anropet (bara
		// i Tauri och bara för single-strategin). Då slipper vi overflow + omstart.
		try {
			if (isTauriEnv() && this.info.strategy === 'single') {
				const promptTokens = await this.tokensWithPrompt(systemPrompt);
				const { ctx } = pickStrategy(promptTokens, serverStore.contextSize ?? undefined);
				const modelPath = serverStore.currentModelPath;
				const gpuIndex = (config().gpuIndex as number) ?? -1;
				if (modelPath && ctx > (serverStore.contextSize ?? 0)) {
					await startServer(modelPath, ctx, gpuIndex);
					serverStore.currentModelPath = modelPath;
					await serverStore.fetch();
				}
			}
		} catch (e) {
			// Proaktiv sizing är en optimering – misslyckas den faller vi tillbaka
			// på chatt-motorns reaktiva auto-expand. Logga och fortsätt.
			console.warn('[summarize] proaktiv ctx-sizing misslyckades:', e);
		}

		// Kör via chatt-motorn. Sätt mallens system-prompt OCH stäng av thinking
		// temporärt (sendMessage läser config synkront vid ny konversation) och återställ.
		//
		// Thinking måste vara AV för sammanfattning: thinking-modeller (t.ex. Qwen3.5)
		// kan annars förbruka hela kontexten på resonemang och aldrig nå svaret.
		const previousSystem = config().systemMessage;
		const previousThinking = config().enableThinking;
		const title = this.meetingName.trim();
		try {
			this.state = 'running';
			settingsStore.updateConfig('systemMessage', systemPrompt);
			settingsStore.updateConfig('enableThinking', false);
			await chatStore.sendMessage(this.transcript);
			this.state = 'idle';
		} catch (e) {
			this.state = 'error';
			this.error = e instanceof Error ? e.message : 'Sammanfattningen misslyckades.';
			toast.error(this.error);
		} finally {
			settingsStore.updateConfig('systemMessage', previousSystem);
			settingsStore.updateConfig('enableThinking', previousThinking);
			// sendMessage döper konversationen efter första meningen i transkriberingen.
			// Skriv över med användarens mötesnamn – i finally så titeln sätts även om
			// körningen avbryts efter att konversationen skapats.
			const convId = conversationsStore.activeConversation?.id;
			if (convId && title) {
				await conversationsStore.updateConversationName(convId, title).catch(() => {});
			}
		}
	}

	/** Räknar tokens för system-prompt + transkribering (för proaktiv ctx). */
	private async tokensWithPrompt(systemPrompt: string): Promise<number> {
		try {
			return await countTokens(`${systemPrompt}\n\n${this.transcript}`);
		} catch {
			return Math.ceil((systemPrompt.length + this.transcript.length) / 4);
		}
	}
}

export const summarizeStore = new SummarizeStore();
