<script lang="ts">
	import { onMount } from 'svelte';
	import { Settings } from '@lucide/svelte';
	import {
		listLocalModels,
		listAvailableModels,
		startServer,
		downloadModel,
		deleteModel,
		getSystemInfo,
		onDownloadProgress,
		type ModelInfo,
		type ModelStatus,
		type DownloadProgress,
		type SystemInfo
	} from '$lib/tauri-bridge';
	import { serverStore } from '$lib/stores/server.svelte';
	import { settingsStore } from '$lib/stores/settings.svelte';
	import { DialogChatSettings } from '$lib/components/app';

	let { onServerStarted, onCancel }: { onServerStarted?: () => void; onCancel?: () => void } =
		$props();

	let settingsOpen = $state(false);

	let localModels = $state<ModelInfo[]>([]);
	let availableModels = $state<ModelStatus[]>([]);
	let systemInfo = $state<SystemInfo>({ total_ram_gb: 0, available_ram_gb: 0 });
	let loading = $state(true);
	let starting = $state(false);
	let downloading = $state<string | null>(null);
	let downloadProgress = $state<Record<string, number>>({});
	let deleting = $state<string | null>(null);
	let error = $state<string | null>(null);
	let unlistenProgress: (() => void) | null = null;

	onMount(async () => {
		unlistenProgress = await onDownloadProgress((p: DownloadProgress) => {
			downloadProgress = { ...downloadProgress, [p.model_id]: p.percent };
		});
		await refresh();
		return () => unlistenProgress?.();
	});

	async function refresh() {
		loading = true;
		error = null;
		try {
			[localModels, availableModels, systemInfo] = await Promise.all([
				listLocalModels(),
				listAvailableModels(),
				getSystemInfo()
			]);
		} catch (e) {
			error = e instanceof Error ? e.message : String(e);
		} finally {
			loading = false;
		}
	}

	async function handleStart(modelPath: string) {
		starting = true;
		error = null;
		try {
			const contextSize = settingsStore.config.contextSize as number;
			const gpuIndex = settingsStore.config.gpuIndex as number;
			await startServer(modelPath, contextSize, gpuIndex);
			serverStore.activeGpuIndex = gpuIndex ?? -1;
			serverStore.currentModelPath = modelPath;
			await serverStore.fetch();
			onServerStarted?.();
		} catch (e) {
			error = e instanceof Error ? e.message : String(e);
			starting = false;
		}
	}

	async function handleDownload(modelId: string) {
		downloading = modelId;
		downloadProgress = { ...downloadProgress, [modelId]: 0 };
		error = null;
		try {
			await downloadModel(modelId);
			await refresh();
		} catch (e) {
			error = e instanceof Error ? e.message : String(e);
		} finally {
			downloading = null;
			const { [modelId]: _, ...rest } = downloadProgress;
			downloadProgress = rest;
		}
	}

	async function handleDelete(modelId: string, e: MouseEvent) {
		e.stopPropagation();
		deleting = modelId;
		error = null;
		try {
			await deleteModel(modelId);
			await refresh();
		} catch (err) {
			error = err instanceof Error ? err.message : String(err);
		} finally {
			deleting = null;
		}
	}

	function formatSize(bytes: number): string {
		const gb = bytes / (1024 * 1024 * 1024);
		return gb >= 1 ? `${gb.toFixed(1)} GB` : `${(bytes / (1024 * 1024)).toFixed(0)} MB`;
	}

	// Registry-modeller som är nedladdade (för radera-knapp)
	let registryDownloaded = $derived(availableModels.filter((m) => m.downloaded));
	let notDownloaded = $derived(availableModels.filter((m) => !m.downloaded));

	const flavorColors: Record<string, string> = {
		Kompakt: 'bg-blue-500/15 text-blue-600 dark:text-blue-400',
		Effektiv: 'bg-teal-500/15 text-teal-600 dark:text-teal-400',
		Analytisk: 'bg-orange-500/15 text-orange-600 dark:text-orange-400',
		Balanserad: 'bg-violet-500/15 text-violet-600 dark:text-violet-400',
		Kraftfull: 'bg-red-500/15 text-red-600 dark:text-red-400'
	};

	// RAM-status mot TOTALT systemminne (inte bara ledigt): en modell kan köras på CPU
	// så länge systemet har minnet, även om iGPU:ns VRAM inte räcker. Vi BLOCKERAR aldrig
	// – bara varnar. "tight" = knappt/kräver CPU (informativ varning), aldrig "insufficient".
	function ramStatus(model: ModelStatus): 'ok' | 'tight' {
		if (!systemInfo.total_ram_gb || !model.ram_required_gb) return 'ok';
		return model.ram_required_gb > systemInfo.total_ram_gb * 0.9 ? 'tight' : 'ok';
	}
</script>

<DialogChatSettings bind:open={settingsOpen} />

<div class="fixed inset-0 z-[9999] flex items-start justify-center overflow-y-auto bg-background">
	<div class="w-full max-w-lg space-y-6 p-8">
		<div class="flex items-start justify-between">
			<div class="space-y-1">
				<h1 class="text-2xl font-semibold tracking-tight text-foreground">Välj en modell</h1>
				<p class="text-sm text-muted-foreground">Klicka på en nedladdad modell för att starta.</p>
			</div>
			<div class="flex items-center gap-1">
				<button
					onclick={() => (settingsOpen = true)}
					class="rounded p-1 text-muted-foreground transition-colors hover:text-foreground"
					title="Inställningar"
				>
					<Settings class="h-5 w-5" />
				</button>
				{#if onCancel}
					<button
						onclick={onCancel}
						class="rounded p-1 text-muted-foreground transition-colors hover:text-foreground"
						title="Stäng"
					>
						<svg
							class="h-5 w-5"
							viewBox="0 0 24 24"
							fill="none"
							stroke="currentColor"
							stroke-width="2"
							stroke-linecap="round"
							stroke-linejoin="round"
							><line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" /></svg
						>
					</button>
				{/if}
			</div>
		</div>

		{#if error}
			<div class="rounded-md bg-destructive/10 p-3 text-sm text-destructive">{error}</div>
		{/if}

		{#if loading}
			<div class="animate-pulse text-sm text-muted-foreground">Laddar modeller…</div>
		{:else if starting}
			<div class="space-y-2">
				<div class="text-sm text-muted-foreground">Startar modellen, vänta…</div>
				<div class="h-1.5 w-full overflow-hidden rounded-full bg-muted">
					<div class="h-full w-1/2 animate-pulse rounded-full bg-primary"></div>
				</div>
			</div>
		{:else}
			<!-- Downloaded registry models -->
			{#if registryDownloaded.length > 0}
				<div class="space-y-2">
					<p class="text-xs font-medium tracking-wide text-muted-foreground uppercase">
						Nedladdade modeller
					</p>
					<div class="space-y-1.5">
						{#each registryDownloaded as model}
							{@const localMatch = localModels.find((l) => l.filename === model.filename)}
							<div class="group relative rounded-md border border-border">
								<button
									onclick={() => localMatch && handleStart(localMatch.path)}
									disabled={!localMatch || deleting === model.id}
									class="w-full rounded-md p-3 text-left transition-colors hover:bg-accent disabled:opacity-60"
								>
									<div class="flex items-center justify-between pr-6">
										<div class="flex items-center gap-2">
											<span class="text-sm font-medium text-foreground">{model.name}</span>
											{#if model.flavor}
												<span
													class="rounded-full px-2 py-0.5 text-xs font-medium {flavorColors[
														model.flavor
													] ?? 'bg-muted text-muted-foreground'}">{model.flavor}</span
												>
											{/if}
											{#if ramStatus(model) === 'tight'}
												<span
													title="Kräver ~{model.ram_required_gb} GB RAM (du har {systemInfo.total_ram_gb.toFixed(
														0
													)} GB). Körs på CPU om GPU-minnet inte räcker – långsammare."
													class="text-xs text-yellow-500">⚠ kräver mycket RAM</span
												>
											{/if}
										</div>
										<span class="text-xs text-muted-foreground">{formatSize(model.size_bytes)}</span
										>
									</div>
									{#if model.description}
										<p class="mt-0.5 text-xs text-muted-foreground">{model.description}</p>
									{/if}
								</button>
								<!-- Delete button -->
								<button
									onclick={(e) => handleDelete(model.id, e)}
									disabled={deleting === model.id}
									title="Radera modell"
									class="absolute top-1/2 right-2.5 -translate-y-1/2 rounded p-1 text-muted-foreground opacity-0 transition-opacity group-hover:opacity-100 hover:text-destructive disabled:opacity-40"
								>
									{#if deleting === model.id}
										<svg
											class="h-3.5 w-3.5 animate-spin"
											viewBox="0 0 24 24"
											fill="none"
											stroke="currentColor"
											stroke-width="2"
											><circle cx="12" cy="12" r="10" stroke-opacity="0.25" /><path
												d="M12 2a10 10 0 0 1 10 10"
											/></svg
										>
									{:else}
										<svg
											class="h-3.5 w-3.5"
											viewBox="0 0 24 24"
											fill="none"
											stroke="currentColor"
											stroke-width="2"
											stroke-linecap="round"
											stroke-linejoin="round"
											><polyline points="3 6 5 6 21 6" /><path
												d="M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6"
											/><path d="M10 11v6M14 11v6" /><path d="M9 6V4h6v2" /></svg
										>
									{/if}
								</button>
							</div>
						{/each}
					</div>
				</div>
			{/if}

			<!-- Local-only GGUF files not in registry -->
			{#if localModels.filter((l) => !registryDownloaded.find((r) => r.filename === l.filename)).length > 0}
				<div class="space-y-2">
					<p class="text-xs font-medium tracking-wide text-muted-foreground uppercase">
						Egna modeller
					</p>
					<div class="space-y-1.5">
						{#each localModels.filter((l) => !registryDownloaded.find((r) => r.filename === l.filename)) as model}
							<button
								onclick={() => handleStart(model.path)}
								class="w-full rounded-md border border-border p-3 text-left transition-colors hover:bg-accent"
							>
								<div class="flex items-center justify-between">
									<span class="text-sm font-medium text-foreground">{model.name}</span>
									<span class="text-xs text-muted-foreground">{formatSize(model.size_bytes)}</span>
								</div>
							</button>
						{/each}
					</div>
				</div>
			{/if}

			<!-- Available to download -->
			{#if notDownloaded.length > 0}
				<div class="space-y-2">
					<p class="text-xs font-medium tracking-wide text-muted-foreground uppercase">
						Tillgängliga att ladda ner
					</p>
					<div class="space-y-1.5">
						{#each notDownloaded as model}
							<div class="rounded-md border border-border p-3">
								<div class="flex items-center justify-between">
									<div>
										<div class="flex items-center gap-2">
											<span class="text-sm font-medium text-foreground">{model.name}</span>
											{#if model.flavor}
												<span
													class="rounded-full px-2 py-0.5 text-xs font-medium {flavorColors[
														model.flavor
													] ?? 'bg-muted text-muted-foreground'}">{model.flavor}</span
												>
											{/if}
											{#if ramStatus(model) === 'tight'}
												<span
													title="Kräver ~{model.ram_required_gb} GB RAM (du har {systemInfo.total_ram_gb.toFixed(
														0
													)} GB). Körs på CPU om GPU-minnet inte räcker – långsammare."
													class="text-xs text-yellow-500"
													>⚠ kräver ~{model.ram_required_gb} GB RAM</span
												>
											{/if}
										</div>
										{#if model.size_bytes > 0}
											<span class="ml-2 text-xs text-muted-foreground"
												>{formatSize(model.size_bytes)}</span
											>
										{/if}
										{#if model.description}
											<p class="text-xs text-muted-foreground">{model.description}</p>
										{/if}
									</div>
									<button
										onclick={() => handleDownload(model.id)}
										disabled={downloading !== null}
										class="rounded bg-primary px-3 py-1 text-xs font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
									>
										{downloading === model.id ? 'Laddar ner…' : 'Ladda ner'}
									</button>
								</div>

								<!-- Progress bar -->
								{#if downloading === model.id}
									{@const pct = downloadProgress[model.id] ?? 0}
									<div class="mt-2 space-y-1">
										<div class="h-1.5 w-full overflow-hidden rounded-full bg-muted">
											<div
												class="h-full rounded-full bg-primary transition-all duration-300"
												style="width: {pct}%"
											></div>
										</div>
										<p class="text-xs text-muted-foreground">{pct.toFixed(0)}%</p>
									</div>
								{/if}
							</div>
						{/each}
					</div>
				</div>
			{/if}

			{#if localModels.length === 0 && notDownloaded.length === 0 && registryDownloaded.length === 0}
				<p class="text-sm text-muted-foreground">Inga modeller hittades.</p>
			{/if}
		{/if}

		{#if systemInfo.total_ram_gb > 0}
			<p class="text-xs text-muted-foreground">
				RAM: {systemInfo.available_ram_gb.toFixed(1)} GB tillgängligt / {systemInfo.total_ram_gb.toFixed(
					1
				)} GB totalt
			</p>
		{/if}
	</div>
</div>
