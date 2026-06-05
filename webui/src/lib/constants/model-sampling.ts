/**
 * Modell-anpassade sampling-inställningar för sammanfattningsläget.
 *
 * Olika modeller vill ha olika sampling för att fungera optimalt. Att köra en
 * enda hårdkodad temperatur gav dålig output (Qwen3.5 utan presence_penalty
 * upprepade sig och spårade ur). Här ligger leverantörernas rekommenderade
 * värden, valda per modell och thinking-läge.
 *
 * Källor (verifierade 2026-06-04):
 * - Qwen3.5 instruct/non-thinking: temp 0.7, top_p 0.80, top_k 20, presence 1.5
 *   (presence_penalty motverkar Qwens benägenhet till oändlig upprepning).
 * - Qwen3.5 thinking: temp 0.6, top_p 0.95, top_k 20, presence 1.5.
 * - Gemma 3/4: temp 1.0, top_p 0.95, top_k 64 (Google-teamets standard).
 */

export interface SamplingProfile {
	temperature: number;
	top_p: number;
	top_k: number;
	min_p: number;
	presence_penalty: number;
}

/** Qwen3.5 i instruct/non-thinking-läge (sammanfattningens standard). */
export const QWEN_INSTRUCT: SamplingProfile = {
	temperature: 0.7,
	top_p: 0.8,
	top_k: 20,
	min_p: 0.0,
	presence_penalty: 1.5
};

/** Qwen3.5 i thinking-läge ("Noggrannare", precisa uppgifter). */
export const QWEN_THINKING: SamplingProfile = {
	temperature: 0.6,
	top_p: 0.95,
	top_k: 20,
	min_p: 0.0,
	presence_penalty: 1.5
};

/** Gemma 3/4 (Google-teamets rekommenderade standard). */
export const GEMMA_DEFAULT: SamplingProfile = {
	temperature: 1.0,
	top_p: 0.95,
	top_k: 64,
	min_p: 0.0,
	presence_penalty: 0.0
};

/** Konservativ fallback för okända modeller – balanserad och repetitions-säker. */
export const GENERIC_DEFAULT: SamplingProfile = {
	temperature: 0.7,
	top_p: 0.9,
	top_k: 40,
	min_p: 0.0,
	presence_penalty: 1.0
};

/**
 * Väljer rätt sampling-profil utifrån modellens filnamn/sökväg och thinking-läge.
 *
 * @param modelPath  Modellens sökväg eller filnamn (matchas mot 'qwen'/'gemma').
 * @param thinking   Om thinking är på (påverkar bara Qwen-profilen).
 */
export function samplingForModel(
	modelPath: string | null | undefined,
	thinking: boolean
): SamplingProfile {
	const name = (modelPath ?? '').toLowerCase();
	if (name.includes('qwen')) return thinking ? QWEN_THINKING : QWEN_INSTRUCT;
	if (name.includes('gemma')) return GEMMA_DEFAULT;
	return GENERIC_DEFAULT;
}
