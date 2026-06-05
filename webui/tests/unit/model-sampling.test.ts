import { describe, it, expect } from 'vitest';
import {
	samplingForModel,
	mergeSampling,
	QWEN_INSTRUCT,
	QWEN_THINKING,
	GEMMA_DEFAULT,
	GENERIC_DEFAULT
} from '$lib/constants/model-sampling';

describe('samplingForModel', () => {
	it('väljer Qwen instruct (non-thinking) som standard', () => {
		expect(samplingForModel('C:/models/Qwen3.5-4B-Q4_K_M.gguf', false)).toEqual(QWEN_INSTRUCT);
	});

	it('väljer Qwen thinking-profil när thinking är på', () => {
		expect(samplingForModel('Qwen3.5-4B.gguf', true)).toEqual(QWEN_THINKING);
	});

	it('väljer Gemma-profil oavsett thinking-flagga', () => {
		expect(samplingForModel('google_gemma-4-E2B-it.gguf', false)).toEqual(GEMMA_DEFAULT);
		expect(samplingForModel('google_gemma-4-E2B-it.gguf', true)).toEqual(GEMMA_DEFAULT);
	});

	it('Qwen instruct har presence_penalty mot upprepning', () => {
		expect(QWEN_INSTRUCT.presence_penalty).toBeGreaterThan(0);
	});

	it('faller tillbaka till generisk profil för okänd modell', () => {
		expect(samplingForModel('mystery-model.gguf', false)).toEqual(GENERIC_DEFAULT);
		expect(samplingForModel(null, false)).toEqual(GENERIC_DEFAULT);
		expect(samplingForModel(undefined, false)).toEqual(GENERIC_DEFAULT);
	});

	it('matchar oberoende av versaler i sökvägen', () => {
		expect(samplingForModel('QWEN3.5.GGUF', false)).toEqual(QWEN_INSTRUCT);
		expect(samplingForModel('GEMMA-4.GGUF', false)).toEqual(GEMMA_DEFAULT);
	});
});

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
		expect(merged.top_p).toBe(base.top_p);
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
