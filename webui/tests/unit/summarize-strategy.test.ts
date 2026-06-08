import { describe, it, expect } from 'vitest';
import {
	pickStrategy,
	roundCtx,
	splitIntoChunks,
	buildSystemPrompt,
	buildMapPrompt,
	isEmptyChunkResponse,
	safeCtxCeiling,
	MAX_CTX,
	RESPONSE_MARGIN,
	RESPONSE_MARGIN_THINKING
} from '$lib/services/summarize.service';
import {
	COMMON_PREAMBLE,
	CONTEXT_INSTRUCTION,
	type SummaryTemplate
} from '$lib/constants/summary-templates';

/** En stabil delfras ur preambeln (de första 30 tecknen) som inte hårdkodas. */
const PREAMBLE_PHRASE = COMMON_PREAMBLE.slice(0, 30);

/** Stabila delfraser ur kontext-instruktionerna (hårdkoda inte hela texten). */
const CONTEXT_PHRASE = CONTEXT_INSTRUCTION.slice(0, 30);
/** En fras som BARA finns i den kreativa varianten ("forma tonen"). */
const CREATIVE_PHRASE = 'forma tonen';

const TEMPLATE: SummaryTemplate = {
	id: 'test',
	label: 'Test',
	description: '',
	systemPrompt: 'BAS-PROMPT',
	builtin: true
};

describe('roundCtx', () => {
	it('avrundar uppåt till närmaste 1024', () => {
		expect(roundCtx(1)).toBe(1024);
		expect(roundCtx(1024)).toBe(1024);
		expect(roundCtx(1025)).toBe(2048);
		expect(roundCtx(23439)).toBe(23552);
	});
});

describe('pickStrategy', () => {
	it('väljer single med exakt ctx när prompten ryms', () => {
		const d = pickStrategy(23439, 131_072);
		expect(d.strategy).toBe('single');
		// 23439 + 2048 = 25487 → avrundat 26624
		expect(d.ctx).toBe(roundCtx(23439 + RESPONSE_MARGIN));
		expect(d.ctx % 1024).toBe(0);
	});

	it('respekterar modellens maxkontext som tak', () => {
		// Prompt som ryms i 131k men inte i en liten modell (8k tränad ctx)
		const d = pickStrategy(20_000, 8192);
		expect(d.strategy).toBe('chunked');
	});

	it('väljer chunked när prompten överstiger taket', () => {
		const d = pickStrategy(MAX_CTX, 131_072);
		expect(d.strategy).toBe('chunked');
	});

	it('default-tak är MAX_CTX när modellmax inte anges', () => {
		const d = pickStrategy(1000);
		expect(d.strategy).toBe('single');
		expect(d.ctx).toBe(roundCtx(1000 + RESPONSE_MARGIN));
	});

	it('reserverar större ctx när thinking-marginal anges', () => {
		const normal = pickStrategy(20_000, 131_072, RESPONSE_MARGIN);
		const thinking = pickStrategy(20_000, 131_072, RESPONSE_MARGIN_THINKING);
		expect(thinking.ctx).toBeGreaterThan(normal.ctx);
		expect(thinking.ctx).toBe(roundCtx(20_000 + RESPONSE_MARGIN_THINKING));
	});
});

describe('buildSystemPrompt', () => {
	it('returnerar bara mallens prompt utan agenda', () => {
		expect(buildSystemPrompt(TEMPLATE)).toBe('BAS-PROMPT');
		expect(buildSystemPrompt(TEMPLATE, '   ')).toBe('BAS-PROMPT');
	});

	it('injicerar agendan när den finns', () => {
		const out = buildSystemPrompt(TEMPLATE, '1. Budget\n2. Personal');
		expect(out).toContain('BAS-PROMPT');
		expect(out).toContain('STRUKTUR ENLIGT AGENDA');
		expect(out).toContain('1. Budget');
		expect(out).toContain('2. Personal');
		expect(out).toContain('Övrigt');
	});

	it('lägger trohets-preambeln på egna mallar (builtin: false)', () => {
		const custom: SummaryTemplate = { ...TEMPLATE, builtin: false };
		const out = buildSystemPrompt(custom);
		expect(out).toContain('BAS-PROMPT');
		expect(out).toContain(PREAMBLE_PHRASE); // preambeln har lagts på
	});

	it('dubblerar inte preambeln om den redan finns i prompten (kopia av inbyggd)', () => {
		const builtinOut = buildSystemPrompt(TEMPLATE); // = BAS-PROMPT (TEMPLATE saknar preambel)
		// Simulera en kopia av en inbyggd mall: prompten innehåller redan preambeln.
		const copyOfBuiltin: SummaryTemplate = {
			...TEMPLATE,
			builtin: false,
			systemPrompt: buildSystemPrompt({ ...TEMPLATE, builtin: false }) // har preambel
		};
		const out = buildSystemPrompt(copyOfBuiltin);
		// Preambelns kärnfras får bara förekomma en gång.
		const occurrences = out.split(PREAMBLE_PHRASE).length - 1;
		expect(occurrences).toBe(1);
		expect(builtinOut).toBe('BAS-PROMPT');
	});
});

describe('buildMapPrompt', () => {
	it('inkluderar mallens prompt + avsnitts-instruktion', () => {
		const out = buildMapPrompt(TEMPLATE);
		expect(out).toContain('BAS-PROMPT');
		expect(out).toContain('ETT AVSNITT');
		expect(out).toContain('Inget relevant i detta avsnitt');
	});

	it('inkluderar kontexten + kreativ instruktion när creative är på', () => {
		const out = buildMapPrompt(TEMPLATE, 'rehabprocess sjukvård', true);
		expect(out).toContain('rehabprocess sjukvård');
		expect(out).toContain(CREATIVE_PHRASE);
	});
});

describe('buildSystemPrompt – kontext', () => {
	it('injicerar kontexten med strikt instruktion (creative av, default)', () => {
		const out = buildSystemPrompt(TEMPLATE, undefined, 'rehabprocess sjukvård');
		expect(out).toContain(CONTEXT_PHRASE);
		expect(out).toContain('rehabprocess sjukvård');
		expect(out).not.toContain(CREATIVE_PHRASE);
	});

	it('använder kreativ instruktion när creative är på', () => {
		const out = buildSystemPrompt(TEMPLATE, undefined, 'rehabprocess sjukvård', true);
		expect(out).toContain(CREATIVE_PHRASE);
		expect(out).toContain('rehabprocess sjukvård');
	});

	it('placerar kontexten FÖRE agendan', () => {
		const out = buildSystemPrompt(TEMPLATE, '1. Budget', 'rehabprocess sjukvård');
		expect(out).toContain('rehabprocess sjukvård');
		expect(out).toContain('STRUKTUR ENLIGT AGENDA');
		expect(out.indexOf('rehabprocess sjukvård')).toBeLessThan(out.indexOf('STRUKTUR ENLIGT AGENDA'));
	});

	it('ingen kontext → ingen kontext-instruktion (ingen regression)', () => {
		const out = buildSystemPrompt(TEMPLATE);
		expect(out).toBe('BAS-PROMPT');
		expect(out).not.toContain(CONTEXT_PHRASE);
	});
});

describe('safeCtxCeiling', () => {
	const GB = 1_000_000_000;

	it('faller tillbaka till MAX_CTX när RAM-info saknas', () => {
		expect(safeCtxCeiling(null, 6.7 * GB)).toBe(MAX_CTX);
		expect(safeCtxCeiling(0, 6.7 * GB)).toBe(MAX_CTX);
	});

	it('maxar mot MAX_CTX när RAM räcker (KV-cachen är billig)', () => {
		// E4B på 14,6 GB har gott om RAM-budget → taket når MAX_CTX (120-min ryms single).
		expect(safeCtxCeiling(14.6, 6.9 * GB)).toBe(MAX_CTX);
		expect(safeCtxCeiling(16, 6.7 * GB)).toBe(MAX_CTX);
	});

	it('ger mer utrymme på en maskin med mer RAM (vid knappt minne)', () => {
		const ram8 = safeCtxCeiling(8, 6.7 * GB);
		const ram16 = safeCtxCeiling(16, 6.7 * GB);
		expect(ram16).toBeGreaterThan(ram8);
	});

	it('ger en liten modell mer utrymme på samma knappa RAM', () => {
		// På 10 GB ryms E2B med mer ctx än 12B (mindre modellvikter → större KV-budget).
		const small = safeCtxCeiling(10, 3.5 * GB); // E2B
		const big = safeCtxCeiling(10, 6.7 * GB); // 12B
		expect(small).toBeGreaterThan(big);
	});

	it('klampar till minst 8k (golv) när minnet är knappt', () => {
		// 12B på en maskin med bara 8 GB → mycket liten KV-budget, men golvet garanterar 8k.
		expect(safeCtxCeiling(8, 6.7 * GB)).toBe(8192);
	});

	it('överstiger aldrig MAX_CTX', () => {
		expect(safeCtxCeiling(256, 3.5 * GB)).toBeLessThanOrEqual(MAX_CTX);
	});

	// Invariant (Steg 10): en 120-min-transkribering (~23k tokens) körs SINGLE, inte chunkad.
	it('120-min-transkribering (23k) körs single på E4B/14.6GB (ingen chunkning)', () => {
		const ceiling = safeCtxCeiling(14.6, 6.9 * GB);
		const d = pickStrategy(23000, ceiling, RESPONSE_MARGIN);
		expect(d.strategy).toBe('single');
	});

	// Invariant (Steg 9): en vanlig mötesprompt ryms single i E4B-taket, ingen overflow.
	it('en 8885-tokens-prompt ryms single i E4B-taket (ingen overflow)', () => {
		const ceiling = safeCtxCeiling(14.6, 6.9 * GB);
		const d = pickStrategy(8885, ceiling, RESPONSE_MARGIN);
		expect(d.strategy).toBe('single');
		expect(d.ctx).toBeGreaterThanOrEqual(8885 + RESPONSE_MARGIN);
	});
});

describe('isEmptyChunkResponse', () => {
	it('fångar exakt formulering', () => {
		expect(isEmptyChunkResponse('Inget relevant i detta avsnitt.')).toBe(true);
	});

	it('fångar varianter (citattecken, emfas, omvänd ordföljd)', () => {
		expect(isEmptyChunkResponse('"Inget relevant i detta avsnitt."')).toBe(true);
		expect(isEmptyChunkResponse('*Inget relevant.*')).toBe(true);
		expect(isEmptyChunkResponse('Detta avsnitt innehåller inget relevant.')).toBe(true);
		expect(isEmptyChunkResponse('inget relevant')).toBe(true);
	});

	it('släpper igenom riktigt innehåll', () => {
		expect(isEmptyChunkResponse('### Budget\nGruppen beslutade att öka anslaget.')).toBe(false);
		// Lång text som råkar nämna frasen ska INTE filtreras bort.
		const long =
			'Mötet diskuterade om något var relevant eller inte, och kom fram till att ' +
			'budgetfrågan var högst relevant för verksamheten under kommande kvartal.';
		expect(isEmptyChunkResponse(long)).toBe(false);
	});
});

describe('splitIntoChunks', () => {
	it('returnerar en bit när texten ryms i budgeten', () => {
		const text = 'Kort text.';
		expect(splitIntoChunks(text, 1000, 4)).toEqual([text]);
	});

	it('delar lång text i flera bitar', () => {
		// 10000 tecken, budget 100 tokens * 4 chars = 400 chars/bit → flera bitar
		const text = 'a'.repeat(10_000);
		const chunks = splitIntoChunks(text, 100, 4, 0);
		expect(chunks.length).toBeGreaterThan(1);
	});

	it('bryter helst vid meningsgräns', () => {
		const sentence = 'Detta är en mening. ';
		const text = sentence.repeat(50); // 1000 tecken
		const chunks = splitIntoChunks(text, 50, 4, 0); // ~200 chars/bit
		// Varje bit (utom ev. sista) ska sluta med punkt efter mening
		for (const c of chunks.slice(0, -1)) {
			expect(c.trim().endsWith('.')).toBe(true);
		}
	});

	it('täcker hela texten', () => {
		const text = 'x'.repeat(5000);
		const chunks = splitIntoChunks(text, 100, 4, 0);
		const total = chunks.join('').length;
		// Utan överlapp ska sammanlagda längden motsvara originalet (trim kan ta whitespace)
		expect(total).toBeGreaterThanOrEqual(text.length - chunks.length);
	});
});
