<script lang="ts">
	import { Edit, Copy, RefreshCw, Trash2, ArrowRight } from '@lucide/svelte';
	import {
		ActionIcon,
		ChatMessageBranchingControls,
		DialogConfirmation
	} from '$lib/components/app';
	import { Switch } from '$lib/components/ui/switch';
	import { MessageRole } from '$lib/enums';

	interface Props {
		role: MessageRole.USER | MessageRole.ASSISTANT;
		justify: 'start' | 'end';
		actionsPosition: 'left' | 'right';
		siblingInfo?: ChatMessageSiblingInfo | null;
		showDeleteDialog: boolean;
		deletionInfo: {
			totalCount: number;
			userMessages: number;
			assistantMessages: number;
			messageTypes: string[];
		} | null;
		onCopy: () => void;
		onEdit?: () => void;
		onRegenerate?: () => void;
		onContinue?: () => void;
		onDelete: () => void;
		onConfirmDelete: () => void;
		onNavigateToSibling?: (siblingId: string) => void;
		onShowDeleteDialogChange: (show: boolean) => void;
		showRawOutputSwitch?: boolean;
		rawOutputEnabled?: boolean;
		onRawOutputToggle?: (enabled: boolean) => void;
	}

	let {
		actionsPosition,
		deletionInfo,
		justify,
		onCopy,
		onEdit,
		onConfirmDelete,
		onContinue,
		onDelete,
		onNavigateToSibling,
		onShowDeleteDialogChange,
		onRegenerate,
		role,
		siblingInfo = null,
		showDeleteDialog,
		showRawOutputSwitch = false,
		rawOutputEnabled = false,
		onRawOutputToggle
	}: Props = $props();

	function handleConfirmDelete() {
		onConfirmDelete();
		onShowDeleteDialogChange(false);
	}
</script>

<div class="relative {justify === 'start' ? 'mt-2' : ''} flex h-6 items-center justify-between">
	<div
		class="{actionsPosition === 'left'
			? 'left-0'
			: 'right-0'} flex items-center gap-2 opacity-100 transition-opacity"
	>
		{#if siblingInfo && siblingInfo.totalSiblings > 1}
			<ChatMessageBranchingControls {siblingInfo} {onNavigateToSibling} />
		{/if}

		<div
			class="pointer-events-auto inset-0 flex items-center gap-1 opacity-100 transition-all duration-150"
		>
			<ActionIcon icon={Copy} tooltip="Kopiera" onclick={onCopy} />

			{#if onEdit}
				<ActionIcon icon={Edit} tooltip="Redigera" onclick={onEdit} />
			{/if}

			{#if role === MessageRole.ASSISTANT && onRegenerate}
				<ActionIcon icon={RefreshCw} tooltip="Generera om" onclick={() => onRegenerate()} />
			{/if}

			{#if role === MessageRole.ASSISTANT && onContinue}
				<ActionIcon icon={ArrowRight} tooltip="Fortsätt" onclick={onContinue} />
			{/if}

			<ActionIcon icon={Trash2} tooltip="Radera" onclick={onDelete} />
		</div>
	</div>

	{#if showRawOutputSwitch}
		<div class="flex items-center gap-2">
			<span class="text-xs text-muted-foreground">Visa rå utdata</span>
			<Switch
				checked={rawOutputEnabled}
				onCheckedChange={(checked) => onRawOutputToggle?.(checked)}
			/>
		</div>
	{/if}
</div>

<DialogConfirmation
	bind:open={showDeleteDialog}
	title="Radera meddelande"
	description={deletionInfo && deletionInfo.totalCount > 1
		? `Det här raderar ${deletionInfo.totalCount} meddelanden: ${deletionInfo.userMessages} användarmeddelande${deletionInfo.userMessages > 1 ? 'n' : ''} och ${deletionInfo.assistantMessages} assistantsvar. Åtgärden kan inte ångras.`
		: 'Är du säker på att du vill radera detta meddelande? Åtgärden kan inte ångras.'}
	confirmText={deletionInfo && deletionInfo.totalCount > 1
		? `Radera ${deletionInfo.totalCount} meddelanden`
		: 'Radera'}
	cancelText="Avbryt"
	variant="destructive"
	icon={Trash2}
	onConfirm={handleConfirmDelete}
	onCancel={() => onShowDeleteDialogChange(false)}
/>
