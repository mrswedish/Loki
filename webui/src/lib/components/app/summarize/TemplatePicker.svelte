<script lang="ts">
	import { SUMMARY_TEMPLATES, type SummaryTemplate } from '$lib/constants/summary-templates';
	import { cn } from '$lib/components/ui/utils';

	interface Props {
		selectedId: string;
		customTemplates?: SummaryTemplate[];
		onSelect: (id: string) => void;
	}

	let { selectedId, customTemplates = [], onSelect }: Props = $props();

	let all = $derived([...SUMMARY_TEMPLATES, ...customTemplates]);
</script>

<div class="grid grid-cols-2 gap-3 sm:grid-cols-4">
	{#each all as template (template.id)}
		{@const Icon = template.icon}
		<button
			type="button"
			onclick={() => onSelect(template.id)}
			class={cn(
				'group flex flex-col items-start gap-2 rounded-xl border p-4 text-left transition-all',
				'hover:border-primary/60 hover:bg-accent/40 focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none',
				selectedId === template.id
					? 'border-primary bg-primary/5 shadow-sm ring-1 ring-primary/20'
					: 'border-border bg-card'
			)}
		>
			{#if Icon}
				<Icon
					class={cn(
						'size-5 transition-colors',
						selectedId === template.id
							? 'text-primary'
							: 'text-muted-foreground group-hover:text-foreground'
					)}
				/>
			{/if}
			<span class="text-sm font-medium">{template.label}</span>
			<span class="text-xs leading-snug text-muted-foreground">{template.description}</span>
		</button>
	{/each}
</div>
