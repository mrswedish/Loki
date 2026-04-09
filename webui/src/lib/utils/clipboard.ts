import { toast } from 'svelte-sonner';
import { remark } from 'remark';
import remarkGfm from 'remark-gfm';
import remarkBreaks from 'remark-breaks';
import remarkRehype from 'remark-rehype';
import rehypeStringify from 'rehype-stringify';
import { AttachmentType } from '$lib/enums';
import { AGENTIC_REGEX } from '$lib/constants';
import type {
	DatabaseMessageExtra,
	DatabaseMessageExtraTextFile,
	DatabaseMessageExtraLegacyContext,
	DatabaseMessageExtraMcpPrompt,
	DatabaseMessageExtraMcpResource,
	ClipboardTextAttachment,
	ClipboardMcpPromptAttachment,
	ClipboardAttachment,
	ParsedClipboardContent
} from '$lib/types';

/**
 * Strips markdown syntax from text, converting it to plain readable text.
 * Headers, bold, italic, links, code blocks etc are converted to their text content.
 */
export function stripMarkdown(text: string): string {
	return text
		.replace(/```[\w-]*\n?([\s\S]*?)```/g, '$1') // fenced code blocks → content only
		.replace(/`([^`]+)`/g, '$1') // inline code
		.replace(/^#{1,6}\s+/gm, '') // headers
		.replace(/\*{1,3}([^*\n]+)\*{1,3}/g, '$1') // bold/italic
		.replace(/_{1,3}([^_\n]+)_{1,3}/g, '$1') // underscore bold/italic
		.replace(/!\[([^\]]*)\]\([^)]+\)/g, '$1') // images → alt text
		.replace(/\[([^\]]+)\]\([^)]+\)/g, '$1') // links → link text
		.replace(/^>\s+/gm, '') // blockquotes
		.replace(/^[-*+]\s+/gm, '• ') // unordered lists → bullets
		.replace(/~~([^~]+)~~/g, '$1') // ~~strikethrough~~
		.replace(/^---+$/gm, '') // horizontal rules
		.trim();
}

/**
 * Strips reasoning/thinking blocks and agentic tool call markers from content,
 * then converts markdown to plain text. Use for clipboard and text exports.
 */
export function stripReasoningAndMarkdown(text: string): string {
	const cleaned = text
		.replace(AGENTIC_REGEX.REASONING_BLOCK, '')
		.replace(AGENTIC_REGEX.REASONING_OPEN, '')
		.replace(AGENTIC_REGEX.AGENTIC_TOOL_CALL_BLOCK, '')
		.replace(AGENTIC_REGEX.AGENTIC_TOOL_CALL_OPEN, '')
		.trim();
	return stripMarkdown(cleaned);
}

/**
 * Copy text to clipboard with toast notification
 * Uses modern clipboard API when available, falls back to legacy method for non-secure contexts
 * @param text - Text to copy to clipboard
 * @param successMessage - Custom success message (optional)
 * @param errorMessage - Custom error message (optional)
 * @returns Promise<boolean> - True if successful, false otherwise
 */
export async function copyToClipboard(
	text: string,
	successMessage = 'Copied to clipboard',
	errorMessage = 'Failed to copy to clipboard'
): Promise<boolean> {
	try {
		// Try modern clipboard API first (secure contexts only)
		if (navigator.clipboard && navigator.clipboard.writeText) {
			await navigator.clipboard.writeText(text);
			toast.success(successMessage);
			return true;
		}

		// Fallback for non-secure contexts
		const textArea = document.createElement('textarea');
		textArea.value = text;
		textArea.style.position = 'fixed';
		textArea.style.left = '-999999px';
		textArea.style.top = '-999999px';
		document.body.appendChild(textArea);
		textArea.focus();
		textArea.select();

		const successful = document.execCommand('copy');
		document.body.removeChild(textArea);

		if (successful) {
			toast.success(successMessage);
			return true;
		} else {
			throw new Error('execCommand failed');
		}
	} catch (error) {
		console.error('Failed to copy to clipboard:', error);
		toast.error(errorMessage);
		return false;
	}
}

/**
 * Copy code with HTML entity decoding and toast notification
 * @param rawCode - Raw code string that may contain HTML entities
 * @param successMessage - Custom success message (optional)
 * @param errorMessage - Custom error message (optional)
 * @returns Promise<boolean> - True if successful, false otherwise
 */
export async function copyCodeToClipboard(
	rawCode: string,
	successMessage = 'Code copied to clipboard',
	errorMessage = 'Failed to copy code'
): Promise<boolean> {
	return copyToClipboard(rawCode, successMessage, errorMessage);
}

/**
 * Converts markdown to HTML using remark/rehype pipeline.
 * Supports GFM (tables, strikethrough, task lists) and line breaks.
 */
async function markdownToHtml(markdown: string): Promise<string> {
	const result = await remark()
		.use(remarkGfm)
		.use(remarkBreaks)
		.use(remarkRehype)
		.use(rehypeStringify)
		.process(markdown);
	return String(result);
}

/**
 * Copy message content as rich text (HTML + plain text fallback).
 * Email clients and rich text editors receive formatted HTML with headers,
 * bold, bullet lists etc. Plain text editors receive stripped plain text.
 *
 * @param cleanedMarkdown - Markdown with reasoning already stripped
 * @param successMessage - Custom success message (optional)
 * @param errorMessage - Custom error message (optional)
 * @returns Promise<boolean> - True if successful, false otherwise
 */
export async function copyRichToClipboard(
	cleanedMarkdown: string,
	successMessage = 'Copied to clipboard',
	errorMessage = 'Failed to copy to clipboard'
): Promise<boolean> {
	try {
		if (navigator.clipboard && 'write' in navigator.clipboard) {
			const html = await markdownToHtml(cleanedMarkdown);
			const plainText = stripMarkdown(cleanedMarkdown);
			const item = new ClipboardItem({
				'text/html': new Blob([html], { type: 'text/html' }),
				'text/plain': new Blob([plainText], { type: 'text/plain' })
			});
			await navigator.clipboard.write([item]);
			toast.success(successMessage);
			return true;
		}
	} catch {
		// Fall through to plain text fallback
	}
	return copyToClipboard(stripMarkdown(cleanedMarkdown), successMessage, errorMessage);
}

/**
 * Formats a message with text attachments for clipboard copying.
 *
 * Default format (asPlainText = false):
 * ```
 * "Text message content"
 * [
 *   {"type":"TEXT","name":"filename.txt","content":"..."},
 *   {"type":"TEXT","name":"another.txt","content":"..."}
 * ]
 * ```
 *
 * Plain text format (asPlainText = true):
 * ```
 * Text message content
 *
 * file content here
 *
 * another file content
 * ```
 *
 * @param content - The message text content
 * @param extras - Optional array of message attachments
 * @param asPlainText - If true, format as plain text without JSON structure
 * @returns Formatted string for clipboard
 */
export function formatMessageForClipboard(
	content: string,
	extras?: DatabaseMessageExtra[],
	asPlainText: boolean = false
): string {
	// Always strip reasoning and markdown from the main content
	const cleanContent = stripReasoningAndMarkdown(content);

	// Filter text-like attachments (TEXT, LEGACY_CONTEXT, MCP_PROMPT, and MCP_RESOURCE types)
	const textAttachments =
		extras?.filter(
			(
				extra
			): extra is
				| DatabaseMessageExtraTextFile
				| DatabaseMessageExtraLegacyContext
				| DatabaseMessageExtraMcpPrompt
				| DatabaseMessageExtraMcpResource =>
				extra.type === AttachmentType.TEXT ||
				extra.type === AttachmentType.LEGACY_CONTEXT ||
				extra.type === AttachmentType.MCP_PROMPT ||
				extra.type === AttachmentType.MCP_RESOURCE
		) ?? [];

	if (textAttachments.length === 0) {
		return cleanContent;
	}

	if (asPlainText) {
		const parts = [cleanContent];
		for (const att of textAttachments) {
			parts.push(att.content);
		}
		return parts.join('\n\n');
	}

	const clipboardAttachments: ClipboardAttachment[] = textAttachments.map((att) => {
		if (att.type === AttachmentType.MCP_PROMPT) {
			const mcpAtt = att as DatabaseMessageExtraMcpPrompt;
			return {
				type: AttachmentType.MCP_PROMPT,
				name: mcpAtt.name,
				serverName: mcpAtt.serverName,
				promptName: mcpAtt.promptName,
				content: mcpAtt.content,
				arguments: mcpAtt.arguments
			} as ClipboardMcpPromptAttachment;
		}
		return {
			type: AttachmentType.TEXT,
			name: att.name,
			content: att.content
		} as ClipboardTextAttachment;
	});

	return `${JSON.stringify(cleanContent)}\n${JSON.stringify(clipboardAttachments, null, 2)}`;
}

/**
 * Parses clipboard content to extract message and text attachments.
 * Supports both plain text and the special format with attachments.
 *
 * @param clipboardText - Raw text from clipboard
 * @returns Parsed content with message and attachments
 */
export function parseClipboardContent(clipboardText: string): ParsedClipboardContent {
	const defaultResult: ParsedClipboardContent = {
		message: clipboardText,
		textAttachments: [],
		mcpPromptAttachments: []
	};

	if (!clipboardText.startsWith('"')) {
		return defaultResult;
	}

	try {
		let stringEndIndex = -1;
		let escaped = false;

		for (let i = 1; i < clipboardText.length; i++) {
			const char = clipboardText[i];

			if (escaped) {
				escaped = false;
				continue;
			}

			if (char === '\\') {
				escaped = true;
				continue;
			}

			if (char === '"') {
				stringEndIndex = i;
				break;
			}
		}

		if (stringEndIndex === -1) {
			return defaultResult;
		}

		const jsonStringPart = clipboardText.substring(0, stringEndIndex + 1);
		const remainingPart = clipboardText.substring(stringEndIndex + 1).trim();

		const message = JSON.parse(jsonStringPart) as string;

		if (!remainingPart || !remainingPart.startsWith('[')) {
			return {
				message,
				textAttachments: [],
				mcpPromptAttachments: []
			};
		}

		const attachments = JSON.parse(remainingPart) as unknown[];

		const validTextAttachments: ClipboardTextAttachment[] = [];
		const validMcpPromptAttachments: ClipboardMcpPromptAttachment[] = [];

		for (const att of attachments) {
			if (isValidMcpPromptAttachment(att)) {
				validMcpPromptAttachments.push({
					type: AttachmentType.MCP_PROMPT,
					name: att.name,
					serverName: att.serverName,
					promptName: att.promptName,
					content: att.content,
					arguments: att.arguments
				});
			} else if (isValidTextAttachment(att)) {
				validTextAttachments.push({
					type: AttachmentType.TEXT,
					name: att.name,
					content: att.content
				});
			}
		}

		return {
			message,
			textAttachments: validTextAttachments,
			mcpPromptAttachments: validMcpPromptAttachments
		};
	} catch {
		return defaultResult;
	}
}

/**
 * Type guard to validate an MCP prompt attachment object
 * @param obj The object to validate
 * @returns true if the object is a valid MCP prompt attachment
 */
function isValidMcpPromptAttachment(obj: unknown): obj is {
	type: string;
	name: string;
	serverName: string;
	promptName: string;
	content: string;
	arguments?: Record<string, string>;
} {
	if (typeof obj !== 'object' || obj === null) {
		return false;
	}

	const record = obj as Record<string, unknown>;

	return (
		(record.type === AttachmentType.MCP_PROMPT || record.type === 'MCP_PROMPT') &&
		typeof record.name === 'string' &&
		typeof record.serverName === 'string' &&
		typeof record.promptName === 'string' &&
		typeof record.content === 'string'
	);
}

/**
 * Type guard to validate a text attachment object
 * @param obj The object to validate
 * @returns true if the object is a valid text attachment
 */
function isValidTextAttachment(
	obj: unknown
): obj is { type: string; name: string; content: string } {
	if (typeof obj !== 'object' || obj === null) {
		return false;
	}

	const record = obj as Record<string, unknown>;

	return (
		(record.type === AttachmentType.TEXT || record.type === 'TEXT') &&
		typeof record.name === 'string' &&
		typeof record.content === 'string'
	);
}

/**
 * Checks if clipboard content contains our special format with attachments
 * @param clipboardText - Raw text from clipboard
 * @returns true if the clipboard content contains our special format with attachments
 */
export function hasClipboardAttachments(clipboardText: string): boolean {
	if (!clipboardText.startsWith('"')) {
		return false;
	}

	const parsed = parseClipboardContent(clipboardText);
	return parsed.textAttachments.length > 0 || parsed.mcpPromptAttachments.length > 0;
}
