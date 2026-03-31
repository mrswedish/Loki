/**
 * text-scoring.ts – IDF-baserad meningsscoring för extractive pre-filtering.
 *
 * Rensar bort utfyllnad och upprepningar från långa transkriberingar/dokument
 * innan LLM-chunkning. Noll beroenden – ren TypeScript.
 *
 * Algoritm: IDF-only sentence scoring (inte full TF-IDF).
 * Meningar som innehåller sällsynta ord i dokumentet poängsätts högt –
 * de är mer informativa. Vanliga fraser ("vi diskuterade", "eh", "okej")
 * faller bort naturligt via liten df → låg IDF, låg meningspoäng.
 */

/** Svenska stopwords – ord som inte bär informationsvärde. */
const SWEDISH_STOPWORDS = new Set([
	'och', 'eller', 'att', 'för', 'från', 'till', 'i', 'på', 'med', 'av', 'om',
	'är', 'var', 'har', 'hade', 'det', 'den', 'de', 'vi', 'du', 'han', 'hon',
	'ett', 'en', 'sig', 'så', 'men', 'som', 'när', 'hur', 'vad', 'vem',
	'vilken', 'vilket', 'vilka', 'inte', 'också', 'redan', 'bara', 'mycket',
	'mer', 'mest', 'sedan', 'nu', 'då', 'här', 'där', 'detta', 'dessa',
	'denna', 'deras', 'hans', 'hennes', 'våra', 'era', 'min', 'din', 'sin',
	'mitt', 'ditt', 'sitt', 'alla', 'allt', 'något', 'några', 'ingen', 'inget',
	'inga', 'man', 'kan', 'ska', 'vill', 'måste', 'bör', 'får', 'under',
	'över', 'vid', 'mot', 'utan', 'genom', 'efter', 'innan', 'medan', 'dock',
	'även', 'både', 'antingen', 'varken', 'ju', 'nog', 'väl', 'ganska',
	'lite', 'väldigt', 'gärna', 'kanske', 'troligen', 'faktiskt', 'ja', 'nej',
	'okej', 'okay', 'mm', 'eh', 'jo', 'va', 'typ', 'liksom', 'alltså',
	'precis', 'just', 'visst', 'rätt', 'samma', 'annan', 'andra', 'nästa',
	'förra', 'hela', 'halv', 'igen', 'ändå', 'däremot', 'dessutom', 'därför',
	'eftersom', 'ifall', 'huruvida', 'vare', 'varje', 'vart', 'vars',
	'vars', 'vems', 'åt', 'ur', 'jag', 'mig', 'dem', 'oss', 'er', 'ni',
	'dessa', 'sina', 'dina', 'mina', 'ens', 'inga', 'inget'
]);

/** Delar text i meningar på .  !  ? och filtrerar bort för korta. */
function splitSentences(text: string): string[] {
	return text
		.split(/(?<=[.!?])\s+/)
		.map((s) => s.trim())
		.filter((s) => s.length > 30);
}

/** Tokeniserar en mening till unika informationsbärande ord. */
function tokenize(sentence: string): string[] {
	return [
		...new Set(
			sentence
				.toLowerCase()
				.replace(/[^a-zåäöA-ZÅÄÖ\s]/g, ' ')
				.split(/\s+/)
				.filter((w) => w.length > 2 && !SWEDISH_STOPWORDS.has(w))
		)
	];
}

/**
 * Beräknar IDF-baserad meningsscoring och returnerar filtrerad text
 * i ursprunglig ordning.
 *
 * Minnesanvändning för 2h-möte (~3 000 meningar, ~8 000 unika ord):
 *   df-map:      ~400 KB
 *   scores-array: ~24 KB
 *   Totalt:      < 500 KB
 *
 * @param text       Hela dokumenttexten
 * @param keepRatio  Andel meningar att behålla (0.5–0.8). Default 0.65.
 * @returns          Filtrerad text i ursprunglig ordning
 */
export function extractKeySentenceText(text: string, keepRatio = 0.65): string {
	const sentences = splitSentences(text);
	if (sentences.length < 5) return text; // för kort – filtrera inte

	const N = sentences.length;
	const tokenized = sentences.map(tokenize);

	// Bygg document-frequency map (i hur många meningar förekommer ordet)
	const df = new Map<string, number>();
	for (const words of tokenized) {
		for (const w of words) {
			df.set(w, (df.get(w) ?? 0) + 1);
		}
	}

	// Poängberäkning: Σ log((N+1) / df(w)) per unikt ord, normerat per ordantal
	const scores = tokenized.map((words) => {
		if (words.length === 0) return 0;
		const score = words.reduce((sum, w) => {
			const docFreq = df.get(w) ?? 1;
			return sum + Math.log((N + 1) / docFreq); // +1 Laplace-smoothing
		}, 0);
		return score / words.length;
	});

	// Välj top keepRatio meningar, bevara ursprunglig ordning
	const keepCount = Math.max(3, Math.round(sentences.length * keepRatio));
	const topIndices = new Set(
		scores
			.map((score, i) => ({ score, i }))
			.sort((a, b) => b.score - a.score)
			.slice(0, keepCount)
			.map(({ i }) => i)
	);

	return sentences
		.filter((_, i) => topIndices.has(i))
		.join('\n\n');
}
