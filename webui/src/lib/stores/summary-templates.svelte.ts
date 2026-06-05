import { settingsStore, config } from '$lib/stores/settings.svelte';
import { SUMMARY_TEMPLATES, type SummaryTemplate } from '$lib/constants/summary-templates';
import type { TemplateSampling } from '$lib/constants/model-sampling';

/**
 * Hanterar användarens egna sammanfattningsmallar (utöver de inbyggda).
 *
 * Egna mallar lagras som en JSON-array i settings-nyckeln `summaryTemplates`
 * (samma mönster som `mcpServers`). De har `builtin: false` och kan skapas,
 * redigeras och raderas av användaren. Inbyggda mallar är skrivskyddade.
 */

/** Genererar ett stabilt id för en ny egen mall. */
function newId(): string {
	return `custom-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 7)}`;
}

class SummaryTemplatesStore {
	/** Användarens egna mallar, lästa från settings. */
	get custom(): SummaryTemplate[] {
		try {
			const raw = config().summaryTemplates;
			const parsed = typeof raw === 'string' ? JSON.parse(raw) : [];
			if (!Array.isArray(parsed)) return [];
			// Säkerställ att inlästa mallar är markerade som egna (builtin: false).
			return parsed
				.filter((t) => t && typeof t.id === 'string' && typeof t.systemPrompt === 'string')
				.map((t) => ({ ...t, builtin: false }) as SummaryTemplate);
		} catch {
			return [];
		}
	}

	/** Alla mallar: inbyggda först, sedan användarens egna. */
	get all(): SummaryTemplate[] {
		return [...SUMMARY_TEMPLATES, ...this.custom];
	}

	private persist(templates: SummaryTemplate[]): void {
		// Spara bara fälten vi äger (inte ev. icke-serialiserbara ikon-komponenter).
		const plain = templates.map((t) => ({
			id: t.id,
			label: t.label,
			description: t.description,
			systemPrompt: t.systemPrompt,
			builtin: false,
			sampling: t.sampling
		}));
		settingsStore.updateConfig('summaryTemplates', JSON.stringify(plain));
	}

	/** Skapar en ny egen mall och returnerar dess id. */
	create(data: {
		label: string;
		description: string;
		systemPrompt: string;
		sampling?: TemplateSampling;
	}): string {
		const id = newId();
		const template: SummaryTemplate = {
			id,
			label: data.label.trim() || 'Egen mall',
			description: data.description.trim(),
			systemPrompt: data.systemPrompt.trim(),
			builtin: false,
			sampling: data.sampling
		};
		this.persist([...this.custom, template]);
		return id;
	}

	/**
	 * Duplicerar en befintlig mall (inbyggd eller egen) till en ny redigerbar egen
	 * mall. För inbyggda mallar måste promptens fulla text (inkl. trohets-preambeln)
	 * fångas så att kopian beter sig likadant – inbyggda mallar har preambeln inbakad
	 * i systemPrompt, så vi kopierar den rakt av. Returnerar den nya mallens id.
	 */
	duplicate(id: string): string | null {
		const source = this.get(id);
		if (!source) return null;
		return this.create({
			label: `${source.label} (kopia)`,
			description: source.description,
			systemPrompt: source.systemPrompt,
			sampling: source.sampling
		});
	}

	/** Uppdaterar en befintlig egen mall. */
	update(
		id: string,
		data: {
			label: string;
			description: string;
			systemPrompt: string;
			sampling?: TemplateSampling;
		}
	): void {
		const next = this.custom.map((t) =>
			t.id === id
				? {
						...t,
						label: data.label.trim() || t.label,
						description: data.description.trim(),
						systemPrompt: data.systemPrompt.trim(),
						sampling: data.sampling
					}
				: t
		);
		this.persist(next);
	}

	/** Raderar en egen mall. */
	remove(id: string): void {
		this.persist(this.custom.filter((t) => t.id !== id));
	}

	/** Slår upp en mall (inbyggd eller egen) på id. */
	get(id: string): SummaryTemplate | undefined {
		return this.all.find((t) => t.id === id);
	}
}

export const summaryTemplatesStore = new SummaryTemplatesStore();
