<script lang="ts">
	import { summarizeStore } from '$lib/stores/summarize.svelte';
	import { summaryTemplatesStore } from '$lib/stores/summary-templates.svelte';
	import { DEFAULT_TEMPLATE_ID } from '$lib/constants/summary-templates';
	import TemplatePicker from './TemplatePicker.svelte';
	import TemplateEditorDialog from './TemplateEditorDialog.svelte';
	import { Button } from '$lib/components/ui/button';
	import { Input } from '$lib/components/ui/input';
	import { Label } from '$lib/components/ui/label';
	import { Switch } from '$lib/components/ui/switch';
	import { cn } from '$lib/components/ui/utils';
	import { MarkdownContent } from '$lib/components/app/content';
	import { copyToClipboard, copyRichToClipboard, stripReasoningAndMarkdown } from '$lib/utils';
	import { saveTextFile } from '$lib/tauri-bridge';
	import { toast } from 'svelte-sonner';
	import {
		FileText,
		FileUp,
		Paperclip,
		X,
		Sparkles,
		Settings2,
		Loader2,
		Copy,
		Download,
		Languages,
		Plus,
		FileType
	} from '@lucide/svelte';

	let isDragOver = $state(false);
	let dragCounter = 0;
	let selectedTemplate = $state(DEFAULT_TEMPLATE_ID);
	let editorOpen = $state(false);
	let editorEditId = $state<string | null>(null);

	function openCreateTemplate() {
		editorEditId = null;
		editorOpen = true;
	}
	function openEditTemplate(id: string) {
		editorEditId = id;
		editorOpen = true;
	}
	// Duplicera valfri mall (även inbyggd) till en redigerbar kopia och öppna den.
	function duplicateTemplate(id: string) {
		const newId = summaryTemplatesStore.duplicate(id);
		if (newId) {
			selectedTemplate = newId;
			editorEditId = newId;
			editorOpen = true;
		}
	}
	function onTemplateSaved(id: string) {
		selectedTemplate = id;
	}

	let transcriptInput: HTMLInputElement;
	let agendaInput: HTMLInputElement;

	let info = $derived(summarizeStore.info);
	let busy = $derived(summarizeStore.busy);
	// Visa resultatvyn när vi kör (streaming) eller är klara.
	let showResult = $derived(
		summarizeStore.state === 'running' ||
			summarizeStore.state === 'done' ||
			summarizeStore.result.length > 0
	);

	// Vid chunkad sammanfattning: "del X av N" under map-fasen.
	let chunkLabel = $derived(
		summarizeStore.chunkProgress
			? `del ${summarizeStore.chunkProgress.current} av ${summarizeStore.chunkProgress.total}`
			: ''
	);

	// Förlopps-text under körning: chunk-läge, prompt-processing, tänkande, generering.
	let progressText = $derived.by(() => {
		if (chunkLabel) {
			const inner =
				summarizeStore.promptPercent !== null
					? ` · ${summarizeStore.promptPercent}%`
					: summarizeStore.generatedTokens > 0
						? ` · ${summarizeStore.generatedTokens} tokens`
						: '';
			return `Sammanfattar ${chunkLabel}${inner}`;
		}
		if (summarizeStore.promptPercent !== null) {
			return `Bearbetar ${summarizeStore.promptPercent}%`;
		}
		if (summarizeStore.isThinking) {
			return `Tänker… ${summarizeStore.thinkingTokens} tokens`;
		}
		if (summarizeStore.generatedTokens > 0) {
			const tps = summarizeStore.tokensPerSec;
			const rate = tps > 0 ? ` · ${tps.toFixed(1)} tokens/s` : '';
			return `${summarizeStore.generatedTokens} tokens${rate}`;
		}
		return '';
	});

	// Kopiera som ren, läsbar text (inga **/##) – funkar i mejl/Word/anteckningar.
	async function copyResult() {
		await copyToClipboard(
			stripReasoningAndMarkdown(summarizeStore.result),
			'Kopierat!',
			'Kunde inte kopiera'
		);
	}

	// Kopiera som formaterad text (HTML på urklipp). Word/Outlook/Docs tolkar
	// HTML:en → riktiga rubriker, fetstil och listor vid inklistring.
	async function copyResultForWord() {
		await copyRichToClipboard(
			summarizeStore.result,
			'Kopierat med formatering – klistra in i Word',
			'Kunde inte kopiera'
		);
	}

	// Exportera som ren text-fil.
	async function exportResult() {
		try {
			await saveTextFile(stripReasoningAndMarkdown(summarizeStore.result));
			toast.success('Sparat');
		} catch {
			toast.error('Kunde inte spara filen');
		}
	}

	function handleDragEnter(e: DragEvent) {
		e.preventDefault();
		dragCounter++;
		if (e.dataTransfer?.types.includes('Files')) isDragOver = true;
	}
	function handleDragLeave(e: DragEvent) {
		e.preventDefault();
		dragCounter--;
		if (dragCounter <= 0) isDragOver = false;
	}
	function handleDragOver(e: DragEvent) {
		e.preventDefault();
	}
	async function handleDrop(e: DragEvent) {
		e.preventDefault();
		isDragOver = false;
		dragCounter = 0;
		const file = e.dataTransfer?.files?.[0];
		if (file) await summarizeStore.loadTranscript(file);
	}

	async function onTranscriptPicked(e: Event) {
		const input = e.target as HTMLInputElement;
		const file = input.files?.[0];
		// Nollställ värdet så att samma fil kan väljas igen (annars triggas inte onchange).
		input.value = '';
		if (file) await summarizeStore.loadTranscript(file);
	}
	async function onAgendaPicked(e: Event) {
		const input = e.target as HTMLInputElement;
		const file = input.files?.[0];
		input.value = '';
		if (file) await summarizeStore.loadAgenda(file);
	}

	async function create() {
		await summarizeStore.run(selectedTemplate);
	}

	function formatTokens(n: number): string {
		return n.toLocaleString('sv-SE');
	}
</script>

<div class="mx-auto flex h-full min-h-0 w-full max-w-3xl flex-col gap-6 overflow-hidden px-6 py-10">
	{#if showResult}
		<!-- Steg 3: resultatpanel. Protokollet streamas in här; knappar för att
		förbättra svenskan (Gemma), kopiera, exportera och börja om. -->
		<div class="flex items-center justify-between gap-3">
			<div class="min-w-0">
				<h2 class="truncate text-base font-medium">
					{summarizeStore.meetingName || 'Sammanfattning'}
				</h2>
				<p class="text-xs text-muted-foreground">
					{#if summarizeStore.phase === 'summarizing'}
						<Loader2 class="mr-1 inline size-3 animate-spin" />Sammanfattar med {summarizeStore.resultModel}{progressText
							? ` · ${progressText}`
							: '…'}
					{:else if summarizeStore.phase === 'refining'}
						<Loader2 class="mr-1 inline size-3 animate-spin" />Förbättrar svenskan med Gemma{progressText
							? ` · ${progressText}`
							: '…'}
					{:else}
						Skapad med {summarizeStore.resultModel}{summarizeStore.usedTemperature !== null
							? ` · temp ${summarizeStore.usedTemperature}`
							: ''}{summarizeStore.refined ? ' · språkrättad' : ''}
					{/if}
				</p>
			</div>
			<Button
				variant="ghost"
				size="sm"
				class="gap-1.5"
				onclick={() => summarizeStore.reset()}
				disabled={busy}
			>
				<Plus class="size-4" />
				Ny
			</Button>
		</div>

		<div class="min-h-0 flex-1 overflow-y-auto rounded-xl border border-border bg-card p-5">
			{#if summarizeStore.result}
				<div class="prose prose-sm dark:prose-invert max-w-none break-words">
					<MarkdownContent content={summarizeStore.result} />
				</div>
			{:else}
				<div class="space-y-3">
					<div class="flex items-center gap-2 text-sm text-muted-foreground">
						<Loader2 class="size-4 animate-spin" />
						{#if chunkLabel}
							Sammanfattar {chunkLabel}…
						{:else if summarizeStore.state === 'running' && summarizeStore.chunkProgress === null && info && info.strategy === 'chunked'}
							Slår ihop delarna…
						{:else if summarizeStore.promptPercent !== null}
							Bearbetar transkriberingen… {summarizeStore.promptPercent}%
						{:else if summarizeStore.isThinking}
							Modellen tänker… {summarizeStore.thinkingTokens} tokens
						{:else}
							Förbereder…
						{/if}
					</div>
					{#if summarizeStore.promptPercent !== null}
						<div class="h-1.5 w-full overflow-hidden rounded-full bg-muted">
							<div
								class="h-full rounded-full bg-primary transition-all duration-300"
								style="width: {summarizeStore.promptPercent}%"
							></div>
						</div>
					{/if}
				</div>
			{/if}
		</div>

		{#if summarizeStore.error}
			<p class="text-sm text-destructive">{summarizeStore.error}</p>
		{/if}

		{#if summarizeStore.state === 'done'}
			<div class="flex flex-wrap items-center gap-2">
				{#if !summarizeStore.refined}
					<Button onclick={() => summarizeStore.refineLanguage()} disabled={busy} class="gap-2">
						<Languages class="size-4" />
						Förbättra svenskan (Gemma)
					</Button>
				{/if}
				<Button variant="outline" onclick={copyResult} disabled={busy} class="gap-2">
					<Copy class="size-4" />
					Kopiera
				</Button>
				<Button variant="outline" onclick={copyResultForWord} disabled={busy} class="gap-2">
					<FileType class="size-4" />
					Kopiera till Word
				</Button>
				<Button variant="outline" onclick={exportResult} disabled={busy} class="gap-2">
					<Download class="size-4" />
					Exportera
				</Button>
			</div>
		{/if}
	{:else if !info}
		<!-- Steg 1: drop-zon-först. Lugn, tom startskärm. Drag-handlers på en
		container-div (samma mönster som ChatScreen), klick hanteras av knappen inuti. -->
		<div
			class="flex flex-1 flex-col items-center justify-center"
			role="region"
			aria-label="Släppyta för transkribering"
			ondragenter={handleDragEnter}
			ondragleave={handleDragLeave}
			ondragover={handleDragOver}
			ondrop={handleDrop}
		>
			<button
				type="button"
				onclick={() => transcriptInput.click()}
				class={cn(
					'flex w-full flex-col items-center justify-center gap-4 rounded-2xl border-2 border-dashed px-8 py-20 transition-all',
					isDragOver
						? 'scale-[1.01] border-primary bg-primary/5'
						: 'border-border hover:border-primary/50 hover:bg-accent/30'
				)}
			>
				<div
					class={cn(
						'flex size-16 items-center justify-center rounded-full transition-colors',
						isDragOver ? 'bg-primary/10' : 'bg-muted'
					)}
				>
					{#if summarizeStore.state === 'reading'}
						<Loader2 class="size-7 animate-spin text-primary" />
					{:else}
						<FileUp class={cn('size-7', isDragOver ? 'text-primary' : 'text-muted-foreground')} />
					{/if}
				</div>
				<div class="text-center">
					<p class="text-lg font-medium">
						{summarizeStore.state === 'reading' ? 'Läser…' : 'Släpp transkribering här'}
					</p>
					<p class="mt-1 text-sm text-muted-foreground">
						eller klicka för att välja fil &middot; .txt, .md, .pdf
					</p>
				</div>
			</button>

			{#if summarizeStore.error}
				<p class="mt-4 text-sm text-destructive">{summarizeStore.error}</p>
			{/if}
		</div>
	{:else}
		<!-- Steg 2: transkribering inläst → visa info, mallar, agenda, Skapa. -->
		<div class="flex items-center gap-3 rounded-xl border border-border bg-card p-4">
			<FileText class="size-5 shrink-0 text-primary" />
			<div class="min-w-0 flex-1">
				<p class="truncate text-sm font-medium">{info.fileName}</p>
				<p class="text-xs text-muted-foreground">
					~{formatTokens(info.tokens)} tokens &middot; ca {info.approxMinutes} min tal
					{#if info.strategy === 'single'}
						&middot; <span class="text-primary">ryms i ett pass</span>
					{:else}
						&middot; <span class="text-amber-600 dark:text-amber-500">mycket lång – delas upp</span>
					{/if}
				</p>
			</div>
			<Button variant="ghost" size="icon" onclick={() => summarizeStore.reset()} disabled={busy}>
				<X class="size-4" />
			</Button>
		</div>

		<!-- Mötesnamn → konversationstitel i historiken -->
		<div class="space-y-1.5">
			<Label for="meeting-name" class="text-sm font-medium">Namn på mötet</Label>
			<Input
				id="meeting-name"
				bind:value={summarizeStore.meetingName}
				placeholder="t.ex. Styrelsemöte 4 juni"
				disabled={busy}
			/>
		</div>

		<div class="space-y-3">
			<h2 class="text-sm font-medium">Vad vill du skapa?</h2>
			<TemplatePicker
				selectedId={selectedTemplate}
				onSelect={(id) => (selectedTemplate = id)}
				onCreate={openCreateTemplate}
				onEdit={openEditTemplate}
				onDuplicate={duplicateTemplate}
			/>
		</div>

		<!-- Valfri agenda -->
		<div class="space-y-2">
			{#if summarizeStore.agenda}
				<div
					class="flex items-center gap-2 rounded-lg border border-border bg-muted/40 px-3 py-2 text-sm"
				>
					<Paperclip class="size-4 text-muted-foreground" />
					<span class="flex-1">Agenda bifogad – protokollet följer dess struktur</span>
					<Button
						variant="ghost"
						size="icon"
						class="size-6"
						onclick={() => summarizeStore.clearAgenda()}
						disabled={busy}
					>
						<X class="size-3.5" />
					</Button>
				</div>
			{:else}
				<button
					type="button"
					onclick={() => agendaInput.click()}
					disabled={busy}
					class="inline-flex items-center gap-1.5 text-sm text-muted-foreground transition-colors hover:text-foreground disabled:opacity-50"
				>
					<Paperclip class="size-3.5" />
					Lägg till agenda (valfritt)
				</button>
			{/if}
		</div>

		<div class="flex items-center justify-between gap-3 pt-2">
			<Button onclick={create} disabled={busy} size="lg" class="gap-2">
				{#if busy}
					<Loader2 class="size-4 animate-spin" />
					{summarizeStore.state === 'preparing' ? 'Förbereder…' : 'Skapar…'}
				{:else}
					<Sparkles class="size-4" />
					Skapa
				{/if}
			</Button>

			<!-- Noggrannare = låt modellen resonera (thinking). Av som standard. -->
			<div class="flex items-center gap-2">
				<Switch id="thorough" bind:checked={summarizeStore.thorough} disabled={busy} />
				<Label for="thorough" class="cursor-pointer text-sm text-muted-foreground">
					Noggrannare
				</Label>
			</div>
		</div>
	{/if}

	<!-- Diskret länk till avancerat läge (full chatt) -->
	<div class="flex justify-center pt-2">
		<a
			href="/chat"
			class="inline-flex items-center gap-1.5 text-xs text-muted-foreground transition-colors hover:text-foreground"
		>
			<Settings2 class="size-3.5" />
			Avancerat läge
		</a>
	</div>

	<input
		bind:this={transcriptInput}
		type="file"
		accept=".txt,.md,.pdf,text/plain,text/markdown,application/pdf"
		class="hidden"
		onchange={onTranscriptPicked}
	/>
	<input
		bind:this={agendaInput}
		type="file"
		accept=".txt,.md,.pdf,text/plain,text/markdown,application/pdf"
		class="hidden"
		onchange={onAgendaPicked}
	/>
</div>

<TemplateEditorDialog bind:open={editorOpen} editId={editorEditId} onSaved={onTemplateSaved} />
