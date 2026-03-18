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

	let { onServerStarted, onCancel }: { onServerStarted?: () => void; onCancel?: () => void } = $props();

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
		'Kompakt': 'bg-blue-500/15 text-blue-600 dark:text-blue-400',
		'Effektiv': 'bg-teal-500/15 text-teal-600 dark:text-teal-400',
		'Analytisk': 'bg-orange-500/15 text-orange-600 dark:text-orange-400',
		'Balanserad': 'bg-violet-500/15 text-violet-600 dark:text-violet-400',
		'Kraftfull': 'bg-red-500/15 text-red-600 dark:text-red-400',
	};

	function ramStatus(model: ModelStatus): 'ok' | 'tight' | 'insufficient' {
		if (!systemInfo.available_ram_gb || !model.ram_required_gb) return 'ok';
		const avail = systemInfo.available_ram_gb;
		const req = model.ram_required_gb;
		if (req > avail * 1.1) return 'insufficient';
		if (req > avail * 0.85) return 'tight';
		return 'ok';
	}
</script>

<DialogChatSettings bind:open={settingsOpen} />

<div class="bg-background fixed inset-0 z-[9999] flex items-start justify-center overflow-y-auto">
	<div class="w-full max-w-lg space-y-6 p-8">
		<div class="flex items-start justify-between">
			<div class="space-y-1">
				<h1 class="text-foreground text-2xl font-semibold tracking-tight">Välj en modell</h1>
				<p class="text-muted-foreground text-sm">Klicka på en nedladdad modell för att starta.</p>
			</div>
			<div class="flex items-center gap-1">
				<button
					onclick={() => (settingsOpen = true)}
					class="text-muted-foreground hover:text-foreground rounded p-1 transition-colors"
					title="Inställningar"
				>
					<Settings class="h-5 w-5" />
				</button>
				{#if onCancel}
					<button
						onclick={onCancel}
						class="text-muted-foreground hover:text-foreground rounded p-1 transition-colors"
						title="Stäng"
					>
						<svg class="h-5 w-5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
					</button>
				{/if}
			</div>
		</div>

		{#if error}
			<div class="bg-destructive/10 text-destructive rounded-md p-3 text-sm">{error}</div>
		{/if}

		{#if loading}
			<div class="text-muted-foreground animate-pulse text-sm">Laddar modeller…</div>
		{:else if starting}
			<div class="space-y-2">
				<div class="text-muted-foreground text-sm">Startar modellen, vänta…</div>
				<div class="bg-muted h-1.5 w-full overflow-hidden rounded-full">
					<div class="bg-primary h-full w-1/2 animate-pulse rounded-full"></div>
				</div>
			</div>
		{:else}
			<!-- Downloaded registry models -->
			{#if registryDownloaded.length > 0}
				<div class="space-y-2">
					<p class="text-muted-foreground text-xs font-medium uppercase tracking-wide">
						Nedladdade modeller
					</p>
					<div class="space-y-1.5">
						{#each registryDownloaded as model}
							{@const localMatch = localModels.find((l) => l.filename === model.filename)}
							<div class="border-border group relative rounded-md border">
								<button
									onclick={() => localMatch && handleStart(localMatch.path)}
									disabled={!localMatch || deleting === model.id}
									class="hover:bg-accent w-full rounded-md p-3 text-left transition-colors disabled:opacity-60"
								>
									<div class="flex items-center justify-between pr-6">
										<div class="flex items-center gap-2">
											<span class="text-foreground text-sm font-medium">{model.name}</span>
											{#if model.flavor}
												<span class="rounded-full px-2 py-0.5 text-xs font-medium {flavorColors[model.flavor] ?? 'bg-muted text-muted-foreground'}">{model.flavor}</span>
											{/if}
											{#if ramStatus(model) === 'insufficient'}
												<span title="Troligen för lite RAM ({model.ram_required_gb} GB krävs, {systemInfo.available_ram_gb.toFixed(1)} GB tillgängligt)" class="text-red-500 text-xs">⚠ RAM</span>
											{:else if ramStatus(model) === 'tight'}
												<span title="Kan vara trög ({model.ram_required_gb} GB krävs, {systemInfo.available_ram_gb.toFixed(1)} GB tillgängligt)" class="text-yellow-500 text-xs">⚠</span>
											{/if}
										</div>
										<span class="text-muted-foreground text-xs">{formatSize(model.size_bytes)}</span>
									</div>
									{#if model.description}
										<p class="text-muted-foreground mt-0.5 text-xs">{model.description}</p>
									{/if}
								</button>
								<!-- Delete button -->
								<button
									onclick={(e) => handleDelete(model.id, e)}
									disabled={deleting === model.id}
									title="Radera modell"
									class="text-muted-foreground hover:text-destructive absolute right-2.5 top-1/2 -translate-y-1/2 rounded p-1 opacity-0 transition-opacity group-hover:opacity-100 disabled:opacity-40"
								>
									{#if deleting === model.id}
										<svg class="h-3.5 w-3.5 animate-spin" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="10" stroke-opacity="0.25"/><path d="M12 2a10 10 0 0 1 10 10" /></svg>
									{:else}
										<svg class="h-3.5 w-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6"/><path d="M10 11v6M14 11v6"/><path d="M9 6V4h6v2"/></svg>
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
					<p class="text-muted-foreground text-xs font-medium uppercase tracking-wide">
						Egna modeller
					</p>
					<div class="space-y-1.5">
						{#each localModels.filter((l) => !registryDownloaded.find((r) => r.filename === l.filename)) as model}
							<button
								onclick={() => handleStart(model.path)}
								class="border-border hover:bg-accent w-full rounded-md border p-3 text-left transition-colors"
							>
								<div class="flex items-center justify-between">
									<span class="text-foreground text-sm font-medium">{model.name}</span>
									<span class="text-muted-foreground text-xs">{formatSize(model.size_bytes)}</span>
								</div>
							</button>
						{/each}
					</div>
				</div>
			{/if}

			<!-- Available to download -->
			{#if notDownloaded.length > 0}
				<div class="space-y-2">
					<p class="text-muted-foreground text-xs font-medium uppercase tracking-wide">
						Tillgängliga att ladda ner
					</p>
					<div class="space-y-1.5">
						{#each notDownloaded as model}
							<div class="border-border rounded-md border p-3">
								<div class="flex items-center justify-between">
									<div>
										<div class="flex items-center gap-2">
											<span class="text-foreground text-sm font-medium">{model.name}</span>
											{#if model.flavor}
												<span class="rounded-full px-2 py-0.5 text-xs font-medium {flavorColors[model.flavor] ?? 'bg-muted text-muted-foreground'}">{model.flavor}</span>
											{/if}
											{#if ramStatus(model) === 'insufficient'}
												<span title="Troligen för lite RAM ({model.ram_required_gb} GB krävs, {systemInfo.available_ram_gb.toFixed(1)} GB tillgängligt)" class="text-red-500 text-xs">⚠ RAM</span>
											{:else if ramStatus(model) === 'tight'}
												<span title="Kan vara trög ({model.ram_required_gb} GB krävs, {systemInfo.available_ram_gb.toFixed(1)} GB tillgängligt)" class="text-yellow-500 text-xs">⚠</span>
											{/if}
										</div>
										{#if model.size_bytes > 0}
											<span class="text-muted-foreground ml-2 text-xs"
												>{formatSize(model.size_bytes)}</span
											>
										{/if}
										{#if model.description}
											<p class="text-muted-foreground text-xs">{model.description}</p>
										{/if}
									</div>
									<button
										onclick={() => handleDownload(model.id)}
										disabled={downloading !== null}
										class="bg-primary text-primary-foreground hover:bg-primary/90 rounded px-3 py-1 text-xs font-medium disabled:opacity-50"
									>
										{downloading === model.id ? 'Laddar ner…' : 'Ladda ner'}
									</button>
								</div>

								<!-- Progress bar -->
								{#if downloading === model.id}
									{@const pct = downloadProgress[model.id] ?? 0}
									<div class="mt-2 space-y-1">
										<div class="bg-muted h-1.5 w-full overflow-hidden rounded-full">
											<div
												class="bg-primary h-full rounded-full transition-all duration-300"
												style="width: {pct}%"
											></div>
										</div>
										<p class="text-muted-foreground text-xs">{pct.toFixed(0)}%</p>
									</div>
								{/if}
							</div>
						{/each}
					</div>
				</div>
			{/if}

			{#if localModels.length === 0 && notDownloaded.length === 0 && registryDownloaded.length === 0}
				<p class="text-muted-foreground text-sm">Inga modeller hittades.</p>
			{/if}
		{/if}

		{#if systemInfo.total_ram_gb > 0}
			<p class="text-muted-foreground text-xs">
				RAM: {systemInfo.available_ram_gb.toFixed(1)} GB tillgängligt / {systemInfo.total_ram_gb.toFixed(1)} GB totalt
			</p>
		{/if}
	</div>
</div>
