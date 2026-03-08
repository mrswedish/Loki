<script lang="ts">
	import { Button } from '$lib/components/ui/button';
	import * as AlertDialog from '$lib/components/ui/alert-dialog';
	import { settingsStore } from '$lib/stores/settings.svelte';
	import { RotateCcw } from '@lucide/svelte';

	interface Props {
		onReset?: () => void;
		onSave?: () => void;
	}

	let { onReset, onSave }: Props = $props();

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

	<Button onclick={handleSave}>Spara inställningar</Button>
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
