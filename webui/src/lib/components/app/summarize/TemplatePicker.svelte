<script lang="ts">
	import { summaryTemplatesStore } from '$lib/stores/summary-templates.svelte';
	import { cn } from '$lib/components/ui/utils';
	import { Plus, Pencil } from '@lucide/svelte';

	interface Props {
		selectedId: string;
		onSelect: (id: string) => void;
		onCreate: () => void;
		onEdit: (id: string) => void;
	}

	let { selectedId, onSelect, onCreate, onEdit }: Props = $props();

	let all = $derived(summaryTemplatesStore.all);
</script>

<div class="grid grid-cols-2 gap-3 sm:grid-cols-4">
	{#each all as template (template.id)}
		{@const Icon = template.icon}
		<div
			class={cn(
				'group relative flex flex-col items-start gap-2 rounded-xl border p-4 transition-all',
				selectedId === template.id
					? 'border-primary bg-primary/5 shadow-sm ring-1 ring-primary/20'
					: 'border-border bg-card hover:border-primary/60 hover:bg-accent/40'
			)}
		>
			<button
				type="button"
				onclick={() => onSelect(template.id)}
				class="flex w-full flex-col items-start gap-2 text-left focus-visible:outline-none"
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
			{#if !template.builtin}
				<button
					type="button"
					title="Redigera mall"
					onclick={() => onEdit(template.id)}
					class="absolute top-2 right-2 rounded p-1 text-muted-foreground opacity-0 transition-opacity group-hover:opacity-100 hover:text-foreground"
				>
					<Pencil class="size-3.5" />
				</button>
			{/if}
		</div>
	{/each}

	<!-- Skapa egen mall -->
	<button
		type="button"
		onclick={onCreate}
		class="flex flex-col items-center justify-center gap-2 rounded-xl border border-dashed border-border p-4 text-muted-foreground transition-all hover:border-primary/60 hover:text-foreground"
	>
		<Plus class="size-5" />
		<span class="text-sm font-medium">Egen mall</span>
	</button>
</div>
