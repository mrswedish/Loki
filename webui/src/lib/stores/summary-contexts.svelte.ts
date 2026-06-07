import { settingsStore, config } from '$lib/stores/settings.svelte';

/**
 * Hanterar användarens sparade kontext-förval för sammanfattningsläget.
 *
 * Ett förval är en namngiven beskrivning av ett mötes domän/sammanhang (t.ex.
 * "Rehabmöte sjukvård" → "Avstämningsmöte i rehabprocessen inom regionvården…").
 * Användaren kan välja ett förval med ett klick i stället för att skriva om samma
 * domän varje gång. Lagras som en JSON-array i settings-nyckeln `summaryContexts`
 * (samma mönster som `summaryTemplates`).
 */

export interface SummaryContext {
	/** Stabilt id. */
	id: string;
	/** Kort namn som visas på förvals-chipet. */
	label: string;
	/** Själva kontexttexten som fyller fältet. */
	text: string;
}

/** Genererar ett stabilt id för ett nytt förval. */
function newId(): string {
	return `ctx-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 7)}`;
}

class SummaryContextsStore {
	/** Alla sparade förval, lästa från settings. */
	get all(): SummaryContext[] {
		try {
			const raw = config().summaryContexts;
			const parsed = typeof raw === 'string' ? JSON.parse(raw) : [];
			if (!Array.isArray(parsed)) return [];
			return parsed.filter(
				(c) =>
					c &&
					typeof c.id === 'string' &&
					typeof c.label === 'string' &&
					typeof c.text === 'string'
			) as SummaryContext[];
		} catch {
			return [];
		}
	}

	private persist(list: SummaryContext[]): void {
		settingsStore.updateConfig('summaryContexts', JSON.stringify(list));
	}

	/** Skapar ett nytt förval och returnerar dess id. */
	add(label: string, text: string): string {
		const id = newId();
		const entry: SummaryContext = {
			id,
			label: label.trim() || 'Sammanhang',
			text: text.trim()
		};
		this.persist([...this.all, entry]);
		return id;
	}

	/** Raderar ett förval. */
	remove(id: string): void {
		this.persist(this.all.filter((c) => c.id !== id));
	}
}

export const summaryContextsStore = new SummaryContextsStore();
