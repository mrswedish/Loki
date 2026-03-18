<script lang="ts">
	import { onMount } from 'svelte';
	import { RefreshCw, HardDrive, CheckCircle, AlertCircle } from '@lucide/svelte';
	import { Button } from '$lib/components/ui/button';
	import { getServerBinaryVersion, updateServerBinary, stopServer, startServer } from '$lib/tauri-bridge';
	import { serverStore } from '$lib/stores/server.svelte';
	import { toast } from 'svelte-sonner';

	let installedVersion: string | null = $state(null);
	let loading = $state(true);
	let updating = $state(false);

	onMount(async () => {
		installedVersion = await getServerBinaryVersion();
		loading = false;
	});

	async function handleUpdate() {
		updating = true;
		try {
			await updateServerBinary();
			installedVersion = null;
			toast.info('Binären raderad – ny version laddas ned vid nästa start.');

			const modelPath = serverStore.props?.model_path;
			if (modelPath) {
				toast.info('Startar om servern...');
				await stopServer();
				const ctx = serverStore.contextSize ?? 4096;
				const gpu = serverStore.activeGpuIndex ?? -1;
				await startServer(modelPath, ctx, gpu);
				await serverStore.fetch();
				installedVersion = await getServerBinaryVersion();
				toast.success('Klar! llama-server uppdaterad till ' + (installedVersion ?? 'okänd version'));
			} else {
				toast.success('Binären raderas – ny version hämtas när du startar nästa modell.');
			}
		} catch (e) {
			toast.error('Uppdatering misslyckades: ' + (e instanceof Error ? e.message : String(e)));
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
			{#if loading}
				<span class="text-sm text-muted-foreground">Kontrollerar version...</span>
			{:else if installedVersion}
				<CheckCircle class="h-4 w-4 text-green-500" />
				<span class="text-sm">Installerad version: <code class="font-mono">{installedVersion}</code></span>
			{:else}
				<AlertCircle class="h-4 w-4 text-yellow-500" />
				<span class="text-sm text-muted-foreground">Ingen binär installerad än</span>
			{/if}
		</div>

		<Button
			variant="outline"
			size="sm"
			onclick={handleUpdate}
			disabled={updating || loading}
			class="gap-2"
		>
			<RefreshCw class="h-4 w-4 {updating ? 'animate-spin' : ''}" />
			{updating ? 'Uppdaterar...' : 'Uppdatera till senaste version'}
		</Button>

		<p class="mt-3 text-xs text-muted-foreground">
			Laddar ned senaste llama-server från ggml-org/llama.cpp och startar om.
			Tar ~30–60 sekunder beroende på anslutning.
		</p>
	</div>
</div>
