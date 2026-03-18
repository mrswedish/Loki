<script lang="ts">
	import {
		Settings,
		Funnel,
		AlertTriangle,
		Monitor,
		ChevronLeft,
		ChevronRight,
		Database,
		Cpu
	} from '@lucide/svelte';
	import {
		ChatSettingsFooter,
		ChatSettingsImportExportTab,
		ChatSettingsFields,
		ChatSettingsSystemTab
	} from '$lib/components/app';
	import { ScrollArea } from '$lib/components/ui/scroll-area';
	import { config, settingsStore } from '$lib/stores/settings.svelte';
	import { serverStore } from '$lib/stores/server.svelte';
	import { stopServer, startServer } from '$lib/tauri-bridge';
	import { toast } from 'svelte-sonner';
	import {
		SETTINGS_SECTION_TITLES,
		type SettingsSectionTitle,
		NUMERIC_FIELDS,
		POSITIVE_INTEGER_FIELDS,
		SETTINGS_COLOR_MODES_CONFIG,
		SETTINGS_KEYS
	} from '$lib/constants';
	import { setMode } from 'mode-watcher';
	import { ColorMode } from '$lib/enums/ui';
	import { SettingsFieldType } from '$lib/enums/settings';
	import type { Component } from 'svelte';

	interface Props {
		onSave?: () => void;
		initialSection?: SettingsSectionTitle;
	}

	let { onSave, initialSection }: Props = $props();

	const settingSections: Array<{
		fields: SettingsFieldConfig[];
		icon: Component;
		title: SettingsSectionTitle;
	}> = [
		{
			title: SETTINGS_SECTION_TITLES.GENERAL,
			icon: Settings,
			fields: [
				{
					key: SETTINGS_KEYS.THEME,
					label: 'Tema',
					type: SettingsFieldType.SELECT,
					options: SETTINGS_COLOR_MODES_CONFIG
				},
				{ key: SETTINGS_KEYS.API_KEY, label: 'API-nyckel', type: SettingsFieldType.INPUT },
				{
					key: SETTINGS_KEYS.SYSTEM_MESSAGE,
					label: 'Systemprompt',
					type: SettingsFieldType.TEXTAREA
				},
				{
					key: SETTINGS_KEYS.PASTE_LONG_TEXT_TO_FILE_LEN,
					label: 'Klistra in lång text till fil (längd)',
					type: SettingsFieldType.INPUT
				},
				{
					key: SETTINGS_KEYS.CONTEXT_SIZE,
					label: 'Kontextfönster (tokens)',
					type: SettingsFieldType.SLIDER,
					min: 1024,
					max: 65536,
					step: 1024
				},
				{
					key: SETTINGS_KEYS.COPY_TEXT_ATTACHMENTS_AS_PLAIN_TEXT,
					label: 'Kopiera textbilagor som ren text',
					type: SettingsFieldType.CHECKBOX
				},
				{
					key: SETTINGS_KEYS.ENABLE_CONTINUE_GENERATION,
					label: 'Aktivera "Fortsätt"-knapp',
					type: SettingsFieldType.CHECKBOX,
					isExperimental: true
				},
				{
					key: SETTINGS_KEYS.PDF_AS_IMAGE,
					label: 'Tolka PDF som bild',
					type: SettingsFieldType.CHECKBOX
				},
				{
					key: SETTINGS_KEYS.ASK_FOR_TITLE_CONFIRMATION,
					label: 'Fråga innan konversationstitel ändras',
					type: SettingsFieldType.CHECKBOX
				}
			]
		},
		{
			title: SETTINGS_SECTION_TITLES.DISPLAY,
			icon: Monitor,
			fields: [
				{
					key: SETTINGS_KEYS.SHOW_MESSAGE_STATS,
					label: 'Visa genereringsstatistik',
					type: SettingsFieldType.CHECKBOX
				},
				{
					key: SETTINGS_KEYS.SHOW_THOUGHT_IN_PROGRESS,
					label: 'Visa pågående tankeprocess',
					type: SettingsFieldType.CHECKBOX
				},
				{
					key: SETTINGS_KEYS.KEEP_STATS_VISIBLE,
					label: 'Behåll statistik synlig efter generering',
					type: SettingsFieldType.CHECKBOX
				},
				{
					key: SETTINGS_KEYS.DISABLE_REASONING_PARSING,
					label: 'Inaktivera resonemangs-parsing',
					type: SettingsFieldType.CHECKBOX
				},
				{
					key: SETTINGS_KEYS.AUTO_MIC_ON_EMPTY,
					label: 'Visa mikrofon vid tomt inmatningsfält',
					type: SettingsFieldType.CHECKBOX,
					isExperimental: true
				},
				{
					key: SETTINGS_KEYS.RENDER_USER_CONTENT_AS_MARKDOWN,
					label: 'Rendera användartext som Markdown',
					type: SettingsFieldType.CHECKBOX
				},
				{
					key: SETTINGS_KEYS.FULL_HEIGHT_CODE_BLOCKS,
					label: 'Använd kodblockar med full höjd',
					type: SettingsFieldType.CHECKBOX
				},
				{
					key: SETTINGS_KEYS.DISABLE_AUTO_SCROLL,
					label: 'Stäng av automatisk scrollning',
					type: SettingsFieldType.CHECKBOX
				},
				{
					key: SETTINGS_KEYS.ALWAYS_SHOW_SIDEBAR_ON_DESKTOP,
					label: 'Visa alltid sidofält på desktop',
					type: SettingsFieldType.CHECKBOX
				},
				{
					key: SETTINGS_KEYS.AUTO_SHOW_SIDEBAR_ON_NEW_CHAT,
					label: 'Öppna sidofält automatiskt vid ny chatt',
					type: SettingsFieldType.CHECKBOX
				},
				{
					key: SETTINGS_KEYS.SHOW_RAW_MODEL_NAMES,
					label: 'Visa råa modellnamn',
					type: SettingsFieldType.CHECKBOX
				}
			]
		},
		{
			title: SETTINGS_SECTION_TITLES.SAMPLING,
			icon: Funnel,
			fields: [
				{
					key: SETTINGS_KEYS.TEMPERATURE,
					label: 'Temperatur',
					type: SettingsFieldType.SLIDER,
					min: 0,
					max: 2,
					step: 0.1
				},
				{
					key: SETTINGS_KEYS.DYNATEMP_RANGE,
					label: 'Dynamiskt temperaturintervall',
					type: SettingsFieldType.INPUT
				},
				{
					key: SETTINGS_KEYS.DYNATEMP_EXPONENT,
					label: 'Dynamisk temperaturexponent',
					type: SettingsFieldType.INPUT
				},
				{
					key: SETTINGS_KEYS.TOP_K,
					label: 'Top K',
					type: SettingsFieldType.INPUT
				},
				{
					key: SETTINGS_KEYS.TOP_P,
					label: 'Top P',
					type: SettingsFieldType.INPUT
				},
				{
					key: SETTINGS_KEYS.MIN_P,
					label: 'Min P',
					type: SettingsFieldType.INPUT
				},
				{
					key: SETTINGS_KEYS.XTC_PROBABILITY,
					label: 'XTC probability',
					type: SettingsFieldType.INPUT
				},
				{
					key: SETTINGS_KEYS.XTC_THRESHOLD,
					label: 'XTC threshold',
					type: SettingsFieldType.INPUT
				},
				{
					key: SETTINGS_KEYS.TYP_P,
					label: 'Typical P',
					type: SettingsFieldType.INPUT
				},
				{
					key: SETTINGS_KEYS.MAX_TOKENS,
					label: 'Max tokens',
					type: SettingsFieldType.INPUT
				},
				{
					key: SETTINGS_KEYS.SAMPLERS,
					label: 'Samplers',
					type: SettingsFieldType.INPUT
				},
				{
					key: SETTINGS_KEYS.BACKEND_SAMPLING,
					label: 'Backend-sampling',
					type: SettingsFieldType.CHECKBOX
				}
			]
		},
		{
			title: SETTINGS_SECTION_TITLES.PENALTIES,
			icon: AlertTriangle,
			fields: [
				{
					key: SETTINGS_KEYS.REPEAT_LAST_N,
					label: 'Upprepa senaste N',
					type: SettingsFieldType.INPUT
				},
				{
					key: SETTINGS_KEYS.REPEAT_PENALTY,
					label: 'Upprepningsstraff',
					type: SettingsFieldType.INPUT
				},
				{
					key: SETTINGS_KEYS.PRESENCE_PENALTY,
					label: 'Närvarodstraff',
					type: SettingsFieldType.INPUT
				},
				{
					key: SETTINGS_KEYS.FREQUENCY_PENALTY,
					label: 'Frekvensstraff',
					type: SettingsFieldType.INPUT
				},
				{
					key: SETTINGS_KEYS.DRY_MULTIPLIER,
					label: 'DRY multiplier',
					type: SettingsFieldType.INPUT
				},
				{
					key: SETTINGS_KEYS.DRY_BASE,
					label: 'DRY base',
					type: SettingsFieldType.INPUT
				},
				{
					key: SETTINGS_KEYS.DRY_ALLOWED_LENGTH,
					label: 'DRY allowed length',
					type: SettingsFieldType.INPUT
				},
				{
					key: SETTINGS_KEYS.DRY_PENALTY_LAST_N,
					label: 'DRY penalty last N',
					type: SettingsFieldType.INPUT
				}
			]
		},
		{
			title: SETTINGS_SECTION_TITLES.IMPORT_EXPORT,
			icon: Database,
			fields: []
		},
		{
			title: SETTINGS_SECTION_TITLES.SYSTEM,
			icon: Cpu,
			fields: []
		}
	];

	let activeSection = $derived<SettingsSectionTitle>(
		initialSection ?? SETTINGS_SECTION_TITLES.GENERAL
	);
	let currentSection = $derived(
		settingSections.find((section) => section.title === activeSection) || settingSections[0]
	);
	let needsRestart = $derived(
		localConfig.contextSize !== (serverStore.contextSize ?? 4096) ||
		localConfig.gpuIndex !== (serverStore.activeGpuIndex ?? -1)
	);

	let localConfig: SettingsConfigType = $state({ ...config() });

	let canScrollLeft = $state(false);
	let canScrollRight = $state(false);
	let scrollContainer: HTMLDivElement | undefined = $state();

	$effect(() => {
		if (initialSection) {
			activeSection = initialSection;
		}
	});

	function applyTheme(theme: string) {
		if (theme === ColorMode.RETRO) {
			setMode(ColorMode.DARK);
			document.documentElement.classList.add('retro');
		} else {
			document.documentElement.classList.remove('retro');
			setMode(theme as ColorMode);
		}
	}

	function handleThemeChange(newTheme: string) {
		localConfig.theme = newTheme;
		applyTheme(newTheme);
	}

	function handleConfigChange(key: string, value: string | boolean) {
		localConfig[key] = value;
	}

	function handleReset() {
		localConfig = { ...config() };
		applyTheme(localConfig.theme as string);
	}

	async function handleSaveAndRestart() {
		const modelPath = serverStore.props?.model_path;
		if (!modelPath) {
			handleSave();
			return;
		}

		handleSave();

		const ctx = localConfig.contextSize as number;
		const gpu = (localConfig.gpuIndex ?? -1) as number;

		try {
			toast.info('Startar om servern med nya inställningar...');
			await stopServer();
			await startServer(modelPath, ctx, gpu);
			serverStore.activeGpuIndex = gpu;
			await serverStore.fetch();
			toast.success('Servern har startats om');
			onSave?.();
		} catch (e) {
			toast.error('Kunde inte starta om: ' + (e instanceof Error ? e.message : String(e)));
		}
	}

	function handleSave() {
		if (localConfig.custom && typeof localConfig.custom === 'string' && localConfig.custom.trim()) {
			try {
				JSON.parse(localConfig.custom);
			} catch (error) {
				alert('Invalid JSON in custom parameters. Please check the format and try again.');
				console.error(error);
				return;
			}
		}

		// Convert numeric strings to numbers for numeric fields
		const processedConfig = { ...localConfig };

		for (const field of NUMERIC_FIELDS) {
			if (processedConfig[field] !== undefined && processedConfig[field] !== '') {
				const numValue = Number(processedConfig[field]);
				if (!isNaN(numValue)) {
					if ((POSITIVE_INTEGER_FIELDS as readonly string[]).includes(field)) {
						processedConfig[field] = Math.max(1, Math.round(numValue));
					} else {
						processedConfig[field] = numValue;
					}
				} else {
					alert(`Invalid numeric value for ${field}. Please enter a valid number.`);
					return;
				}
			}
		}

		settingsStore.updateMultipleConfig(processedConfig);
		onSave?.();
	}

	function scrollToCenter(element: HTMLElement) {
		if (!scrollContainer) return;

		const containerRect = scrollContainer.getBoundingClientRect();
		const elementRect = element.getBoundingClientRect();

		const elementCenter = elementRect.left + elementRect.width / 2;
		const containerCenter = containerRect.left + containerRect.width / 2;
		const scrollOffset = elementCenter - containerCenter;

		scrollContainer.scrollBy({ left: scrollOffset, behavior: 'smooth' });
	}

	function scrollLeft() {
		if (!scrollContainer) return;

		scrollContainer.scrollBy({ left: -250, behavior: 'smooth' });
	}

	function scrollRight() {
		if (!scrollContainer) return;

		scrollContainer.scrollBy({ left: 250, behavior: 'smooth' });
	}

	function updateScrollButtons() {
		if (!scrollContainer) return;

		const { scrollLeft, scrollWidth, clientWidth } = scrollContainer;
		canScrollLeft = scrollLeft > 0;
		canScrollRight = scrollLeft < scrollWidth - clientWidth - 1; // -1 for rounding
	}

	export function reset() {
		localConfig = { ...config() };

		setTimeout(updateScrollButtons, 100);
	}

	$effect(() => {
		if (scrollContainer) {
			updateScrollButtons();
		}
	});
</script>

<div class="flex h-full flex-col overflow-hidden md:flex-row">
	<!-- Desktop Sidebar -->
	<div class="hidden w-64 border-r border-border/30 p-6 md:block">
		<nav class="space-y-1 py-2">
			{#each settingSections as section (section.title)}
				<button
					class="flex w-full cursor-pointer items-center gap-3 rounded-lg px-3 py-2 text-left text-sm transition-colors hover:bg-accent {activeSection ===
					section.title
						? 'bg-accent text-accent-foreground'
						: 'text-muted-foreground'}"
					onclick={() => (activeSection = section.title)}
				>
					<section.icon class="h-4 w-4" />

					<span class="ml-2">{section.title}</span>
				</button>
			{/each}
		</nav>
	</div>

	<!-- Mobile Header with Horizontal Scrollable Menu -->
	<div class="flex flex-col pt-6 md:hidden">
		<div class="border-b border-border/30 pt-4 md:py-4">
			<!-- Horizontal Scrollable Category Menu with Navigation -->
			<div class="relative flex items-center" style="scroll-padding: 1rem;">
				<button
					class="absolute left-2 z-10 flex h-6 w-6 items-center justify-center rounded-full bg-muted shadow-md backdrop-blur-sm transition-opacity hover:bg-accent {canScrollLeft
						? 'opacity-100'
						: 'pointer-events-none opacity-0'}"
					onclick={scrollLeft}
					aria-label="Scroll left"
				>
					<ChevronLeft class="h-4 w-4" />
				</button>

				<div
					class="scrollbar-hide overflow-x-auto py-2"
					bind:this={scrollContainer}
					onscroll={updateScrollButtons}
				>
					<div class="flex min-w-max gap-2">
						{#each settingSections as section (section.title)}
							<button
								class="flex cursor-pointer items-center gap-2 rounded-lg px-3 py-2 text-sm whitespace-nowrap transition-colors first:ml-4 last:mr-4 hover:bg-accent {activeSection ===
								section.title
									? 'bg-accent text-accent-foreground'
									: 'text-muted-foreground'}"
								onclick={(e: MouseEvent) => {
									activeSection = section.title;
									scrollToCenter(e.currentTarget as HTMLElement);
								}}
							>
								<section.icon class="h-4 w-4 flex-shrink-0" />
								<span>{section.title}</span>
							</button>
						{/each}
					</div>
				</div>

				<button
					class="absolute right-2 z-10 flex h-6 w-6 items-center justify-center rounded-full bg-muted shadow-md backdrop-blur-sm transition-opacity hover:bg-accent {canScrollRight
						? 'opacity-100'
						: 'pointer-events-none opacity-0'}"
					onclick={scrollRight}
					aria-label="Scroll right"
				>
					<ChevronRight class="h-4 w-4" />
				</button>
			</div>
		</div>
	</div>

	<ScrollArea class="max-h-[calc(100dvh-13.5rem)] flex-1 md:max-h-[calc(100vh-13.5rem)]">
		<div class="space-y-6 p-4 md:p-6">
			<div class="grid">
				<div class="mb-6 flex hidden items-center gap-2 border-b border-border/30 pb-6 md:flex">
					<currentSection.icon class="h-5 w-5" />

					<h3 class="text-lg font-semibold">{currentSection.title}</h3>
				</div>

				{#if currentSection.title === SETTINGS_SECTION_TITLES.SYSTEM}
					<ChatSettingsSystemTab />
				{:else if currentSection.title === SETTINGS_SECTION_TITLES.IMPORT_EXPORT}
					<ChatSettingsImportExportTab />
				{:else}
					<div class="space-y-6">
						<ChatSettingsFields
							fields={currentSection.fields}
							{localConfig}
							onConfigChange={handleConfigChange}
							onThemeChange={handleThemeChange}
						/>
					</div>
				{/if}
			</div>

			<div class="mt-8 border-t pt-6">
				<p class="text-xs text-muted-foreground">Inställningar sparas i webbläsarens localStorage</p>
			</div>
		</div>
	</ScrollArea>
</div>

<ChatSettingsFooter
	onReset={handleReset}
	onSave={handleSave}
	onSaveAndRestart={handleSaveAndRestart}
	showRestartButton={needsRestart}
/>
