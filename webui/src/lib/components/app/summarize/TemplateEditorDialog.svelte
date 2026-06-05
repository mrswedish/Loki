<script lang="ts">
	import * as Dialog from '$lib/components/ui/dialog';
	import { Button } from '$lib/components/ui/button';
	import { Input } from '$lib/components/ui/input';
	import { Label } from '$lib/components/ui/label';
	import { Textarea } from '$lib/components/ui/textarea';
	import { summaryTemplatesStore } from '$lib/stores/summary-templates.svelte';
	import { processFilesToChatUploaded } from '$lib/utils/process-uploaded-files';
	import { toast } from 'svelte-sonner';
	import { FileUp, Trash2, ChevronDown } from '@lucide/svelte';
	import * as Collapsible from '$lib/components/ui/collapsible';

	interface Props {
		open?: boolean;
		/** Id på mall som redigeras, eller null för att skapa ny. */
		editId?: string | null;
		/** Anropas med id på sparad/skapad mall. */
		onSaved?: (id: string) => void;
	}

	let { open = $bindable(false), editId = null, onSaved }: Props = $props();

	let label = $state('');
	let description = $state('');
	let systemPrompt = $state('');
	// Sampling-överstyrningar. Tomt fält = ärv modellens default.
	let temperature = $state('');
	let presencePenalty = $state('');
	let repeatPenalty = $state('');
	let advancedOpen = $state(false);
	let fileInput: HTMLInputElement;

	let isEditing = $derived(!!editId);

	// Fyll i fälten när dialogen öppnas (redigering) eller töm (ny).
	$effect(() => {
		if (open) {
			const existing = editId ? summaryTemplatesStore.get(editId) : undefined;
			label = existing?.label ?? '';
			description = existing?.description ?? '';
			systemPrompt = existing?.systemPrompt ?? '';
			temperature = existing?.sampling?.temperature?.toString() ?? '';
			presencePenalty = existing?.sampling?.presence_penalty?.toString() ?? '';
			repeatPenalty = existing?.sampling?.repeat_penalty?.toString() ?? '';
			advancedOpen = !!existing?.sampling;
		}
	});

	async function onFilePicked(e: Event) {
		const input = e.target as HTMLInputElement;
		const file = input.files?.[0];
		input.value = '';
		if (!file) return;
		try {
			const [uploaded] = await processFilesToChatUploaded([file]);
			const text = uploaded?.textContent?.trim() ?? '';
			if (!text) {
				toast.error('Filen verkar tom eller kunde inte läsas.');
				return;
			}
			systemPrompt = text;
			if (!label) label = file.name.replace(/\.[^.]+$/, '').trim();
			toast.success('Mall ifylld från fil');
		} catch {
			toast.error('Kunde inte läsa filen.');
		}
	}

	function buildSampling(): import('$lib/constants/model-sampling').TemplateSampling | undefined {
		const num = (s: string) => {
			const n = parseFloat(s.replace(',', '.'));
			return Number.isFinite(n) ? n : undefined;
		};
		const s = {
			temperature: num(temperature),
			presence_penalty: num(presencePenalty),
			repeat_penalty: num(repeatPenalty)
		};
		const hasAny =
			s.temperature !== undefined ||
			s.presence_penalty !== undefined ||
			s.repeat_penalty !== undefined;
		return hasAny ? s : undefined;
	}

	function resetSampling() {
		temperature = '';
		presencePenalty = '';
		repeatPenalty = '';
	}

	function save() {
		if (!systemPrompt.trim()) {
			toast.error('Skriv en instruktion för mallen.');
			return;
		}
		const data = { label, description, systemPrompt, sampling: buildSampling() };
		const id = editId
			? (summaryTemplatesStore.update(editId, data), editId)
			: summaryTemplatesStore.create(data);
		open = false;
		onSaved?.(id);
	}

	function remove() {
		if (!editId) return;
		summaryTemplatesStore.remove(editId);
		open = false;
		toast.success('Mall raderad');
	}
</script>

<Dialog.Root bind:open>
	<Dialog.Portal>
		<Dialog.Overlay />
		<Dialog.Content class="max-w-lg">
			<Dialog.Header>
				<Dialog.Title>{isEditing ? 'Redigera mall' : 'Ny egen mall'}</Dialog.Title>
				<Dialog.Description>
					Beskriv vad mallen ska skapa. Instruktionen skickas till modellen tillsammans med
					transkriberingen.
				</Dialog.Description>
			</Dialog.Header>

			<div class="space-y-4 py-2">
				<div class="space-y-1.5">
					<Label for="tpl-label">Namn</Label>
					<Input id="tpl-label" bind:value={label} placeholder="t.ex. Kort beslutslista" />
				</div>
				<div class="space-y-1.5">
					<Label for="tpl-desc">Beskrivning</Label>
					<Input
						id="tpl-desc"
						bind:value={description}
						placeholder="Kort text som visas under mallnamnet"
					/>
				</div>
				<div class="space-y-1.5">
					<div class="flex items-center justify-between">
						<Label for="tpl-prompt">Instruktion till modellen</Label>
						<Button
							variant="ghost"
							size="sm"
							class="h-7 gap-1.5 text-xs"
							onclick={() => fileInput.click()}
						>
							<FileUp class="size-3.5" />
							Från fil
						</Button>
					</div>
					<Textarea
						id="tpl-prompt"
						bind:value={systemPrompt}
						rows={7}
						placeholder="Beskriv vad modellen ska göra med transkriberingen, t.ex. 'Skapa en punktlista med alla beslut och vem som är ansvarig.'"
					/>
					<p class="text-xs text-muted-foreground">
						Trohet mot källan (inga omskrivningar) läggs till automatiskt.
					</p>
				</div>
				<Collapsible.Root bind:open={advancedOpen}>
					<Collapsible.Trigger
						class="flex items-center gap-1.5 text-sm text-muted-foreground hover:text-foreground"
					>
						<ChevronDown
							class={'size-4 transition-transform ' + (advancedOpen ? 'rotate-180' : '')}
						/>
						Avancerat (sampling)
					</Collapsible.Trigger>
					<Collapsible.Content class="space-y-4 pt-3">
						<div class="space-y-1.5">
							<Label for="tpl-temp">Temperatur</Label>
							<Input id="tpl-temp" bind:value={temperature} placeholder="modellens standard" />
							<p class="text-xs text-muted-foreground">
								Lägre = mer förutsägbart och troget källan, högre = mer kreativt. Lämna tomt för
								modellens standard. Typiskt 0.3–0.8 för protokoll.
							</p>
						</div>
						<div class="space-y-1.5">
							<Label for="tpl-presence">Upprepningsstraff</Label>
							<Input
								id="tpl-presence"
								bind:value={presencePenalty}
								placeholder="modellens standard"
							/>
							<p class="text-xs text-muted-foreground">
								Högre värde (0–2) motverkar att modellen upprepar sig.
							</p>
						</div>
						<div class="space-y-1.5">
							<Label for="tpl-repeat">Repetitionsstraff</Label>
							<Input id="tpl-repeat" bind:value={repeatPenalty} placeholder="modellens standard" />
							<p class="text-xs text-muted-foreground">
								Alternativt straff mot upprepning (typiskt 1.0–1.3).
							</p>
						</div>
						<Button variant="ghost" size="sm" class="text-xs" onclick={resetSampling}>
							Återställ till modellens standard
						</Button>
					</Collapsible.Content>
				</Collapsible.Root>
			</div>

			<Dialog.Footer class="flex items-center justify-between gap-2 sm:justify-between">
				{#if isEditing}
					<Button variant="ghost" class="gap-1.5 text-destructive" onclick={remove}>
						<Trash2 class="size-4" />
						Radera
					</Button>
				{:else}
					<span></span>
				{/if}
				<div class="flex gap-2">
					<Button variant="outline" onclick={() => (open = false)}>Avbryt</Button>
					<Button onclick={save}>Spara</Button>
				</div>
			</Dialog.Footer>
		</Dialog.Content>
	</Dialog.Portal>
</Dialog.Root>

<input
	bind:this={fileInput}
	type="file"
	accept=".txt,.md,.pdf,text/plain,text/markdown,application/pdf"
	class="hidden"
	onchange={onFilePicked}
/>
