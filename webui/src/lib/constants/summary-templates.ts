import { NotebookPen, FileText, ListChecks, ClipboardList, Gavel } from '@lucide/svelte';
import type { Component } from 'svelte';
import type { TemplateSampling } from '$lib/constants/model-sampling';

/**
 * Gemensam sampling-default för inbyggda mallar: mycket låg temp för trohet mot
 * källan och färre feltolkningar (modellen väljer det mest sannolika ordet, vilket
 * minskar "kreativa" gissningar på rörig transkriberingstext).
 */
const STRICT_SAMPLING: TemplateSampling = { temperature: 0.05, repeat_penalty: 1.1 };

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
	/** Valfria sampling-överstyrningar (temperatur m.m.). Saknas → modellens default. */
	sampling?: TemplateSampling;
}

/**
 * Gemensam instruktion som läggs först i varje inbyggd prompt. Håller modellen
 * trogen källan och språket, och undviker pratiga inledningar.
 */
export const COMMON_PREAMBLE =
	'Du är expert på administrativ struktur och textförädling. Du bearbetar en rå, brusig ' +
	'transkribering av ett möte eller samtal till ett rent, begripligt dokument.\n\n' +
	'TRANSKRIBERINGSFEL (VIKTIGT): Texten kommer från röstigenkänning och innehåller grova ' +
	'feltolkningar – helt felaktiga ord, avbrutna meningar och upprepningar. Använd sammanhanget ' +
	'för att förstå vad som egentligen menades och korrigera uppenbart feltolkade ord till rätt ' +
	'facktermer eller korrekta svenska ord.\n' +
	'- Obegripliga förkortningar (t.ex. "DR", "IHL", "ITC") och märkliga, malplacerade ord ' +
	'(t.ex. "skolor", "kårkortsavdelningen" i ett tjänstemannamöte) är nästan alltid ' +
	'transkriberingsfel. Tolka dem utifrån sammanhanget.\n' +
	'- Om du är osäker på vad ett skevt ord ska vara: använd hellre en generell, neutral term ' +
	'("organisationen", "medarbetare", "en avdelning") än att föra vidare det felaktiga ordet ' +
	'eller gissa fritt.\n' +
	'- Hitta ALDRIG på fakta, namn, beslut eller siffror som inte finns i texten.\n\n' +
	'FILTRERA BORT KALLPRAT: Ta bort socialt småprat och privat snack utan relevans för mötets ' +
	'kärnfrågor (t.ex. hälsningar, väder, teknikstrul, privata anekdoter).\n\n' +
	'KRONOLOGI: Följ mötet i den ordning det skedde, ämne för ämne.\n\n' +
	'TROHET: Använd talarnas egna ord och termer. Byt INTE ut ord mot synonymer i onödan och ' +
	'använd inga metaforer eller tolkande omskrivningar som kan förvränga vad som sades. ' +
	'Sammanfatta vad diskussionen landade i – skriv inte av dialogen rakt av.\n\n' +
	'Svara på samma språk som transkriberingen (oftast svenska). Skriv inga inledande eller ' +
	'avslutande kommentarer om uppgiften – leverera bara det begärda dokumentet i Markdown.';

export const SUMMARY_TEMPLATES: SummaryTemplate[] = [
	{
		id: 'meeting-notes',
		label: 'Minnesanteckning',
		description: 'Kronologisk minnesanteckning, kallprat bortrensat',
		icon: NotebookPen,
		builtin: true,
		sampling: STRICT_SAMPLING,
		systemPrompt:
			`${COMMON_PREAMBLE}\n\n` +
			'Skapa en kronologisk minnesanteckning av mötet.\n\n' +
			'FORMAT: Skriv i löpande text och korta stycken med tydliga, beskrivande underrubriker ' +
			'för varje nytt ämne som diskuteras, i den ordning ämnena togs upp. Använd punktlistor ' +
			'under en rubrik ENDAST om konkreta beslut eller beslutade åtgärder nämns.\n\n' +
			'STRUKTUR per ämne:\n' +
			'### [Ämne i kronologisk ordning]\n' +
			'Kort sammanfattning i styckeform av vad som diskuterades.\n' +
			'- **Beslut/Åtgärd (om det finns):** vem gör vad och när.\n\n' +
			'TON: saklig, professionell och sammanfattande.'
	},
	{
		id: 'meeting-minutes',
		label: 'Mötesprotokoll',
		description: 'Strukturerat protokoll med deltagare, beslut och åtgärder',
		icon: FileText,
		builtin: true,
		sampling: STRICT_SAMPLING,
		systemPrompt:
			`${COMMON_PREAMBLE}\n\n` +
			'Skapa ett strukturerat mötesprotokoll med följande rubriker (utelämna en rubrik ' +
			'helt om underlag saknas i transkriberingen):\n' +
			'## Sammanfattning – två till fyra meningar om mötets syfte och utfall.\n' +
			'## Deltagare – namngivna personer som tydligt deltar i samtalet.\n' +
			'## Diskussionspunkter – de viktigaste ämnena i kronologisk ordning, kort per punkt.\n' +
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
		sampling: STRICT_SAMPLING,
		systemPrompt:
			`${COMMON_PREAMBLE}\n\n` +
			'Skapa en kort, lättläst sammanfattning som en punktlista med 5–10 punkter, i den ' +
			'ordning ämnena togs upp. Fånga de viktigaste ämnena, slutsatserna och eventuella ' +
			'beslut. Varje punkt ska vara en kort, fristående mening. Inga underrubriker.'
	},
	{
		id: 'action-items',
		label: 'Åtgärdspunkter',
		description: 'Bara att-göra-listan med ansvariga',
		icon: ClipboardList,
		builtin: true,
		sampling: STRICT_SAMPLING,
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
		sampling: STRICT_SAMPLING,
		systemPrompt:
			`${COMMON_PREAMBLE}\n\n` +
			'Lista alla beslut som fattas i transkriberingen, i kronologisk ordning. För varje ' +
			'beslut, ange på en egen rad: **Beslut:** vad som beslutades, och **Motivering:** den ' +
			'motivering eller det resonemang som framgår av samtalet. Utelämna motivering om den ' +
			'inte framgår. Om inga beslut fattas, skriv "Inga beslut fattades."'
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

/**
 * Prompt för reduce-steget i chunkad sammanfattning. Slår ihop delsammanfattningar
 * (en per avsnitt av en lång transkribering) till ett sammanhängande dokument.
 * Kronologin från delarna bevaras strikt.
 */
export const REDUCE_PROMPT =
	'Du får flera delsammanfattningar av olika avsnitt av samma möte, i kronologisk ordning ' +
	'(separerade med "---"). Slå ihop dem till ETT enda sammanhängande dokument.\n\n' +
	'- Behåll kronologin strikt: avsnitten kommer i den ordning de inträffade.\n' +
	'- Slå ihop redundans: om samma ämne, beslut eller åtgärd nämns i flera delar, ta med det ' +
	'en gång.\n' +
	'- Bevara alla beslut, åtgärder, namn och siffror exakt.\n' +
	'- Ignorera delar som bara säger "Inget relevant i detta avsnitt".\n' +
	'- Använd tydliga underrubriker per ämne. Lägg inte till nytt innehåll och tolka inte om.\n' +
	'Svara enbart med det sammanslagna dokumentet i Markdown.';

/** Den mall som väljs som standard när vyn öppnas. */
export const DEFAULT_TEMPLATE_ID = 'meeting-notes';

/** Slår upp en inbyggd mall på id. */
export function getBuiltinTemplate(id: string): SummaryTemplate | undefined {
	return SUMMARY_TEMPLATES.find((t) => t.id === id);
}
