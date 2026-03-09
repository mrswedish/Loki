<script lang="ts">
	import { RotateCcw, FlaskConical, AlertCircle, Info } from '@lucide/svelte';
	import { Checkbox } from '$lib/components/ui/checkbox';
	import { Input } from '$lib/components/ui/input';
	import Label from '$lib/components/ui/label/label.svelte';
	import * as Select from '$lib/components/ui/select';
	import { Textarea } from '$lib/components/ui/textarea';
	import { SETTING_CONFIG_DEFAULT, SETTING_CONFIG_INFO, SETTINGS_KEYS } from '$lib/constants';
	import { SettingsFieldType } from '$lib/enums/settings';
	import { settingsStore } from '$lib/stores/settings.svelte';
	import { serverStore } from '$lib/stores/server.svelte';
	import { ChatSettingsParameterSourceIndicator } from '$lib/components/app';
	import { getSystemInfo, type SystemInfo } from '$lib/tauri-bridge';
	import { onMount } from 'svelte';
	import type { Component } from 'svelte';

	interface Props {
		fields: SettingsFieldConfig[];
		localConfig: SettingsConfigType;
		onConfigChange: (key: string, value: string | boolean | number) => void;
		onThemeChange?: (theme: string) => void;
	}

	let { fields, localConfig, onConfigChange, onThemeChange }: Props = $props();

	let systemInfo = $state<SystemInfo | null>(null);

	onMount(async () => {
		try {
			systemInfo = await getSystemInfo();
		} catch (e) {
			console.error('Failed to get system info:', e);
		}
	});

	// Helper function to get parameter source info for syncable parameters
	function getParameterSourceInfo(key: string) {
		if (!settingsStore.canSyncParameter(key)) {
			return null;
		}

		return settingsStore.getParameterInfo(key);
	}
</script>

{#each fields as field (field.key)}
	<div class="space-y-2">
		{#if field.type === SettingsFieldType.INPUT}
			{@const paramInfo = getParameterSourceInfo(field.key)}
			{@const currentValue = String(localConfig[field.key] ?? '')}
			{@const propsDefault = paramInfo?.serverDefault}
			{@const isCustomRealTime = (() => {
				if (!paramInfo || propsDefault === undefined) return false;

				// Apply same rounding logic for real-time comparison
				const inputValue = currentValue;
				const numericInput = parseFloat(inputValue);
				const normalizedInput = !isNaN(numericInput)
					? Math.round(numericInput * 1000000) / 1000000
					: inputValue;
				const normalizedDefault =
					typeof propsDefault === 'number'
						? Math.round(propsDefault * 1000000) / 1000000
						: propsDefault;

				return normalizedInput !== normalizedDefault;
			})()}

			<div class="flex items-center gap-2">
				<Label for={field.key} class="flex items-center gap-1.5 text-sm font-medium">
					{field.label}

					{#if field.isExperimental}
						<FlaskConical class="h-3.5 w-3.5 text-muted-foreground" />
					{/if}
				</Label>
				{#if isCustomRealTime}
					<ChatSettingsParameterSourceIndicator />
				{/if}
			</div>

			<div class="relative w-full md:max-w-md">
				<Input
					id={field.key}
					value={currentValue}
					oninput={(e) => {
						// Update local config immediately for real-time badge feedback
						onConfigChange(field.key, e.currentTarget.value);
					}}
					placeholder={`Default: ${SETTING_CONFIG_DEFAULT[field.key] ?? 'none'}`}
					class="w-full {isCustomRealTime ? 'pr-8' : ''}"
				/>
				{#if isCustomRealTime}
					<button
						type="button"
						onclick={() => {
							settingsStore.resetParameterToServerDefault(field.key);
							// Trigger UI update by calling onConfigChange with the default value
							const defaultValue = propsDefault ?? SETTING_CONFIG_DEFAULT[field.key];
							onConfigChange(field.key, String(defaultValue));
						}}
						class="absolute top-1/2 right-2 inline-flex h-5 w-5 -translate-y-1/2 items-center justify-center rounded transition-colors hover:bg-muted"
						aria-label="Reset to default"
						title="Reset to default"
					>
						<RotateCcw class="h-3 w-3" />
					</button>
				{/if}
			</div>
			{#if field.help || SETTING_CONFIG_INFO[field.key]}
				<p class="mt-1 text-xs text-muted-foreground">
					{@html field.help || SETTING_CONFIG_INFO[field.key]}
				</p>
			{/if}
		{:else if field.type === SettingsFieldType.TEXTAREA}
			<Label for={field.key} class="block flex items-center gap-1.5 text-sm font-medium">
				{field.label}

				{#if field.isExperimental}
					<FlaskConical class="h-3.5 w-3.5 text-muted-foreground" />
				{/if}
			</Label>

			<Textarea
				id={field.key}
				value={String(localConfig[field.key] ?? '')}
				onchange={(e) => onConfigChange(field.key, e.currentTarget.value)}
				placeholder={`Default: ${SETTING_CONFIG_DEFAULT[field.key] ?? 'none'}`}
				class="min-h-[10rem] w-full md:max-w-2xl"
			/>

			{#if field.help || SETTING_CONFIG_INFO[field.key]}
				<p class="mt-1 text-xs text-muted-foreground">
					{field.help || SETTING_CONFIG_INFO[field.key]}
				</p>
			{/if}

			{#if field.key === SETTINGS_KEYS.SYSTEM_MESSAGE}
				<div class="mt-3 flex items-center gap-2">
					<Checkbox
						id="showSystemMessage"
						checked={Boolean(localConfig.showSystemMessage ?? true)}
						onCheckedChange={(checked) => onConfigChange('showSystemMessage', Boolean(checked))}
					/>

					<Label for="showSystemMessage" class="cursor-pointer text-sm font-normal">
						Show system message in conversations
					</Label>
				</div>
			{/if}
		{:else if field.type === SettingsFieldType.SELECT}
			{@const selectedOption = field.options?.find(
				(opt: { value: string; label: string; icon?: Component }) =>
					opt.value === localConfig[field.key]
			)}
			{@const paramInfo = getParameterSourceInfo(field.key)}
			{@const currentValue = localConfig[field.key]}
			{@const propsDefault = paramInfo?.serverDefault}
			{@const isCustomRealTime = (() => {
				if (!paramInfo || propsDefault === undefined) return false;

				// For select fields, do direct comparison (no rounding needed)
				return currentValue !== propsDefault;
			})()}

			<div class="flex items-center gap-2">
				<Label for={field.key} class="flex items-center gap-1.5 text-sm font-medium">
					{field.label}

					{#if field.isExperimental}
						<FlaskConical class="h-3.5 w-3.5 text-muted-foreground" />
					{/if}
				</Label>
				{#if isCustomRealTime}
					<ChatSettingsParameterSourceIndicator />
				{/if}
			</div>

			<Select.Root
				type="single"
				value={currentValue}
				onValueChange={(value) => {
					if (field.key === SETTINGS_KEYS.THEME && value && onThemeChange) {
						onThemeChange(value);
					} else {
						onConfigChange(field.key, value);
					}
				}}
			>
				<div class="relative w-full md:w-auto md:max-w-md">
					<Select.Trigger class="w-full">
						<div class="flex items-center gap-2">
							{#if selectedOption?.icon}
								{@const IconComponent = selectedOption.icon}
								<IconComponent class="h-4 w-4" />
							{/if}

							{selectedOption?.label || `Select ${field.label.toLowerCase()}`}
						</div>
					</Select.Trigger>
					{#if isCustomRealTime}
						<button
							type="button"
							onclick={() => {
								settingsStore.resetParameterToServerDefault(field.key);
								// Trigger UI update by calling onConfigChange with the default value
								const defaultValue = propsDefault ?? SETTING_CONFIG_DEFAULT[field.key];
								onConfigChange(field.key, String(defaultValue));
							}}
							class="absolute top-1/2 right-8 inline-flex h-5 w-5 -translate-y-1/2 items-center justify-center rounded transition-colors hover:bg-muted"
							aria-label="Reset to default"
							title="Reset to default"
						>
							<RotateCcw class="h-3 w-3" />
						</button>
					{/if}
				</div>
				<Select.Content>
					{#if field.options}
						{#each field.options as option (option.value)}
							<Select.Item value={option.value} label={option.label}>
								<div class="flex items-center gap-2">
									{#if option.icon}
										{@const IconComponent = option.icon}
										<IconComponent class="h-4 w-4" />
									{/if}
									{option.label}
								</div>
							</Select.Item>
						{/each}
					{/if}
				</Select.Content>
			</Select.Root>
			{#if field.help || SETTING_CONFIG_INFO[field.key]}
				<p class="mt-1 text-xs text-muted-foreground">
					{field.help || SETTING_CONFIG_INFO[field.key]}
				</p>
			{/if}
		{:else if field.type === SettingsFieldType.SLIDER}
			{@const paramInfo = getParameterSourceInfo(field.key)}
			{@const currentValue = Number(localConfig[field.key] ?? SETTING_CONFIG_DEFAULT[field.key])}
			{@const propsDefault = paramInfo?.serverDefault}
			{@const isCustomRealTime = (() => {
				if (!paramInfo || propsDefault === undefined) return false;
				return currentValue !== propsDefault;
			})()}

			<div class="flex items-center justify-between gap-2">
				<div class="flex items-center gap-2">
					<Label for={field.key} class="flex items-center gap-1.5 text-sm font-medium">
						{field.label}
						{#if field.isExperimental}
							<FlaskConical class="h-3.5 w-3.5 text-muted-foreground" />
						{/if}
					</Label>
					{#if isCustomRealTime}
						<ChatSettingsParameterSourceIndicator />
					{/if}
				</div>
				<span class="text-xs font-mono bg-muted px-1.5 py-0.5 rounded text-muted-foreground">
					{currentValue}
				</span>
			</div>

			<div class="flex items-center gap-4 w-full md:max-w-md">
				<input
					type="range"
					id={field.key}
					min={field.min ?? 0}
					max={field.max ?? 100}
					step={field.step ?? 1}
					value={currentValue}
					oninput={(e) => onConfigChange(field.key, Number(e.currentTarget.value))}
					class="w-full h-1.5 bg-muted rounded-lg appearance-none cursor-pointer accent-primary"
				/>
				{#if isCustomRealTime}
					<button
						type="button"
						onclick={() => {
							settingsStore.resetParameterToServerDefault(field.key);
							const defaultValue = propsDefault ?? SETTING_CONFIG_DEFAULT[field.key];
							onConfigChange(field.key, Number(defaultValue));
						}}
						class="inline-flex h-5 w-5 items-center justify-center rounded transition-colors hover:bg-muted"
						aria-label="Reset to default"
						title="Reset to default"
					>
						<RotateCcw class="h-3 w-3" />
					</button>
				{/if}
			</div>

			{#if field.key === 'contextSize'}
				{@const estimatedRamGb = (currentValue * 102400) / (1024 * 1024 * 1024)}
				<!-- conservative estimate: approx 100KB per token for KV cache + overhead in 7B-8B models -->
				{@const isHighRam = systemInfo && estimatedRamGb > systemInfo.available_ram_gb * 0.5}
				{@const needsRestart = serverStore.contextSize !== null && serverStore.contextSize !== currentValue}

				<div class="mt-2 space-y-2">
					{#if isHighRam}
						<div
							class="flex items-start gap-2 p-2 rounded bg-destructive/10 border border-destructive/20 text-destructive text-xs"
						>
							<AlertCircle class="h-3.5 w-3.5 mt-0.5 flex-shrink-0" />
							<p>
								Varning: Högt kontextfönster kan kräva mer RAM än vad som är ledigt ({estimatedRamGb.toFixed(
									1
								)} GB uppskattat). Appen kan bli instabil.
							</p>
						</div>
					{/if}

					{#if needsRestart}
						<div
							class="flex items-start gap-2 p-2 rounded bg-amber-500/10 border border-amber-500/20 text-amber-600 dark:text-amber-400 text-xs"
						>
							<Info class="h-3.5 w-3.5 mt-0.5 flex-shrink-0" />
							<p>
								Ändringen slår igenom nästa gång du startar eller byter modell.
							</p>
						</div>
					{/if}
				</div>
			{/if}

			{#if field.key === 'contextSize'}
				<div class="mt-4 pt-4 border-t border-border/50">
					<div class="flex items-center gap-2 mb-2">
						<Label for="gpuIndex" class="text-sm font-medium">Grafikkortsindex (GPU)</Label>
						<FlaskConical class="h-3.5 w-3.5 text-muted-foreground transition-colors hover:text-primary" />
					</div>
					<div class="flex items-center gap-3">
						<Input
							id="gpuIndex"
							type="number"
							value={localConfig.gpuIndex ?? -1}
							oninput={(e) => onConfigChange('gpuIndex', Number(e.currentTarget.value))}
							placeholder="Auto (-1)"
							class="w-full md:max-w-[100px] h-9"
						/>
						<p class="text-[11px] leading-tight text-muted-foreground max-w-[280px]">
							<span class="font-medium text-foreground">Standard: Auto (-1)</span>. 
							Använd 0 eller 1 för att tvinga fram rätt grafikkort på system med dubbla GPU:er (t.ex. bärbara).
						</p>
					</div>
				</div>
			{/if}

			{#if field.help || SETTING_CONFIG_INFO[field.key]}
				<p class="mt-1 text-xs text-muted-foreground">
					{@html field.help || SETTING_CONFIG_INFO[field.key]}
				</p>
			{/if}
		{:else if field.type === SettingsFieldType.CHECKBOX}
			<div class="flex items-start space-x-3">
				<Checkbox
					id={field.key}
					checked={Boolean(localConfig[field.key])}
					onCheckedChange={(checked) => onConfigChange(field.key, checked)}
					class="mt-1"
				/>

				<div class="space-y-1">
					<label
						for={field.key}
						class="flex cursor-pointer items-center gap-1.5 pt-1 pb-0.5 text-sm leading-none font-medium"
					>
						{field.label}

						{#if field.isExperimental}
							<FlaskConical class="h-3.5 w-3.5 text-muted-foreground" />
						{/if}
					</label>

					{#if field.help || SETTING_CONFIG_INFO[field.key]}
						<p class="text-xs text-muted-foreground">
							{field.help || SETTING_CONFIG_INFO[field.key]}
						</p>
					{/if}
				</div>
			</div>
		{/if}
	</div>
{/each}
