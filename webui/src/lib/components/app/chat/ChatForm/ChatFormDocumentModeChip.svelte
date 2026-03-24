<script lang="ts">
	import { FileSearch, BookOpen, Wand2, Info } from '@lucide/svelte';
	import * as Tooltip from '$lib/components/ui/tooltip';
	import { chatStore, type DocumentMode } from '$lib/stores/chat.svelte';

	interface Props {
		class?: string;
	}

	let { class: className = '' }: Props = $props();

	const modes: Array<{
		value: DocumentMode;
		label: string;
		shortLabel: string;
		icon: typeof FileSearch;
		description: string;
		useCases: string[];
	}> = [
		{
			value: 'auto',
			label: 'Auto',
			shortLabel: 'Auto',
			icon: Wand2,
			description: 'Väljer läge automatiskt baserat på din fråga.',
			useCases: ['Fungerar bra i de flesta fall']
		},
		{
			value: 'extract',
			label: 'Extrahera',
			shortLabel: 'Extrahera',
			icon: FileSearch,
			description: 'Söker igenom dokumentet efter information kopplad till din specifika fråga.',
			useCases: [
				'Hitta specifika beslut eller fakta',
				'Vem ansvarar för X?',
				'Lista alla action points',
				'När hände Y?'
			]
		},
		{
			value: 'summarize',
			label: 'Sammanfatta',
			shortLabel: 'Sammanfatta',
			icon: BookOpen,
			description: 'Läser dokumentet i sin helhet och bygger en sammanhängande helhetsbild.',
			useCases: [
				'Mötesprotokoll och rapporter',
				'Vad handlar dokumentet om?',
				'Ge mig en översikt',
				'Sammanfatta detta avtal'
			]
		}
	];

	let current = $derived(modes.find((m) => m.value === chatStore.documentMode) ?? modes[0]);

	function cycle() {
		const idx = modes.findIndex((m) => m.value === chatStore.documentMode);
		chatStore.setDocumentMode(modes[(idx + 1) % modes.length].value);
	}
</script>

<div class="flex items-center gap-1 {className}">
	<button
		type="button"
		onclick={cycle}
		class="flex items-center gap-1.5 rounded-full border border-border/60 bg-muted/50 px-2.5 py-1 text-xs text-muted-foreground transition-colors hover:border-border hover:bg-muted hover:text-foreground"
		title="Klicka för att byta dokumentanalysläge"
	>
		<current.icon class="h-3 w-3 flex-shrink-0" />
		<span>{current.shortLabel}</span>
	</button>

	<Tooltip.Root>
		<Tooltip.Trigger
			class="flex h-5 w-5 items-center justify-center rounded-full text-muted-foreground/60 transition-colors hover:text-muted-foreground"
		>
			<Info class="h-3 w-3" />
			<span class="sr-only">Information om dokumentanalyslägen</span>
		</Tooltip.Trigger>
		<Tooltip.Content class="max-w-xs p-3" side="top">
			<div class="space-y-3">
				{#each modes as mode (mode.value)}
					<div class:opacity-40={current.value !== mode.value && mode.value !== 'auto'}>
						<div class="mb-1 flex items-center gap-1.5">
							<mode.icon class="h-3.5 w-3.5 flex-shrink-0" />
							<span class="font-medium">{mode.label}</span>
							{#if mode.value === current.value}
								<span class="ml-auto rounded-full bg-primary/20 px-1.5 py-0.5 text-[10px] text-primary">aktiv</span>
							{/if}
						</div>
						<p class="mb-1 text-xs text-muted-foreground">{mode.description}</p>
						{#if mode.useCases.length > 0}
							<ul class="space-y-0.5">
								{#each mode.useCases as uc (uc)}
									<li class="text-[11px] text-muted-foreground/80">· {uc}</li>
								{/each}
							</ul>
						{/if}
					</div>
					{#if mode.value !== 'summarize'}
						<div class="border-t border-border/30"></div>
					{/if}
				{/each}
			</div>
		</Tooltip.Content>
	</Tooltip.Root>
</div>
