export type ThemeChoice = "system" | "light" | "dark";

export interface TemplateSampling {
  temperature?: number;
  repeatPenalty?: number;
  presencePenalty?: number;
}

export interface SummaryTemplate {
  id: string;
  label: string;
  description: string;
  systemPrompt: string;
  builtin?: boolean;
  iconName?:
    | "notebook"
    | "file-text"
    | "list-checks"
    | "clipboard-list"
    | "gavel"
    | "eraser"
    | "sparkles";
  sampling?: TemplateSampling;
}

export interface ModelEntryStatus {
  id: string;
  label: string;
  shortLabel: string;
  description: string;
  filename: string;
  expectedBytes: number;
  fileSizeBytes: number;
  recommendedMinRamGb: number;
  installed: boolean;
  loaded: boolean;
  source: string; // "loki" | "maskera" | "portable" | ""
  canDelete: boolean;
}

export interface ModelsOverview {
  activeModelId: string;
  loadedModelId: string | null;
  anyInstalled: boolean;
  totalRamGb: number;
  safeCtxTokens: number;
  models: ModelEntryStatus[];
}

export interface ModelDownloadProgress {
  modelId: string;
  phase: "downloading" | "copying" | "loading" | "ready";
  label: string;
  downloadedBytes: number;
  totalBytes: number;
}

export interface TranscriptStats {
  rawChars: number;
  cleanedChars: number;
  charsSavedPercent: number;
  wordCount: number;
  estimatedTokens: number;
  approxMinutes: number;
  wasCleaned: boolean;
  consolidatedTurns: number;
}

export interface CleanedTranscript {
  text: string;
  stats: TranscriptStats;
}

export interface ImportedDocument {
  fileName: string;
  format: string;
  text: string;
  summary: string;
  transcriptStats: TranscriptStats | null;
}

export interface SummarizeRequest {
  transcript: string;
  templateId: string;
  templateLabel: string;
  templatePrompt: string;
  isBuiltinTemplate: boolean;
  context?: string;
  agenda?: string;
  creative: boolean;
  thorough: boolean;
  modelId?: string;
  sampling?: TemplateSampling;
}

export interface SummarizeProgressPayload {
  phase:
    | "loading_model"
    | "prefill"
    | "thinking"
    | "generating"
    | "reducing";
  label: string;
  prefillPercent: number;
  kvReusedTokens: number;
  promptTokens: number;
  thinkingTokens: number;
  generatedTokens: number;
  tokensPerSec: number;
  chunkCurrent: number | null;
  chunkTotal: number | null;
}

export interface SummarizeChunkPayload {
  fullText: string;
  generatedTokens: number;
  tokensPerSec: number;
}

export interface SummarizeResult {
  markdown: string;
  modelId: string;
  modelLabel: string;
  promptTokens: number;
  kvReusedTokens: number;
  generatedTokens: number;
  thinkingTokens: number;
  durationMs: number;
  tokensPerSec: number;
  strategy: "single" | "chunked";
  usedTemperature: number;
}

export interface SavedContextPreset {
  id: string;
  label: string;
  text: string;
}

export interface HistoryEntry {
  id: string;
  createdAt: string;
  meetingName: string;
  templateId: string;
  templateLabel: string;
  modelLabel: string;
  markdown: string;
  context?: string | null;
  durationMs: number;
}

export interface LocalAppState {
  activeModelId: string;
  customTemplates: SummaryTemplate[];
  savedContexts: SavedContextPreset[];
  history: HistoryEntry[];
}

