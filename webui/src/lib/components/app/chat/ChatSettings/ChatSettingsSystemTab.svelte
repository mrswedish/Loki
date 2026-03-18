<script lang="ts">
	import { onMount } from 'svelte';
	import { RefreshCw, HardDrive, CheckCircle, AlertCircle, HelpCircle } from '@lucide/svelte';
	import { Button } from '$lib/components/ui/button';
	import {
		getServerBinaryVersion,
		updateServerBinary,
		serverBinaryExists,
		downloadServerBinary
	} from '$lib/tauri-bridge';
	import { toast } from 'svelte-sonner';

	type BinaryState = 'loading' | 'missing' | 'unknown-version' | 'ok';

	let binaryState = $state<BinaryState>('loading');
	let installedVersion = $state<string | null>(null);
	let updating = $state(false);

	onMount(async () => {
		await refreshState();
	});

	async function refreshState() {
		binaryState = 'loading';
		const version = await getServerBinaryVersion();
		if (version) {
			installedVersion = version;
			binaryState = 'ok';
		} else {
			const exists = await serverBinaryExists();
			binaryState = exists ? 'unknown-version' : 'missing';
		}
	}

	async function handleUpdate() {
		updating = true;
		try {
			// Rensa gammal binär
			await updateServerBinary();
			binaryState = 'missing';
			installedVersion = null;

			// Ladda ned direkt
			toast.info('Laddar ned senaste llama-server...');
			const version = await downloadServerBinary();
			installedVersion = version;
			binaryState = 'ok';
			toast.success('Klar! llama-server ' + version + ' installerad.');
		} catch (e) {
			toast.error('Misslyckades: ' + (e instanceof Error ? e.message : String(e)));
			await refreshState();
		} finally {
			updating = false;
		}
	}
</script>

<div class="space-y-6">
	<div class="rounded-lg border border-border/50 p-4">
		<div class="mb-3 flex items-center gap-2">
			<HardDrive class="h-4 w-4 text-muted-foreground" />
			<span class="text-sm font-medium">llama-server (AI-motor)</span>
		</div>

		<div class="mb-4 flex items-center gap-2">
			{#if binaryState === 'loading'}
				<span class="text-sm text-muted-foreground">Kontrollerar...</span>
			{:else if binaryState === 'ok'}
				<CheckCircle class="h-4 w-4 text-green-500" />
				<span class="text-sm">Version: <code class="font-mono">{installedVersion}</code></span>
			{:else if binaryState === 'unknown-version'}
				<HelpCircle class="h-4 w-4 text-yellow-500" />
				<span class="text-sm text-muted-foreground">Installerad – version okänd (äldre installation)</span>
			{:else}
				<AlertCircle class="h-4 w-4 text-yellow-500" />
				{#if updating}
					<span class="text-sm text-muted-foreground">Laddar ned...</span>
				{:else}
					<span class="text-sm text-muted-foreground">Ingen binär installerad</span>
				{/if}
			{/if}
		</div>

		<Button
			variant="outline"
			size="sm"
			onclick={handleUpdate}
			disabled={updating || binaryState === 'loading'}
			class="gap-2"
		>
			<RefreshCw class="h-4 w-4 {updating ? 'animate-spin' : ''}" />
			{#if updating}
				Laddar ned...
			{:else if binaryState === 'missing'}
				Ladda ned llama-server
			{:else}
				Uppdatera till senaste version
			{/if}
		</Button>

		<p class="mt-3 text-xs text-muted-foreground">
			Hämtar senaste llama-server från ggml-org/llama.cpp.
			Tar ~30–60 sekunder beroende på anslutning.
		</p>
	</div>
</div>
