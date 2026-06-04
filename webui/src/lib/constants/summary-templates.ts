import { FileText, ListChecks, ClipboardList, Gavel } from '@lucide/svelte';
import type { Component } from 'svelte';

/**
 * Mallar för Loki 2.0 sammanfattningsläge.
 *
 * Varje mall är en färdig system-prompt som styr vilken sorts dokument modellen
 * producerar från en transkribering. Prompterna är på svenska och instruerar
 * modellen att hålla sig till transkriberingens innehåll (inte hitta på).
 *
 * Egna mallar lagras separat i settings (nyckel `summaryTemplates`, JSON-sträng)
 * och har samma form som SummaryTemplate men `builtin: false`.
 */
export interface SummaryTemplate {
	/** Stabil identifierare, används som nyckel och i historik-titlar. */
	id: string;
	/** Visningsnamn på knappen. */
	label: string;
	/** Kort beskrivning under knappen. */
	description: string;
	/** Lucide-ikon (utelämnas för egna mallar). */
	icon?: Component;
	/** System-prompten som skickas till modellen. */
	systemPrompt: string;
	/** True för inbyggda mallar, false/undefined för användarens egna. */
	builtin?: boolean;
}

/**
 * Gemensam instruktion som läggs först i varje inbyggd prompt. Håller modellen
 * trogen källan och språket, och undviker pratiga inledningar.
 */
const COMMON_PREAMBLE =
	'Du är ett verktyg som bearbetar en transkribering av ett möte eller samtal. ' +
	'Arbeta ENBART utifrån transkriberingens innehåll – hitta aldrig på fakta, namn, ' +
	'beslut eller siffror som inte finns i texten. Svara på samma språk som ' +
	'transkriberingen (oftast svenska). ' +
	'Var trogen källan: använd talarnas egna ord och termer. Byt INTE ut ord mot ' +
	'synonymer, skriv inte om i onödan och använd inga metaforer, bildspråk eller ' +
	'tolkande omskrivningar som kan förvränga vad som sades eller beslutades. ' +
	'Skriv inga inledande eller avslutande kommentarer om uppgiften – leverera bara ' +
	'det begärda dokumentet i Markdown.';

export const SUMMARY_TEMPLATES: SummaryTemplate[] = [
	{
		id: 'meeting-minutes',
		label: 'Mötesprotokoll',
		description: 'Strukturerat protokoll med deltagare, beslut och åtgärder',
		icon: FileText,
		builtin: true,
		systemPrompt:
			`${COMMON_PREAMBLE}\n\n` +
			'Skapa ett strukturerat mötesprotokoll med följande rubriker (utelämna en rubrik ' +
			'helt om underlag saknas i transkriberingen):\n' +
			'## Sammanfattning – två till fyra meningar om mötets syfte och utfall.\n' +
			'## Deltagare – namngivna personer som tydligt deltar i samtalet.\n' +
			'## Diskussionspunkter – de viktigaste ämnena, kort per punkt.\n' +
			'## Beslut – fattade beslut, ett per punkt.\n' +
			'## Åtgärder – att-göra-punkter med ansvarig person när det framgår, format ' +
			'"- [ ] Åtgärd — Ansvarig".'
	},
	{
		id: 'summary',
		label: 'Kort sammanfattning',
		description: '5–10 punkter med det viktigaste',
		icon: ListChecks,
		builtin: true,
		systemPrompt:
			`${COMMON_PREAMBLE}\n\n` +
			'Skapa en kort, lättläst sammanfattning som en punktlista med 5–10 punkter. ' +
			'Fånga de viktigaste ämnena, slutsatserna och eventuella beslut. Varje punkt ska ' +
			'vara en kort, fristående mening. Inga underrubriker.'
	},
	{
		id: 'action-items',
		label: 'Åtgärdspunkter',
		description: 'Bara att-göra-listan med ansvariga',
		icon: ClipboardList,
		builtin: true,
		systemPrompt:
			`${COMMON_PREAMBLE}\n\n` +
			'Extrahera enbart konkreta åtgärder och uppgifter som nämns i transkriberingen. ' +
			'Returnera en checklista i formatet "- [ ] Åtgärd — Ansvarig (deadline om nämnd)". ' +
			'Ta med ansvarig person och deadline bara när det tydligt framgår. Om inga ' +
			'åtgärder nämns, skriv "Inga åtgärder identifierades."'
	},
	{
		id: 'decision-log',
		label: 'Beslutslogg',
		description: 'Fattade beslut med motivering',
		icon: Gavel,
		builtin: true,
		systemPrompt:
			`${COMMON_PREAMBLE}\n\n` +
			'Lista alla beslut som fattas i transkriberingen. För varje beslut, ange på en ' +
			'egen rad: **Beslut:** vad som beslutades, och **Motivering:** den motivering eller ' +
			'det resonemang som framgår av samtalet. Utelämna motivering om den inte framgår. ' +
			'Om inga beslut fattas, skriv "Inga beslut fattades."'
	}
];

/**
 * Prompt för språkrättningssteget (Gemma 4 E2B). Körs på ett färdigt protokoll
 * för att rätta svenskan och polera strukturen LÄTT – utan att skriva om eller
 * lägga till innehåll. Bevarar alla fakta, beslut, namn och punkter exakt.
 */
export const LANGUAGE_REFINE_PROMPT =
	'Du är en svensk språkgranskare. Du får ett mötesprotokoll eller en sammanfattning ' +
	'som kan innehålla språkfel, engelska ord (svengelska) eller stela formuleringar. ' +
	'Din uppgift:\n' +
	'- Rätta stavning, grammatik och osvensk meningsbyggnad till korrekt, naturlig svenska.\n' +
	'- Översätt kvarvarande engelska ord/uttryck till svenska.\n' +
	'- Polera strukturen LÄTT för läsbarhet (rubriker, punktlistor), men ändra inte ordningen.\n' +
	'VIKTIGT: Ändra ALDRIG innehållet. Lägg inte till, ta inte bort och tolka inte om fakta, ' +
	'beslut, namn, siffror eller åtgärder. Byt INTE ut korrekta facktermer, egennamn eller ' +
	'citat mot synonymer, och inför inga metaforer eller bildspråk. Rätta bara det som är ' +
	'språkligt fel. Behåll Markdown-formatet. Svara enbart med det rättade dokumentet – inga ' +
	'kommentarer om vad du ändrat.';

/** Den mall som väljs som standard när vyn öppnas. */
export const DEFAULT_TEMPLATE_ID = 'meeting-minutes';

/** Slår upp en inbyggd mall på id. */
export function getBuiltinTemplate(id: string): SummaryTemplate | undefined {
	return SUMMARY_TEMPLATES.find((t) => t.id === id);
}
