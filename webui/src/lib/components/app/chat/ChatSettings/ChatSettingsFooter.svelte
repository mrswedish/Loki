<script lang="ts">
	import { Button } from '$lib/components/ui/button';
	import * as AlertDialog from '$lib/components/ui/alert-dialog';
	import { settingsStore } from '$lib/stores/settings.svelte';
	import { RotateCcw } from '@lucide/svelte';

	interface Props {
		onReset?: () => void;
		onSave?: () => void;
		onSaveAndRestart?: () => void;
		showRestartButton?: boolean;
	}

	let { onReset, onSave, onSaveAndRestart, showRestartButton }: Props = $props();

	let showResetDialog = $state(false);

	function handleResetClick() {
		showResetDialog = true;
	}

	function handleConfirmReset() {
		settingsStore.forceSyncWithServerDefaults();
		onReset?.();

		showResetDialog = false;
	}

	function handleSave() {
		onSave?.();
	}
</script>

<div class="flex justify-between border-t border-border/30 p-6">
	<div class="flex gap-2">
		<Button variant="outline" onclick={handleResetClick}>
			<RotateCcw class="h-3 w-3" />

			Återställ till standard
		</Button>
	</div>

	<div class="flex gap-3 items-center">
		{#if showRestartButton}
			<Button
				variant="default"
				onclick={onSaveAndRestart}
				class="bg-gradient-to-r from-primary to-blue-600 hover:from-primary/90 hover:to-blue-600/90 shadow-md font-medium"
			>
				Spara och starta om
			</Button>
		{/if}
		<Button onclick={handleSave} variant={showRestartButton ? 'outline' : 'default'}>
			Spara {!showRestartButton ? 'inställningar' : ''}
		</Button>
	</div>
</div>

<AlertDialog.Root bind:open={showResetDialog}>
	<AlertDialog.Content>
		<AlertDialog.Header>
			<AlertDialog.Title>Återställ inställningar till standard</AlertDialog.Title>
			<AlertDialog.Description>
				Är du säker på att du vill återställa alla inställningar till standardvärden? Detta återställer
				alla parametrar till värdena från serverns /props-endpoint och tar bort dina egna konfigurationer.
			</AlertDialog.Description>
		</AlertDialog.Header>
		<AlertDialog.Footer>
			<AlertDialog.Cancel>Avbryt</AlertDialog.Cancel>
			<AlertDialog.Action onclick={handleConfirmReset}>Återställ till standard</AlertDialog.Action>
		</AlertDialog.Footer>
	</AlertDialog.Content>
</AlertDialog.Root>
