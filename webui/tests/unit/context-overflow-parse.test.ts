import { describe, it, expect } from 'vitest';
import { CONTEXT_OVERFLOW_MESSAGE_REGEX } from '$lib/services/chat.service';

// Speglar fallback-grenen i ChatService.parseErrorResponse (privat static): när
// llama.cpp-svaret saknar de strukturerade fälten n_prompt_tokens/n_ctx ska talen
// kunna plockas ur message-strängen så att auto-expand ändå kan trigga.
function extractContextInfoFromMessage(
	message: string
): { n_prompt_tokens: number; n_ctx: number } | undefined {
	const match = message.match(CONTEXT_OVERFLOW_MESSAGE_REGEX);
	if (!match) return undefined;
	return { n_prompt_tokens: Number(match[1]), n_ctx: Number(match[2]) };
}

describe('context-overflow message fallback parsing', () => {
	it('extraherar tokens och ctx ur det exakta llama.cpp-meddelandet', () => {
		// Exakt sträng från serverloggen (send_error).
		const message =
			'request (23908 tokens) exceeds the available context size (16384 tokens), try increasing it';
		expect(extractContextInfoFromMessage(message)).toEqual({
			n_prompt_tokens: 23908,
			n_ctx: 16384
		});
	});

	it('matchar oberoende av versaler', () => {
		const message =
			'Request (5000 tokens) Exceeds The Available Context Size (4096 tokens)';
		expect(extractContextInfoFromMessage(message)).toEqual({
			n_prompt_tokens: 5000,
			n_ctx: 4096
		});
	});

	it('returnerar undefined för orelaterade felmeddelanden', () => {
		expect(extractContextInfoFromMessage('Unknown server error')).toBeUndefined();
		expect(extractContextInfoFromMessage('Connection refused')).toBeUndefined();
	});
});
