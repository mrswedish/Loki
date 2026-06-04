import { describe, it, expect } from 'vitest';
import {
	pickStrategy,
	roundCtx,
	splitIntoChunks,
	buildSystemPrompt,
	MAX_CTX,
	RESPONSE_MARGIN,
	RESPONSE_MARGIN_THINKING
} from '$lib/services/summarize.service';
import type { SummaryTemplate } from '$lib/constants/summary-templates';

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
