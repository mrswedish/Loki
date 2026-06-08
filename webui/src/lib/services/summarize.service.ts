import { apiPost } from '$lib/utils/api-fetch';
import {
	COMMON_PREAMBLE,
	CONTEXT_INSTRUCTION,
	CONTEXT_INSTRUCTION_CREATIVE,
	type SummaryTemplate
} from '$lib/constants/summary-templates';

/**
 * SummarizeService – kärnlogik för Loki 2.0 sammanfattningsläge.
 *
 * Ansvarar för:
 * - Proaktiv tokenräkning via llama-serverns /tokenize-endpoint.
 * - Val av strategi (single vs chunked) utifrån promptens storlek och modellens
 *   maximala kontextfönster.
 * - Beräkning av exakt kontextstorlek att starta servern med, så att overflow
 *   undviks helt (ingen reaktiv auto-expand-omstart behövs).
 *
 * Själva genereringen (streaming) görs av anroparen via ChatService; denna
 * service håller bara den deterministiska, testbara logiken.
 */

/** Hårt tak för kontextfönstret – matchar auto-expand-logiken i chat-storen. */
export const MAX_CTX = 131_072;

/** Marginal i tokens reserverad för modellens svar utöver prompten (thinking av). */
export const RESPONSE_MARGIN = 4096;

/**
 * Marginal när thinking är på ("Noggrannare"). Resonemanget skrivs ut före svaret
 * och förbrukar kontext, så vi reserverar rejält extra utrymme så att modellen
 * hinner både tänka och svara utan att slå i taket.
 */
export const RESPONSE_MARGIN_THINKING = 10_240;

/**
 * Beräknar ett SÄKERT tak för kontextfönstret baserat på tillgängligt RAM och
 * modellens storlek. KV-cachen (plus GPU-overhead vid uppstart) kan spränga minnet
 * långt innan modellens teoretiska maxkontext. Vi laddar därför inte ett större
 * fönster än vad systemet säkert klarar – långa texter chunkas i stället.
 *
 * Heuristiken är FÖRSIKTIG men måste rymma vanliga möten i ETT pass: en typisk
 * transkribering på 8–13k tokens ska INTE tvingas till chunkning eller (värre) skickas
 * till ett för litet fönster → context-overflow. Floor 16k garanterar att sådana
 * prompter ryms. För riktigt långa texter returnerar pickStrategy ändå 'chunked' när
 * prompten överstiger taket. (VRAM vid uppstart hanteras separat av llama.cpp:s
 * --n-gpu-layers auto, inte här.) Faller tillbaka till MAX_CTX om RAM-info saknas.
 *
 * @param totalRamGb     Totalt systemminne i GB (CPU-körning kan nyttja allt).
 * @param modelSizeBytes Modellfilens storlek i bytes (ungefär modellvikternas RAM).
 */
export function safeCtxCeiling(
	totalRamGb: number | null | undefined,
	modelSizeBytes: number | null | undefined
): number {
	if (!totalRamGb || totalRamGb <= 0) return MAX_CTX;
	const modelGb = modelSizeBytes ? modelSizeBytes / 1_000_000_000 : 4;
	// Reservera ~2 GB för OS/app; resten efter modellvikterna får gå till KV-cache.
	const kvBudgetGb = Math.max(0, totalRamGb - modelGb - 2);
	// KV-cachen per token skalar med modellens storlek (fler lager = större cache).
	// 14000 ger E4B/12B ~16k på en 14–16 GB-maskin (rymmer vanliga möten single), och
	// mer på maskiner med mer minne. Mindre modeller får fler tokens/GB → större fönster.
	const tokensPerGb = 14_000 / modelGb;
	const ceiling = roundCtx(kvBudgetGb * tokensPerGb);
	// Klampa till [16k, MAX_CTX] – minst 16k så att vanliga möten alltid ryms i ett pass.
	return Math.min(Math.max(ceiling, 16_384), MAX_CTX);
}

/** llama.cpp /tokenize-svar. */
interface TokenizeResponse {
	tokens: number[];
}

export interface StrategyDecision {
	/** "single" = ett pass; "chunked" = map-reduce över flera bitar. */
	strategy: 'single' | 'chunked';
	/** Kontextstorlek att starta servern med (för single). Avrundad till 1024. */
	ctx: number;
}

/**
 * Bygger den slutliga system-prompten för en körning: mallens prompt, plus ett
 * valfritt kontext-block (mötets domän/sammanhang) och ett valfritt agenda-block
 * som styr protokollet att följa agendans punkter.
 *
 * @param template  Vald mall (inbyggd eller egen).
 * @param agenda    Råtext från en uppladdad agenda (.txt/.pdf), eller undefined.
 * @param context   Användarens fritext om mötets domän/sammanhang, eller undefined.
 * @param creative  "Kreativare tolkning" på → kontexten får forma ton/struktur friare.
 */
export function buildSystemPrompt(
	template: SummaryTemplate,
	agenda?: string,
	context?: string,
	creative = false
): string {
	// Egna mallar har inte trohets-preamblen inbyggd (inbyggda mallar har den) –
	// prependa den så även användarens egna mallar håller sig troget källan. Lägg
	// inte på den om prompten redan innehåller den (t.ex. en kopia av en inbyggd
	// mall, vars prompt redan har preambeln) – annars dubbleras instruktionen.
	const hasPreamble = template.systemPrompt.includes(COMMON_PREAMBLE.slice(0, 60));
	const base =
		template.builtin || hasPreamble
			? template.systemPrompt
			: `${COMMON_PREAMBLE}\n\n${template.systemPrompt}`;

	// Kontext-blocket läggs FÖRE agendan (så agendan kan referera domänen). Instruktionens
	// styrka beror på "Kreativare tolkning"-växeln.
	const ctx = context?.trim();
	const instr = creative ? CONTEXT_INSTRUCTION_CREATIVE : CONTEXT_INSTRUCTION;
	const withContext = ctx ? `${base}\n\n${instr}\n${ctx}` : base;

	const trimmed = agenda?.trim();
	if (!trimmed) return withContext;

	return (
		`${withContext}\n\n` +
		'STRUKTUR ENLIGT AGENDA: Strukturera dokumentet efter agendan nedan. Använd ' +
		'agendans punkter som rubriker i den ordning de står. Placera det som diskuterades ' +
		'under rätt agendapunkt. Punkter som togs upp men inte finns på agendan samlas under ' +
		'rubriken "Övrigt". Agendapunkter som inte berördes markeras kort som "Ej behandlad".\n\n' +
		`--- AGENDA ---\n${trimmed}\n--- SLUT AGENDA ---`
	);
}

/**
 * Bygger map-prompten för ett avsnitt vid chunkad sammanfattning. Återanvänder
 * mallens system-prompt men instruerar modellen att detta är ETT avsnitt av ett
 * längre möte och att svara "Inget relevant i detta avsnitt" om avsnittet bara
 * innehåller kallprat. Agendan utelämnas i map-steget (appliceras i reduce), men
 * kontexten följer med – fackordstolkningen behövs per avsnitt.
 */
export function buildMapPrompt(
	template: SummaryTemplate,
	context?: string,
	creative = false
): string {
	const base = buildSystemPrompt(template, undefined, context, creative);
	return (
		`${base}\n\n` +
		'OBS: Detta är ETT AVSNITT av en längre transkribering, inte hela mötet. ' +
		'Sammanfatta bara innehållet i detta avsnitt. Om avsnittet enbart innehåller ' +
		'kallprat eller socialt småprat utan relevans, svara exakt: ' +
		'"Inget relevant i detta avsnitt."'
	);
}

/**
 * Avgör om en map-delsammanfattning betyder "inget relevant innehåll" och ska
 * hoppas över. Modellen kan formulera sig olika trots instruktionen, så vi matchar
 * brett: bara korta svar (< 80 tecken) som innehåller frasen "inget relevant",
 * efter att inledande citattecken/markdown-emfas strippats.
 */
export function isEmptyChunkResponse(text: string): boolean {
	const cleaned = text
		.trim()
		.replace(/^[*_"'`\s]+/, '') // strippa inledande emfas/citattecken
		.toLowerCase();
	return text.trim().length < 80 && /inget relevant/.test(cleaned);
}

/**
 * Räknar antalet tokens i en text via llama-serverns /tokenize-endpoint.
 * Exakt för den laddade modellens tokeniserare. Kastar vid serverfel.
 */
export async function countTokens(text: string): Promise<number> {
	const res = await apiPost<TokenizeResponse>('/tokenize', { content: text });
	return res.tokens?.length ?? 0;
}

/**
 * Avrundar uppåt till närmaste 1024 (llama-server vill ha jämna ctx-storlekar).
 * Samma beräkning som auto-expand i chat-storen använder.
 */
export function roundCtx(tokens: number): number {
	return Math.ceil(tokens / 1024) * 1024;
}

/**
 * Avgör hur en transkribering ska sammanfattas.
 *
 * @param promptTokens  Antal tokens i hela prompten (system + transkribering).
 * @param modelMaxCtx   Modellens maximala kontext (n_ctx_train), default MAX_CTX.
 * @param responseMargin  Tokens att reservera för svar (+ ev. resonemang). När
 *                        thinking är på, skicka RESPONSE_MARGIN_THINKING.
 * @returns strategi + ctx att starta servern med.
 *
 * Ryms prompten + svarsmarginal inom modellens fönster (och taket) → "single"
 * med exakt rätt ctx. Annars "chunked" (anroparen kör map-reduce).
 */
export function pickStrategy(
	promptTokens: number,
	modelMaxCtx: number = MAX_CTX,
	responseMargin: number = RESPONSE_MARGIN
): StrategyDecision {
	const ceiling = Math.min(modelMaxCtx, MAX_CTX);
	const needed = roundCtx(promptTokens + responseMargin);

	if (needed <= ceiling) {
		return { strategy: 'single', ctx: needed };
	}
	// För stort för ett pass – chunkad map-reduce. Kör varje bit i ett rymligt
	// men inte maximalt fönster för fart/minne på svag hårdvara.
	return { strategy: 'chunked', ctx: Math.min(ceiling, roundCtx(32_768)) };
}

/**
 * Delar en lång text i bitar som ryms i ett givet token-budget, helst vid
 * meningsgränser. Token-budgeten anges i tokens; vi uppskattar tecken→tokens
 * via en kalibrerad kvot (chars per token) som anroparen mäter med countTokens.
 *
 * @param text           Texten att dela.
 * @param tokenBudget    Max tokens per bit.
 * @param charsPerToken  Uppmätt tecken/token-kvot för aktuell text+modell.
 * @param overlapTokens  Antal tokens överlapp mellan bitar (kontext-kontinuitet).
 */
export function splitIntoChunks(
	text: string,
	tokenBudget: number,
	charsPerToken: number,
	overlapTokens = 200
): string[] {
	const charBudget = Math.max(1, Math.floor(tokenBudget * charsPerToken));
	const overlapChars = Math.max(0, Math.floor(overlapTokens * charsPerToken));
	if (text.length <= charBudget) return [text];

	const chunks: string[] = [];
	let start = 0;
	while (start < text.length) {
		let end = Math.min(start + charBudget, text.length);
		// Försök bryta vid en meningsgräns nära slutet för läsbarhet.
		if (end < text.length) {
			const window = text.slice(start, end);
			const lastBreak = Math.max(
				window.lastIndexOf('. '),
				window.lastIndexOf('.\n'),
				window.lastIndexOf('\n\n')
			);
			if (lastBreak > charBudget * 0.5) {
				end = start + lastBreak + 1;
			}
		}
		chunks.push(text.slice(start, end).trim());
		if (end >= text.length) break;
		start = Math.max(end - overlapChars, start + 1);
	}
	return chunks;
}
