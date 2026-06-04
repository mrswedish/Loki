<script lang="ts">
	import SummarizeScreen from '$lib/components/app/summarize/SummarizeScreen.svelte';
	import { conversationsStore, isConversationsInitialized } from '$lib/stores/conversations.svelte';
	import { chatStore } from '$lib/stores/chat.svelte';
	import { summarizeStore } from '$lib/stores/summarize.svelte';
	import { onMount } from 'svelte';

	onMount(async () => {
		if (!isConversationsInitialized()) {
			await conversationsStore.initialize();
		}
		// Börja alltid från ett rent läge i sammanfattningsvyn.
		conversationsStore.clearActiveConversation();
		chatStore.clearUIState();
		summarizeStore.reset();
	});
</script>

<svelte:head>
	<title>Loki – Sammanfatta transkriberingar</title>
</svelte:head>

<SummarizeScreen />
