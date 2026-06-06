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
import { COMMON_PREAMBLE, type SummaryTemplate } from '$lib/constants/summary-templates';

/** En stabil delfras ur preambeln (de första 30 tecknen) som inte hårdkodas. */
const PREAMBLE_PHRASE = COMMON_PREAMBLE.slice(0, 30);

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
});

describe('safeCtxCeiling', () => {
	const GB = 1_000_000_000;

	it('faller tillbaka till MAX_CTX när RAM-info saknas', () => {
		expect(safeCtxCeiling(null, 6.7 * GB)).toBe(MAX_CTX);
		expect(safeCtxCeiling(0, 6.7 * GB)).toBe(MAX_CTX);
	});

	it('begränsar 12B på 16 GB till ~32k (säkert, inte 128k)', () => {
		const ceiling = safeCtxCeiling(16, 6.7 * GB);
		expect(ceiling).toBeGreaterThanOrEqual(24_000);
		expect(ceiling).toBeLessThanOrEqual(40_000);
	});

	it('ger en liten modell mer utrymme på samma RAM', () => {
		const small = safeCtxCeiling(16, 3.5 * GB); // E2B
		const big = safeCtxCeiling(16, 6.7 * GB); // 12B
		expect(small).toBeGreaterThan(big);
	});

	it('klampar till minst 8k när minnet är knappt', () => {
		// 12B på en maskin med bara 8 GB → väldigt lite KV-budget.
		expect(safeCtxCeiling(8, 6.7 * GB)).toBe(8192);
	});

	it('överstiger aldrig MAX_CTX', () => {
		expect(safeCtxCeiling(256, 3.5 * GB)).toBeLessThanOrEqual(MAX_CTX);
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
