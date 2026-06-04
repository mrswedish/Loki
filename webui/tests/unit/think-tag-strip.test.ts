import { describe, it, expect } from 'vitest';
import { AGENTIC_REGEX } from '$lib/constants/agentic';

/** Speglar ChatService.stripRawThinkTags. */
function stripThink(content: string): string {
	return content.replace(AGENTIC_REGEX.THINK_BLOCK, '').replace(AGENTIC_REGEX.THINK_OPEN, '');
}

describe('think-tag stripping', () => {
	it('tar bort <think>...</think>', () => {
		expect(stripThink('<think>resonemang</think>Svar.')).toBe('Svar.');
	});

	it('tar bort <thinking>...</thinking> (Qwen3.5-variant)', () => {
		expect(stripThink('<thinking>resonemang</thinking>Svar.')).toBe('Svar.');
	});

	it('tar bort tomma think-block (läckage när thinking är av)', () => {
		expect(stripThink('<thinking></thinking>Protokoll')).toBe('Protokoll');
		expect(stripThink('<think></think>Protokoll')).toBe('Protokoll');
	});

	it('tar bort oavslutad öppningstagg under streaming', () => {
		expect(stripThink('Svar.<thinking>halvt resonemang')).toBe('Svar.');
		expect(stripThink('<think>oavslutat')).toBe('');
	});

	it('lämnar text utan think-taggar orörd', () => {
		expect(stripThink('Ren text utan taggar.')).toBe('Ren text utan taggar.');
	});
});
