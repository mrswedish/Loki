<script lang="ts">
	import { Brain } from '@lucide/svelte';
	import { Button } from '$lib/components/ui/button';
	import * as Tooltip from '$lib/components/ui/tooltip';
	import { config, settingsStore } from '$lib/stores/settings.svelte';

	interface Props {
		class?: string;
		disabled?: boolean;
	}

	let { class: className = '', disabled = false }: Props = $props();

	// enableThinking saknar explicit värde = på (modellens default följs).
	let enabled = $derived(config().enableThinking !== false);

	function toggle() {
		settingsStore.updateConfig('enableThinking', !enabled);
	}
</script>

<div class="flex items-center gap-1 {className}">
	<Tooltip.Root>
		<Tooltip.Trigger>
			<Button
				type="button"
				variant="ghost"
				{disabled}
				onclick={toggle}
				aria-pressed={enabled}
				class="h-8 w-8 rounded-full p-0 {enabled
					? 'bg-primary/10 text-primary hover:bg-primary/20'
					: 'text-muted-foreground'}"
			>
				<span class="sr-only">{enabled ? 'Stäng av tänkande' : 'Slå på tänkande'}</span>
				<Brain class="h-4 w-4" />
			</Button>
		</Tooltip.Trigger>

		<Tooltip.Content>
			<p>Tänkande (reasoning): {enabled ? 'på' : 'av'}</p>
		</Tooltip.Content>
	</Tooltip.Root>
</div>
