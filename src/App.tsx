import React, { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import {
  Brain,
  Check,
  ClipboardList,
  ClipboardPaste,
  Copy,
  Cpu,
  Download,
  Eraser,
  Eye,
  FileDown,
  Files,
  FileText,
  FolderOpen,
  Gavel,
  History,
  ListChecks,
  Monitor,
  Moon,
  NotebookPen,
  PanelLeftClose,
  PanelLeftOpen,
  Pencil,
  Play,
  Plus,
  RefreshCw,
  ScrollText,
  Sparkles,
  Square,
  Sun,
  Trash2,
  Undo2,
  Upload,
  Wand2,
  X,
} from "lucide-react";

import {
  markdownToCleanHtml,
  markdownToCleanPlainText,
  renderFormattedMarkdown,
} from "./markdownUtils";
import {
  BUILTIN_TEMPLATES,
  DEFAULT_TEMPLATE_ID,
  SUPPORTED_DOCUMENT_EXTENSIONS,
} from "./templates";
import {
  CleanedTranscript,
  HistoryEntry,
  ImportedDocument,
  LocalAppState,
  ModelDownloadProgress,
  ModelsOverview,
  SavedContextPreset,
  SummarizeChunkPayload,
  SummarizeProgressPayload,
  SummarizeResult,
  SummaryTemplate,
  TemplateSampling,
  ThemeChoice,
  TranscriptStats,
} from "./types";

function formatBytes(bytes: number): string {
  if (bytes <= 0) return "0 MB";
  const gb = bytes / 1_000_000_000;
  if (gb >= 1) return `${gb.toFixed(2).replace(".", ",")} GB`;
  const mb = bytes / 1_000_000;
  return `${Math.round(mb)} MB`;
}

function sanitizeFilename(name: string): string {
  const cleaned = name
    .trim()
    .replace(/[<>:"/\\|?*\u0000-\u001F]+/g, "-")
    .replace(/\s+/g, " ")
    .trim();
  return cleaned || `Motesanteckning-${new Date().toISOString().slice(0, 10)}`;
}

function renderTemplateIcon(iconName?: SummaryTemplate["iconName"]) {
  switch (iconName) {
    case "notebook":
      return <NotebookPen size={15} aria-hidden="true" />;
    case "file-text":
      return <FileText size={15} aria-hidden="true" />;
    case "list-checks":
      return <ListChecks size={15} aria-hidden="true" />;
    case "clipboard-list":
      return <ClipboardList size={15} aria-hidden="true" />;
    case "gavel":
      return <Gavel size={15} aria-hidden="true" />;
    case "eraser":
      return <Eraser size={15} aria-hidden="true" />;
    default:
      return <Sparkles size={15} aria-hidden="true" />;
  }
}

function getInitialTheme(): ThemeChoice {
  try {
    const saved = localStorage.getItem("color-scheme");
    if (saved === "light" || saved === "dark") return saved;
  } catch {
    // Ignorera lagringsfel i begränsade miljöer
  }
  return "system";
}

function applyThemeChoice(choice: ThemeChoice) {
  const meta = document.querySelector('meta[name="color-scheme"]');
  if (meta) {
    meta.setAttribute("content", choice === "system" ? "light dark" : choice);
  }
  try {
    if (choice === "system") {
      localStorage.removeItem("color-scheme");
    } else {
      localStorage.setItem("color-scheme", choice);
    }
  } catch {
    // Ignorera lagringsfel
  }
}

export default function App() {
  // ── Tema (MWG dark-mode-toggle) ──
  const [theme, setTheme] = useState<ThemeChoice>(getInitialTheme);

  // ── Modeller & Nedladdning ──
  const [modelsOverview, setModelsOverview] = useState<ModelsOverview | null>(
    null
  );
  const [downloadProgress, setDownloadProgress] =
    useState<ModelDownloadProgress | null>(null);
  const [downloadingModelId, setDownloadingModelId] = useState<string | null>(
    null
  );

  // ── Lokalt sparade egna mallar, sammanhang och historik ──
  const [customTemplates, setCustomTemplates] = useState<SummaryTemplate[]>([]);
  const [savedContexts, setSavedContexts] = useState<SavedContextPreset[]>([]);
  const [history, setHistory] = useState<HistoryEntry[]>([]);
  const [historyQuery, setHistoryQuery] = useState("");

  // ── Mötesinmatning & Tillval ──
  const [meetingName, setMeetingName] = useState("");
  const [transcript, setTranscript] = useState("");
  const [transcriptSourceSummary, setTranscriptSourceSummary] = useState<
    string | null
  >(null);
  const [transcriptStats, setTranscriptStats] =
    useState<TranscriptStats | null>(null);
  const [isDraggingFile, setIsDraggingFile] = useState(false);

  const [selectedTemplateId, setSelectedTemplateId] =
    useState<string>(DEFAULT_TEMPLATE_ID);
  const [contextText, setContextText] = useState("");
  const [agendaOpen, setAgendaOpen] = useState(false);
  const [agendaText, setAgendaText] = useState("");
  const [agendaFileName, setAgendaFileName] = useState<string | null>(null);
  const [creativeMode, setCreativeMode] = useState(false);
  const [thoroughMode, setThoroughMode] = useState(false);

  // ── Körning, Streaming & Resultat ──
  const [isRunning, setIsRunning] = useState(false);
  const [runProgress, setRunProgress] =
    useState<SummarizeProgressPayload | null>(null);
  const [resultMarkdown, setResultMarkdown] = useState("");
  const [lastResultMeta, setLastResultMeta] = useState<{
    templateLabel: string;
    modelLabel: string;
    durationMs: number;
    kvReusedTokens: number;
    tokensPerSec: number;
    strategy: string;
  } | null>(null);
  const [undoStack, setUndoStack] = useState<string[]>([]);
  const [adjustDirective, setAdjustDirective] = useState("");
  const [isEditingResult, setIsEditingResult] = useState(false);
  const [focusResultView, setFocusResultView] = useState(false);
  const [currentHistoryId, setCurrentHistoryId] = useState<string | null>(null);

  // ── Återkoppling / Live Region & Fel ──
  const [errorMessage, setErrorMessage] = useState<string | null>(null);
  const [statusAnnouncement, setStatusAnnouncement] = useState("");
  const [toastText, setToastText] = useState<string | null>(null);
  const toastTimerRef = useRef<number | null>(null);

  // ── Malleditor-tillstånd (<dialog>) ──
  const [editingTemplateId, setEditingTemplateId] = useState<string | null>(
    null
  );
  const [tplLabel, setTplLabel] = useState("");
  const [tplDesc, setTplDesc] = useState("");
  const [tplPrompt, setTplPrompt] = useState("");
  const [tplTemp, setTplTemp] = useState("");
  const [tplRepeat, setTplRepeat] = useState("");

  // ── Native <dialog> & dolda <input type="file">-referenser ──
  const modelDialogRef = useRef<HTMLDialogElement | null>(null);
  const historyDialogRef = useRef<HTMLDialogElement | null>(null);
  const templateDialogRef = useRef<HTMLDialogElement | null>(null);
  const logDialogRef = useRef<HTMLDialogElement | null>(null);
  const [logInfo, setLogInfo] = useState<{
    path: string;
    content: string;
    accelerator: string;
  } | null>(null);
  const transcriptFileInputRef = useRef<HTMLInputElement | null>(null);
  const agendaFileInputRef = useRef<HTMLInputElement | null>(null);
  const templateFileInputRef = useRef<HTMLInputElement | null>(null);

  const allTemplates = useMemo(
    () => [...BUILTIN_TEMPLATES, ...customTemplates],
    [customTemplates]
  );

  const selectedTemplate = useMemo(
    () =>
      allTemplates.find((t) => t.id === selectedTemplateId) ??
      BUILTIN_TEMPLATES[0],
    [allTemplates, selectedTemplateId]
  );

  const activeModelEntry = useMemo(() => {
    if (!modelsOverview) return null;
    return (
      modelsOverview.models.find(
        (m) => m.id === modelsOverview.activeModelId
      ) ?? modelsOverview.models[0]
    );
  }, [modelsOverview]);

  function notifyStatus(msg: string) {
    setStatusAnnouncement(msg);
    setToastText(msg);
    if (toastTimerRef.current !== null) {
      window.clearTimeout(toastTimerRef.current);
    }
    toastTimerRef.current = window.setTimeout(() => {
      setToastText(null);
    }, 2600);
  }

  // ── Initiering & Tauri-lyssnare ──
  useEffect(() => {
    applyThemeChoice(theme);
  }, [theme]);

  useEffect(() => {
    let disposed = false;
    let unlistenDl: (() => void) | undefined;
    let unlistenProg: (() => void) | undefined;
    let unlistenChunk: (() => void) | undefined;
    let unlistenModels: (() => void) | undefined;

    async function bootstrap() {
      try {
        const [overview, localState] = await Promise.all([
          invoke<ModelsOverview>("get_models_status"),
          invoke<LocalAppState>("load_local_state"),
        ]);
        if (disposed) return;
        setModelsOverview(overview);
        setCustomTemplates(
          (localState.customTemplates ?? []).map((t) => ({
            ...t,
            builtin: false,
            iconName: "sparkles",
          }))
        );
        setSavedContexts(localState.savedContexts ?? []);
        setHistory(localState.history ?? []);
      } catch (err) {
        if (!disposed) {
          setErrorMessage(`Kunde inte läsa lokalt programtillstånd: ${String(err)}`);
        }
      }

      const dlFn = await listen<ModelDownloadProgress>(
        "model_download_progress",
        (ev) => {
          setDownloadProgress(ev.payload);
        }
      );
      if (disposed) {
        dlFn();
        return;
      }
      unlistenDl = dlFn;

      const progFn = await listen<SummarizeProgressPayload>(
        "summarize_progress",
        (ev) => {
          setRunProgress(ev.payload);
        }
      );
      if (disposed) {
        progFn();
        return;
      }
      unlistenProg = progFn;

      const chunkFn = await listen<SummarizeChunkPayload>(
        "summarize_chunk",
        (ev) => {
          setResultMarkdown(ev.payload.fullText);
        }
      );
      if (disposed) {
        chunkFn();
        return;
      }
      unlistenChunk = chunkFn;

      const modelsFn = await listen<ModelsOverview>(
        "models_status_changed",
        (ev) => {
          setModelsOverview(ev.payload);
        }
      );
      if (disposed) {
        modelsFn();
        return;
      }
      unlistenModels = modelsFn;
    }

    void bootstrap();

    return () => {
      disposed = true;
      if (unlistenDl) unlistenDl();
      if (unlistenProg) unlistenProg();
      if (unlistenChunk) unlistenChunk();
      if (unlistenModels) unlistenModels();
    };
  }, []);

  // Uppdatera token-/minutstatistik deterministiskt när transkriptet redigeras manuellt
  useEffect(() => {
    if (!transcript.trim()) {
      setTranscriptStats(null);
      return;
    }
    const timer = window.setTimeout(async () => {
      try {
        const res = await invoke<CleanedTranscript>("clean_transcript_text", {
          raw: transcript,
          fileExt: null,
        });
        setTranscriptStats(res.stats);
      } catch {
        // Ignorera under pågående inmatning
      }
    }, 180);
    return () => window.clearTimeout(timer);
  }, [transcript]);

  // Synka redigering i resultatdokumentet till aktuell historikpost
  async function persistHistoryMarkdownUpdate(updatedMd: string) {
    if (!currentHistoryId || !updatedMd.trim()) return;
    const existing = history.find((h) => h.id === currentHistoryId);
    if (!existing) return;
    try {
      const next = await invoke<HistoryEntry[]>("save_history_entry", {
        entry: { ...existing, markdown: updatedMd },
      });
      setHistory(next);
    } catch {
      // Tyst om diskskrivning misslyckas tillfälligt
    }
  }

  // ── Tema-cykling ──
  function cycleTheme() {
    const next: ThemeChoice =
      theme === "system" ? "light" : theme === "light" ? "dark" : "system";
    setTheme(next);
  }

  // ── Modellhantering ──
  async function handleDownloadModel(modelId: string) {
    setErrorMessage(null);
    setDownloadingModelId(modelId);
    try {
      const updated = await invoke<ModelsOverview>("download_model", {
        modelId,
      });
      setModelsOverview(updated);
      notifyStatus("Språkmodellen är installerad och klar att användas.");
    } catch (err) {
      const msg = String(err);
      if (!msg.includes("AVBRUTEN")) {
        setErrorMessage(`Kunde inte hämta modellen: ${msg}`);
      } else {
        notifyStatus("Nedladdningen avbröts.");
      }
    } finally {
      setDownloadingModelId(null);
      setDownloadProgress(null);
    }
  }

  async function handleCancelDownload() {
    await invoke("cancel_model_download");
  }

  async function handleSelectModel(modelId: string) {
    setErrorMessage(null);
    try {
      const updated = await invoke<ModelsOverview>("select_active_model", {
        modelId,
      });
      setModelsOverview(updated);
      const picked = updated.models.find((m) => m.id === modelId);
      if (picked) {
        notifyStatus(`Bytte aktiv modell till ${picked.shortLabel}.`);
      }
    } catch (err) {
      setErrorMessage(`Kunde inte aktivera modellen: ${String(err)}`);
    } finally {
      setDownloadProgress(null);
    }
  }

  async function handleImportModelOffline(pickDirectory: boolean) {
    setErrorMessage(null);
    try {
      const picked = await open({
        directory: pickDirectory,
        multiple: false,
        title: pickDirectory
          ? "Välj mapp med GGUF-modell"
          : "Välj lokal GGUF-modellfil",
        filters: pickDirectory
          ? undefined
          : [{ name: "GGUF-språkmodell", extensions: ["gguf"] }],
      });
      if (!picked || typeof picked !== "string") return;
      setDownloadingModelId("offline");
      const updated = await invoke<ModelsOverview>("import_model_from_path", {
        srcPath: picked,
      });
      setModelsOverview(updated);
      notifyStatus("Lokal modell importerad och redo.");
    } catch (err) {
      setErrorMessage(`Offline-import misslyckades: ${String(err)}`);
    } finally {
      setDownloadingModelId(null);
      setDownloadProgress(null);
    }
  }

  async function handleDeleteModel(modelId: string) {
    setErrorMessage(null);
    try {
      const updated = await invoke<ModelsOverview>("delete_model", { modelId });
      setModelsOverview(updated);
      notifyStatus("Modellfilen togs bort från disken.");
    } catch (err) {
      setErrorMessage(`Kunde inte ta bort modellen: ${String(err)}`);
    }
  }

  // ── Dokument- & Transkriptinläsning (Word, Excel, PDF, ODT, VTT, SRT, TXT, Urklipp) ──
  async function ingestBrowserFile(file: File, target: "transcript" | "agenda") {
    setErrorMessage(null);
    try {
      const ab = await file.arrayBuffer();
      const bytes = Array.from(new Uint8Array(ab));
      const imported = await invoke<ImportedDocument>("import_document_bytes", {
        fileName: file.name,
        bytes,
        cleanAsTranscript: target === "transcript",
      });

      if (target === "transcript") {
        setTranscript(imported.text);
        setTranscriptSourceSummary(imported.summary);
        if (imported.transcriptStats) {
          setTranscriptStats(imported.transcriptStats);
        }
        if (!meetingName.trim()) {
          setMeetingName(file.name.replace(/\.[^.]+$/, ""));
        }
        notifyStatus(`Läste in ${file.name}.`);
      } else {
        setAgendaText(imported.text);
        setAgendaFileName(file.name);
        setAgendaOpen(true);
        notifyStatus(`Läste in agenda från ${file.name}.`);
      }
    } catch (err) {
      setErrorMessage(`Kunde inte läsa filen ${file.name}: ${String(err)}`);
    }
  }

  async function handlePasteTranscriptFromClipboard() {
    setErrorMessage(null);
    try {
      const rawClip = await navigator.clipboard.readText();
      if (!rawClip.trim()) {
        notifyStatus("Urklippet är tomt.");
        return;
      }
      const cleaned = await invoke<CleanedTranscript>("clean_transcript_text", {
        raw: rawClip,
        fileExt: null,
      });
      setTranscript(cleaned.text);
      setTranscriptStats(cleaned.stats);
      setTranscriptSourceSummary(
        cleaned.stats.charsSavedPercent > 0
          ? `Inklistrad text · Städad (-${cleaned.stats.charsSavedPercent} % tidsstämplar/brus)`
          : "Inklistrad mötestext"
      );
      notifyStatus("Mötestext inklistrad.");
    } catch {
      setErrorMessage(
        "Kunde inte läsa urklipp automatiskt. Klicka i textrutan och tryck Ctrl+V / Cmd+V."
      );
    }
  }

  async function handleTranscriptTextPaste(
    e: React.ClipboardEvent<HTMLTextAreaElement>
  ) {
    const pasted = e.clipboardData.getData("text/plain");
    if (
      pasted &&
      (pasted.includes("WEBVTT") ||
        pasted.includes("-->") ||
        /\b\d{1,2}:\d{2}:\d{2}[.,]\d{1,3}\b/.test(pasted))
    ) {
      e.preventDefault();
      const cleaned = await invoke<CleanedTranscript>("clean_transcript_text", {
        raw: pasted,
        fileExt: "vtt",
      });
      setTranscript(cleaned.text);
      setTranscriptStats(cleaned.stats);
      setTranscriptSourceSummary(
        `Inklistrat transkript · Städad (-${cleaned.stats.charsSavedPercent} % tidsstämplar/brus)`
      );
      notifyStatus("Städade bort tidsstämplar och slog ihop repliker automatiskt.");
    }
  }

  // ── Kör sammanfattning (och 1-klicks mallbyte på varm KV-cache!) ──
  async function runSummarizeWithTemplate(template: SummaryTemplate) {
    if (!transcript.trim()) {
      setErrorMessage("Lägg först in en mötestranskribering eller öppna ett dokument.");
      return;
    }
    if (!modelsOverview?.anyInstalled) {
      setErrorMessage(
        "Ladda först ner eller importera språkmodellen Gemma 4 E2B i rutan ovan."
      );
      return;
    }

    setErrorMessage(null);
    setSelectedTemplateId(template.id);
    if (resultMarkdown.trim()) {
      setUndoStack((prev) => [...prev.slice(-9), resultMarkdown]);
    }
    setIsEditingResult(false);
    setResultMarkdown("");
    setIsRunning(true);
    setRunProgress({
      phase: "prefill",
      label: "Förbereder sammanfattning…",
      prefillPercent: 0,
      kvReusedTokens: 0,
      promptTokens: 0,
      thinkingTokens: 0,
      generatedTokens: 0,
      tokensPerSec: 0,
      chunkCurrent: null,
      chunkTotal: null,
    });

    try {
      const res = await invoke<SummarizeResult>("summarize_meeting", {
        request: {
          transcript,
          templateId: template.id,
          templateLabel: template.label,
          templatePrompt: template.systemPrompt,
          isBuiltinTemplate: Boolean(template.builtin),
          context: contextText.trim() || undefined,
          agenda: agendaText.trim() || undefined,
          creative: creativeMode,
          thorough: thoroughMode,
          modelId: modelsOverview.activeModelId,
          sampling: template.sampling,
        },
      });

      setResultMarkdown(res.markdown);
      setLastResultMeta({
        templateLabel: template.label,
        modelLabel: res.modelLabel,
        durationMs: res.durationMs,
        kvReusedTokens: res.kvReusedTokens,
        tokensPerSec: res.tokensPerSec,
        strategy: res.strategy,
      });

      const resolvedTitle =
        meetingName.trim() ||
        `${template.label} ${new Date().toLocaleDateString("sv-SE")}`;
      const historyId = `${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;
      const entry: HistoryEntry = {
        id: historyId,
        createdAt: new Date().toISOString(),
        meetingName: resolvedTitle,
        templateId: template.id,
        templateLabel: template.label,
        modelLabel: res.modelLabel,
        markdown: res.markdown,
        context: contextText.trim() || null,
        durationMs: res.durationMs,
      };
      const updatedHistory = await invoke<HistoryEntry[]>(
        "save_history_entry",
        { entry }
      );
      setHistory(updatedHistory);
      setCurrentHistoryId(historyId);

      // Uppdatera modellöversikten så att "loaded" speglas i headern
      const latestModels = await invoke<ModelsOverview>("get_models_status");
      setModelsOverview(latestModels);
      notifyStatus(`${template.label} är klar.`);
    } catch (err) {
      const msg = String(err);
      if (msg.includes("AVBRUTEN")) {
        notifyStatus("Sammanfattningen avbröts.");
      } else {
        setErrorMessage(msg);
      }
    } finally {
      setIsRunning(false);
      setRunProgress(null);
    }
  }

  async function handleStopRunning() {
    await invoke("cancel_summarize");
  }

  // ── Iterativ "Justera resultatet" med Ångra ──
  async function handleAdjustResult(e: React.FormEvent) {
    e.preventDefault();
    if (!adjustDirective.trim() || !resultMarkdown.trim() || isRunning) return;

    const previousDoc = resultMarkdown;
    const directive = adjustDirective.trim();
    setUndoStack((prev) => [...prev.slice(-9), previousDoc]);
    setAdjustDirective("");
    setErrorMessage(null);
    setIsEditingResult(false);
    setIsRunning(true);
    setRunProgress({
      phase: "prefill",
      label: "Tillämpar dina justeringar på dokumentet…",
      prefillPercent: 0,
      kvReusedTokens: 0,
      promptTokens: 0,
      thinkingTokens: 0,
      generatedTokens: 0,
      tokensPerSec: 0,
      chunkCurrent: null,
      chunkTotal: null,
    });

    try {
      const res = await invoke<SummarizeResult>("adjust_meeting_result", {
        currentMarkdown: previousDoc,
        directive,
        modelId: modelsOverview?.activeModelId,
      });
      setResultMarkdown(res.markdown);
      await persistHistoryMarkdownUpdate(res.markdown);
      notifyStatus("Dokumentet har justerats.");
    } catch (err) {
      const msg = String(err);
      // Återställ dokumentet om körningen avbröts eller misslyckades
      setResultMarkdown(previousDoc);
      setUndoStack((prev) => prev.slice(0, -1));
      if (msg.includes("AVBRUTEN")) {
        notifyStatus("Justeringen avbröts och föregående text behölls.");
      } else {
        setErrorMessage(`Justeringen misslyckades: ${msg}`);
      }
    } finally {
      setIsRunning(false);
      setRunProgress(null);
    }
  }

  function handleUndoResult() {
    if (undoStack.length === 0 || isRunning) return;
    const prev = undoStack[undoStack.length - 1];
    setUndoStack((s) => s.slice(0, -1));
    setResultMarkdown(prev);
    void persistHistoryMarkdownUpdate(prev);
    notifyStatus("Ångrade senaste ändringen.");
  }

  // ── Kopiera & Exportera (.docx, .md, .txt) ──
  async function handleCopyRichForWord() {
    if (!resultMarkdown.trim()) return;
    try {
      const htmlFragment = markdownToCleanHtml(
        resultMarkdown,
        meetingName.trim() || undefined
      );
      const fullHtml = `<!DOCTYPE html><html lang="sv"><head><meta charset="utf-8"></head><body>${htmlFragment}</body></html>`;
      const plain = markdownToCleanPlainText(resultMarkdown);

      if (typeof ClipboardItem !== "undefined" && navigator.clipboard?.write) {
        const item = new ClipboardItem({
          "text/html": new Blob([fullHtml], { type: "text/html" }),
          "text/plain": new Blob([plain], { type: "text/plain" }),
        });
        await navigator.clipboard.write([item]);
      } else {
        await navigator.clipboard.writeText(plain);
      }
      notifyStatus("Kopierad med rubriker och listor för Word / Outlook!");
    } catch {
      await navigator.clipboard.writeText(
        markdownToCleanPlainText(resultMarkdown)
      );
      notifyStatus("Kopierad till urklipp.");
    }
  }

  async function handleCopyPlainText() {
    if (!resultMarkdown.trim()) return;
    const plain = markdownToCleanPlainText(resultMarkdown);
    await navigator.clipboard.writeText(plain);
    notifyStatus("Ren text kopierad till urklipp.");
  }

  async function handleExportDocx() {
    if (!resultMarkdown.trim()) return;
    try {
      const baseName = sanitizeFilename(
        meetingName.trim() || selectedTemplate.label
      );
      const filePath = await save({
        title: "Exportera mötesanteckning som Word (.docx)",
        defaultPath: `${baseName}.docx`,
        filters: [{ name: "Word-dokument (.docx)", extensions: ["docx"] }],
      });
      if (!filePath) return;

      const dateStr = new Date().toLocaleDateString("sv-SE");
      const subtitle = `${lastResultMeta?.templateLabel ?? selectedTemplate.label} · ${dateStr}`;
      await invoke("export_docx_file", {
        path: filePath,
        title: meetingName.trim() || selectedTemplate.label,
        subtitle,
        markdown: resultMarkdown,
      });
      notifyStatus("Sparad som Word-dokument (.docx)!");
    } catch (err) {
      setErrorMessage(`Kunde inte exportera Word-fil: ${String(err)}`);
    }
  }

  async function handleExportTextOrMd() {
    if (!resultMarkdown.trim()) return;
    try {
      const baseName = sanitizeFilename(
        meetingName.trim() || selectedTemplate.label
      );
      const filePath = await save({
        title: "Spara som Markdown (.md) eller Text (.txt)",
        defaultPath: `${baseName}.md`,
        filters: [
          { name: "Markdown (.md)", extensions: ["md"] },
          { name: "Textfil (.txt)", extensions: ["txt"] },
        ],
      });
      if (!filePath) return;

      const isTxt = filePath.toLowerCase().endsWith(".txt");
      const payload = isTxt
        ? markdownToCleanPlainText(resultMarkdown)
        : meetingName.trim()
          ? `# ${meetingName.trim()}\n\n${resultMarkdown}`
          : resultMarkdown;

      await invoke("write_text_file", { path: filePath, content: payload });
      notifyStatus(isTxt ? "Sparad som .txt!" : "Sparad som .md!");
    } catch (err) {
      setErrorMessage(`Kunde inte spara filen: ${String(err)}`);
    }
  }

  // ── Sparade Mötessammanhang ──
  async function handleSaveCurrentContext() {
    const trimmed = contextText.trim();
    if (!trimmed) return;
    const shortLabel =
      trimmed.length > 28 ? `${trimmed.slice(0, 26).trim()}…` : trimmed;
    const preset: SavedContextPreset = {
      id: `ctx-${Date.now()}`,
      label: shortLabel,
      text: trimmed,
    };
    const next = [
      preset,
      ...savedContexts.filter((c) => c.text !== trimmed),
    ].slice(0, 12);
    setSavedContexts(next);
    await invoke("save_context_presets", { contexts: next });
    notifyStatus("Mötessammanhang sparat som snabbval.");
  }

  async function handleDeleteContextPreset(id: string) {
    const next = savedContexts.filter((c) => c.id !== id);
    setSavedContexts(next);
    await invoke("save_context_presets", { contexts: next });
  }

  // ── Egna Mallar (<dialog>) ──
  function openNewTemplateDialog(duplicateFrom?: SummaryTemplate) {
    setEditingTemplateId(null);
    setTplLabel(duplicateFrom ? `${duplicateFrom.label} (kopia)` : "");
    setTplDesc(duplicateFrom?.description ?? "");
    setTplPrompt(duplicateFrom?.systemPrompt ?? "");
    setTplTemp(
      duplicateFrom?.sampling?.temperature !== undefined
        ? String(duplicateFrom.sampling.temperature)
        : ""
    );
    setTplRepeat(
      duplicateFrom?.sampling?.repeatPenalty !== undefined
        ? String(duplicateFrom.sampling.repeatPenalty)
        : ""
    );
    templateDialogRef.current?.showModal();
  }

  function openEditTemplateDialog(tpl: SummaryTemplate) {
    setEditingTemplateId(tpl.id);
    setTplLabel(tpl.label);
    setTplDesc(tpl.description);
    setTplPrompt(tpl.systemPrompt);
    setTplTemp(
      tpl.sampling?.temperature !== undefined
        ? String(tpl.sampling.temperature)
        : ""
    );
    setTplRepeat(
      tpl.sampling?.repeatPenalty !== undefined
        ? String(tpl.sampling.repeatPenalty)
        : ""
    );
    templateDialogRef.current?.showModal();
  }

  async function handleSaveCustomTemplate(e: React.FormEvent) {
    e.preventDefault();
    if (!tplPrompt.trim()) return;

    const parseOptNum = (raw: string, min: number, max: number) => {
      const n = Number.parseFloat(raw.replace(",", "."));
      if (!Number.isFinite(n)) return undefined;
      return Math.min(Math.max(n, min), max);
    };

    const tempVal = parseOptNum(tplTemp, 0, 1.5);
    const repVal = parseOptNum(tplRepeat, 0.8, 1.8);
    const sampling: TemplateSampling | undefined =
      tempVal !== undefined || repVal !== undefined
        ? { temperature: tempVal, repeatPenalty: repVal }
        : undefined;

    const id = editingTemplateId ?? `custom-${Date.now()}`;
    const newEntry: SummaryTemplate = {
      id,
      label: tplLabel.trim() || "Egen mall",
      description: tplDesc.trim() || "Anpassad mötesmall",
      systemPrompt: tplPrompt.trim(),
      builtin: false,
      iconName: "sparkles",
      sampling,
    };

    const next = editingTemplateId
      ? customTemplates.map((t) => (t.id === editingTemplateId ? newEntry : t))
      : [...customTemplates, newEntry];

    setCustomTemplates(next);
    setSelectedTemplateId(id);
    await invoke("save_custom_templates", { templates: next });
    templateDialogRef.current?.close();
    notifyStatus("Egen mall sparad lokalt.");
  }

  async function handleDeleteCustomTemplate() {
    if (!editingTemplateId) return;
    const next = customTemplates.filter((t) => t.id !== editingTemplateId);
    setCustomTemplates(next);
    if (selectedTemplateId === editingTemplateId) {
      setSelectedTemplateId(DEFAULT_TEMPLATE_ID);
    }
    await invoke("save_custom_templates", { templates: next });
    templateDialogRef.current?.close();
    notifyStatus("Mallen raderades.");
  }

  async function handleExportTemplatesJson() {
    if (customTemplates.length === 0) {
      notifyStatus("Det finns inga egna mallar att exportera ännu.");
      return;
    }
    const filePath = await save({
      title: "Exportera egna mallar (.json)",
      defaultPath: "loki-mallar.json",
      filters: [{ name: "JSON-fil", extensions: ["json"] }],
    });
    if (!filePath) return;
    await invoke("write_text_file", {
      path: filePath,
      content: JSON.stringify(customTemplates, null, 2),
    });
    notifyStatus("Egna mallar exporterade till JSON.");
  }

  async function handleTemplatePromptFilePicked(
    e: React.ChangeEvent<HTMLInputElement>
  ) {
    const file = e.target.files?.[0];
    e.target.value = "";
    if (!file) return;
    try {
      const ab = await file.arrayBuffer();
      const imported = await invoke<ImportedDocument>("import_document_bytes", {
        fileName: file.name,
        bytes: Array.from(new Uint8Array(ab)),
        cleanAsTranscript: false,
      });

      // Om det är en exporterad JSON-malllista, importera mallarna direkt
      if (file.name.toLowerCase().endsWith(".json")) {
        try {
          const parsed = JSON.parse(imported.text);
          const arr = Array.isArray(parsed) ? parsed : [parsed];
          const valid: SummaryTemplate[] = arr
            .filter(
              (item) =>
                item &&
                typeof item.systemPrompt === "string" &&
                item.systemPrompt.trim()
            )
            .map((item, idx) => ({
              id: `custom-${Date.now()}-${idx}`,
              label: String(item.label || "Importerad mall"),
              description: String(item.description || "Importerad mötesmall"),
              systemPrompt: String(item.systemPrompt),
              builtin: false,
              iconName: "sparkles" as const,
              sampling: item.sampling,
            }));
          if (valid.length > 0) {
            const merged = [...customTemplates, ...valid];
            setCustomTemplates(merged);
            setSelectedTemplateId(valid[0].id);
            await invoke("save_custom_templates", { templates: merged });
            templateDialogRef.current?.close();
            notifyStatus(`Importerade ${valid.length} mall(ar) från JSON.`);
            return;
          }
        } catch {
          // Föll tillbaka till vanlig textinstruktion
        }
      }

      setTplPrompt(imported.text);
      if (!tplLabel.trim()) {
        setTplLabel(file.name.replace(/\.[^.]+$/, ""));
      }
      notifyStatus("Mallinstruktion inläst från fil.");
    } catch (err) {
      setErrorMessage(`Kunde inte läsa mallfilen: ${String(err)}`);
    }
  }

  // ── Historikhantering ──
  function handleOpenHistoryEntry(entry: HistoryEntry) {
    setMeetingName(entry.meetingName);
    setResultMarkdown(entry.markdown);
    setSelectedTemplateId(entry.templateId);
    if (entry.context) setContextText(entry.context);
    setCurrentHistoryId(entry.id);
    setUndoStack([]);
    setIsEditingResult(false);
    setLastResultMeta({
      templateLabel: entry.templateLabel,
      modelLabel: entry.modelLabel,
      durationMs: entry.durationMs,
      kvReusedTokens: 0,
      tokensPerSec: 0,
      strategy: "single",
    });
    historyDialogRef.current?.close();
  }

  async function handleDeleteHistoryEntry(id: string) {
    const next = await invoke<HistoryEntry[]>("delete_history_entry", { id });
    setHistory(next);
    if (currentHistoryId === id) setCurrentHistoryId(null);
  }

  async function handleClearAllHistory() {
    await invoke("clear_all_history");
    setHistory([]);
    setCurrentHistoryId(null);
    notifyStatus("Möteshistoriken har rensats.");
  }

  const filteredHistory = useMemo(() => {
    const q = historyQuery.trim().toLowerCase();
    if (!q) return history;
    return history.filter(
      (h) =>
        h.meetingName.toLowerCase().includes(q) ||
        h.templateLabel.toLowerCase().includes(q) ||
        h.markdown.toLowerCase().includes(q)
    );
  }, [history, historyQuery]);

  const estimatedStrategy = useMemo(() => {
    if (!transcriptStats || !modelsOverview) return "single";
    const margin = thoroughMode ? 8192 : 4096;
    return transcriptStats.estimatedTokens + margin + 900 <=
      modelsOverview.safeCtxTokens
      ? "single"
      : "chunked";
  }, [transcriptStats, modelsOverview, thoroughMode]);

  const dlPercent =
    downloadProgress && downloadProgress.totalBytes > 0
      ? Math.min(
          100,
          Math.round(
            (downloadProgress.downloadedBytes / downloadProgress.totalBytes) *
              100
          )
        )
      : undefined;

  return (
    <div className="loki-root">
      {/* Polite skärmläsarregion utanför verktygsrader (MWG toolbar & accessibility) */}
      <div role="status" aria-live="polite" className="visually-hidden">
        {statusAnnouncement}
      </div>

      {toastText && (
        <div className="toast-pill" aria-hidden="true">
          <Check
            size={14}
            style={{ verticalAlign: "-2px", marginRight: "0.35rem" }}
          />
          {toastText}
        </div>
      )}

      {/* ── 1. Kompakt Sidhuvud ── */}
      <header className="top-header">
        <div className="header-brand">
          <img
            src="/loki-icon.png"
            alt=""
            aria-hidden="true"
            className="brand-icon-img"
          />
          <div className="brand-titles">
            <h1 className="brand-title">Loki</h1>
            <span className="brand-tagline">
              Lokala mötesanteckningar · 100 % på din dator
            </span>
          </div>
        </div>

        <div className="header-controls">
          <button
            type="button"
            className={`header-pill-btn ${
              modelsOverview?.anyInstalled ? "model-ready" : "model-missing"
            }`}
            onClick={() => modelDialogRef.current?.showModal()}
          >
            <Cpu size={14} aria-hidden="true" />
            <span>
              {modelsOverview?.anyInstalled && activeModelEntry
                ? `Modell: ${activeModelEntry.shortLabel}${
                    activeModelEntry.loaded ? " (redo)" : ""
                  }`
                : "Välj / Hämta språkmodell"}
            </span>
          </button>

          <button
            type="button"
            className="header-pill-btn"
            onClick={() => historyDialogRef.current?.showModal()}
          >
            <History size={14} aria-hidden="true" />
            <span>Tidigare möten</span>
            {history.length > 0 && (
              <span className="pill-badge-count">{history.length}</span>
            )}
          </button>

          <button
            type="button"
            className="header-pill-btn"
            onClick={async () => {
              try {
                const info = await invoke<{
                  path: string;
                  content: string;
                  accelerator: string;
                }>("get_log_info");
                setLogInfo(info);
              } catch (err) {
                setLogInfo({
                  path: "(kunde inte läsa)",
                  content: String(err),
                  accelerator: "",
                });
              }
              logDialogRef.current?.showModal();
            }}
            title="Visa lokal felsökningslogg"
          >
            <ScrollText size={14} aria-hidden="true" />
            <span>Logg</span>
          </button>

          <button
            type="button"
            className="header-pill-btn"
            onClick={cycleTheme}
            aria-label={`Färgtema: ${
              theme === "system"
                ? "Systemstandard"
                : theme === "light"
                  ? "Ljust"
                  : "Mörkt"
            }. Klicka för att växla tema.`}
          >
            {theme === "system" ? (
              <Monitor size={14} aria-hidden="true" />
            ) : theme === "light" ? (
              <Sun size={14} aria-hidden="true" />
            ) : (
              <Moon size={14} aria-hidden="true" />
            )}
            <span>
              {theme === "system"
                ? "System"
                : theme === "light"
                  ? "Ljust"
                  : "Mörkt"}
            </span>
          </button>
        </div>
      </header>

      {/* ── Felbanner ── */}
      {errorMessage && (
        <div className="error-banner" role="alert">
          <span>{errorMessage}</span>
          <button
            type="button"
            className="tb-btn"
            onClick={() => setErrorMessage(null)}
          >
            Stäng
          </button>
        </div>
      )}

      {/* ── Första-starten-kort om ingen lokal GGUF-modell hittas ── */}
      {modelsOverview && !modelsOverview.anyInstalled && (
        <section className="setup-banner" aria-labelledby="setup-heading">
          <div className="setup-banner-top">
            <div className="setup-banner-text">
              <h2 id="setup-heading">
                Aktivera lokal språkmodell (engångsinstallation)
              </h2>
              <p>
                Loki skickar aldrig dina mötestexter till molnet. Hämta den
                optimerade modellen <strong>Gemma 4 E2B QAT (2,62 GB)</strong>{" "}
                en gång, eller peka ut en mapp om du redan har filen på datorn.
              </p>
            </div>
            <div className="setup-banner-actions">
              {downloadingModelId ? (
                <button
                  type="button"
                  className="tb-btn"
                  onClick={handleCancelDownload}
                >
                  <Square size={14} aria-hidden="true" />
                  Avbryt nedladdning
                </button>
              ) : (
                <>
                  <button
                    type="button"
                    className="tb-btn-copy"
                    onClick={() => handleDownloadModel("e2b")}
                  >
                    <Download size={15} aria-hidden="true" />
                    Hämta Gemma 4 E2B (2,62 GB)
                  </button>
                  <button
                    type="button"
                    className="tb-btn"
                    onClick={() => handleImportModelOffline(false)}
                  >
                    <FolderOpen size={15} aria-hidden="true" />
                    Välj GGUF-fil offline…
                  </button>
                </>
              )}
            </div>
          </div>

          {downloadingModelId && downloadProgress && (
            <div className="progress-card">
              <div className="progress-card-row">
                <span id="setup-dl-label">{downloadProgress.label}</span>
                <span className="progress-meta">
                  {downloadProgress.phase === "downloading"
                    ? `${formatBytes(downloadProgress.downloadedBytes)} / ${formatBytes(downloadProgress.totalBytes)}`
                    : "Initierar…"}
                </span>
              </div>
              <progress
                className="loki-progress"
                aria-labelledby="setup-dl-label"
                value={
                  downloadProgress.phase === "downloading"
                    ? dlPercent
                    : undefined
                }
                max={100}
              />
            </div>
          )}
        </section>
      )}

      {/* ── 2. Huvudarbetsyta ── */}
      <main
        className={`loki-main ${focusResultView ? "focus-result" : ""}`}
      >
        {/* VÄNSTER KOLUMN: Mötesunderlag, Mall & Inställningar */}
        {!focusResultView && (
          <div className="panel-card input-column">
            {/* Mötesnamn */}
            <div className="field-group">
              <label htmlFor="meeting-name-input" className="field-label">
                Mötesnamn / Rubrik
              </label>
              <input
                id="meeting-name-input"
                type="text"
                className="text-input"
                value={meetingName}
                onChange={(e) => setMeetingName(e.target.value)}
                placeholder="t.ex. Styrgruppsmöte Digitalisering 2026-11-12"
              />
            </div>

            {/* Mötesunderlag / Transkribering */}
            <div className="field-group">
              <div className="field-label-row">
                <label htmlFor="transcript-textarea" className="field-label">
                  Mötesunderlag / Transkribering
                </label>
                <span className="field-hint">
                  Word, Excel, PDF, VTT, SRT, ODT, TXT eller klistra in
                </span>
              </div>

              <div
                className={`transcript-box ${isDraggingFile ? "dragging" : ""}`}
                onDragOver={(e) => {
                  e.preventDefault();
                  setIsDraggingFile(true);
                }}
                onDragLeave={() => setIsDraggingFile(false)}
                onDrop={(e) => {
                  e.preventDefault();
                  setIsDraggingFile(false);
                  const file = e.dataTransfer.files?.[0];
                  if (file) {
                    void ingestBrowserFile(file, "transcript");
                  }
                }}
              >
                <div className="transcript-toolbar">
                  <button
                    type="button"
                    className="tb-btn"
                    onClick={() => transcriptFileInputRef.current?.click()}
                  >
                    <Upload size={14} aria-hidden="true" />
                    Öppna fil…
                  </button>
                  <button
                    type="button"
                    className="tb-btn"
                    onClick={handlePasteTranscriptFromClipboard}
                  >
                    <ClipboardPaste size={14} aria-hidden="true" />
                    Klistra in
                  </button>
                  {transcript && (
                    <button
                      type="button"
                      className="tb-btn"
                      onClick={() => {
                        setTranscript("");
                        setTranscriptSourceSummary(null);
                        setTranscriptStats(null);
                      }}
                    >
                      <Trash2 size={14} aria-hidden="true" />
                      Rensa
                    </button>
                  )}
                </div>

                <textarea
                  id="transcript-textarea"
                  className="transcript-textarea"
                  value={transcript}
                  onChange={(e) => setTranscript(e.target.value)}
                  onPaste={handleTranscriptTextPaste}
                  placeholder="Dra in en fil (.vtt, .srt, .docx, .xlsx, .pdf, .odt, .txt) eller klistra in mötestranskriberingen här. Tidsstämplar och talarupprepningar städas automatiskt…"
                />

                {transcriptStats && transcriptStats.wordCount > 0 && (
                  <div className="stats-strip">
                    {transcriptSourceSummary && (
                      <span
                        className={`stat-chip ${
                          transcriptStats.wasCleaned ? "cleaned" : ""
                        }`}
                      >
                        {transcriptSourceSummary}
                      </span>
                    )}
                    <span className="stat-chip">
                      {transcriptStats.wordCount.toLocaleString("sv-SE")} ord ·
                      ~{transcriptStats.approxMinutes} min tal
                    </span>
                    <span className="stat-chip">
                      ~{transcriptStats.estimatedTokens.toLocaleString("sv-SE")}{" "}
                      tokens
                    </span>
                    <span
                      className={`stat-chip ${
                        estimatedStrategy === "chunked" ? "chunked" : ""
                      }`}
                    >
                      {estimatedStrategy === "single"
                        ? "Ett sammanhängande pass"
                        : "Automatisk uppdelning"}
                    </span>
                  </div>
                )}
              </div>
            </div>

            {/* Mallväljare */}
            <div className="field-group">
              <div className="field-label-row">
                <span className="field-label">Välj dokumentmall</span>
                <div style={{ display: "flex", gap: "0.35rem" }}>
                  <button
                    type="button"
                    className="tb-btn"
                    onClick={() => openNewTemplateDialog()}
                  >
                    <Plus size={13} aria-hidden="true" />
                    Ny mall
                  </button>
                  {customTemplates.length > 0 && (
                    <button
                      type="button"
                      className="tb-btn"
                      onClick={handleExportTemplatesJson}
                    >
                      Exportera mallar
                    </button>
                  )}
                </div>
              </div>

              <div
                className="template-grid"
                role="group"
                aria-label="Dokumentmallar"
              >
                {allTemplates.map((tpl) => (
                  <div key={tpl.id} className="template-card-wrap">
                    <button
                      type="button"
                      className="template-card-btn"
                      aria-pressed={selectedTemplateId === tpl.id}
                      onClick={() => setSelectedTemplateId(tpl.id)}
                    >
                      <span className="tpl-title-row">
                        {renderTemplateIcon(tpl.iconName)}
                        <span>{tpl.label}</span>
                      </span>
                      <span className="tpl-desc">{tpl.description}</span>
                    </button>
                    {tpl.builtin ? (
                      <button
                        type="button"
                        className="tpl-card-action"
                        aria-label={`Duplicera mallen ${tpl.label} som egen mall`}
                        title="Skapa egen kopia av mallen"
                        onClick={() => openNewTemplateDialog(tpl)}
                      >
                        <Files size={13} aria-hidden="true" />
                      </button>
                    ) : (
                      <button
                        type="button"
                        className="tpl-card-action"
                        aria-label={`Redigera mallen ${tpl.label}`}
                        title="Redigera egen mall"
                        onClick={() => openEditTemplateDialog(tpl)}
                      >
                        <Pencil size={13} aria-hidden="true" />
                      </button>
                    )}
                  </div>
                ))}
              </div>
            </div>

            {/* Mötessammanhang (för fonetisk fackordstolkning) */}
            <div className="field-group">
              <div className="field-label-row">
                <label htmlFor="context-input" className="field-label">
                  Mötets sammanhang / Verksamhet (valfritt)
                </label>
                {contextText.trim() && (
                  <button
                    type="button"
                    className="tb-btn"
                    onClick={handleSaveCurrentContext}
                  >
                    Spara som snabbval
                  </button>
                )}
              </div>
              <input
                id="context-input"
                type="text"
                className="text-input"
                value={contextText}
                onChange={(e) => setContextText(e.target.value)}
                placeholder="t.ex. Regionens IT-enhet, införande av Cosmic och SITHS-kort"
              />
              {savedContexts.length > 0 && (
                <div className="context-chips-row">
                  {savedContexts.map((preset) => (
                    <span key={preset.id} className="context-chip-group">
                      <button
                        type="button"
                        className="context-chip-btn"
                        onClick={() => setContextText(preset.text)}
                      >
                        {preset.label}
                      </button>
                      <button
                        type="button"
                        className="context-chip-del"
                        aria-label={`Ta bort sparat sammanhang ${preset.label}`}
                        onClick={() => handleDeleteContextPreset(preset.id)}
                      >
                        <X size={12} aria-hidden="true" />
                      </button>
                    </span>
                  ))}
                </div>
              )}
            </div>

            {/* Tillval: Agenda, Kreativare tolkning & Noggrannare */}
            <div className="options-row">
              <button
                type="button"
                className="toggle-chip-btn"
                aria-pressed={agendaOpen || Boolean(agendaText.trim())}
                onClick={() => setAgendaOpen((v) => !v)}
              >
                <ListChecks size={14} aria-hidden="true" />
                <span>
                  {agendaText.trim()
                    ? `Agenda tillagd${agendaFileName ? ` (${agendaFileName})` : ""}`
                    : "+ Lägg till agenda"}
                </span>
              </button>

              <button
                type="button"
                className="toggle-chip-btn"
                aria-pressed={creativeMode}
                onClick={() => setCreativeMode((v) => !v)}
                title="Låter mötessammanhanget forma ton och struktur med mer språkligt flyt (inga påhittade fakta)"
              >
                <Sparkles size={14} aria-hidden="true" />
                <span>Kreativare språk</span>
              </button>

              <button
                type="button"
                className="toggle-chip-btn"
                aria-pressed={thoroughMode}
                onClick={() => setThoroughMode((v) => !v)}
                title="Låter modellen resonera igenom underlaget i ett extra tankesteg före utskrift"
              >
                <Brain size={14} aria-hidden="true" />
                <span>Noggrannare</span>
              </button>
            </div>

            {agendaOpen && (
              <div className="agenda-drawer">
                <div className="field-label-row">
                  <label htmlFor="agenda-textarea" className="field-label">
                    Agenda (strukturerar rubrikerna efter mötets punkter)
                  </label>
                  <div style={{ display: "flex", gap: "0.35rem" }}>
                    <button
                      type="button"
                      className="tb-btn"
                      onClick={() => agendaFileInputRef.current?.click()}
                    >
                      <Upload size={13} aria-hidden="true" />
                      Från fil…
                    </button>
                    {agendaText && (
                      <button
                        type="button"
                        className="tb-btn"
                        onClick={() => {
                          setAgendaText("");
                          setAgendaFileName(null);
                        }}
                      >
                        Rensa
                      </button>
                    )}
                  </div>
                </div>
                <textarea
                  id="agenda-textarea"
                  className="textarea-input"
                  rows={3}
                  value={agendaText}
                  onChange={(e) => setAgendaText(e.target.value)}
                  placeholder="1. Mötets öppnande&#10;2. Budgetuppföljning Q3&#10;3. Beslut om lanseringsdatum…"
                />
              </div>
            )}

            {/* Huvudknapp: Skapa anteckning / Avbryt */}
            {isRunning ? (
              <button
                type="button"
                className="btn-danger-cta"
                onClick={handleStopRunning}
              >
                <Square size={16} aria-hidden="true" />
                Avbryt körning
              </button>
            ) : (
              <button
                type="button"
                className="btn-primary-cta"
                disabled={!transcript.trim() || !modelsOverview?.anyInstalled}
                onClick={() => runSummarizeWithTemplate(selectedTemplate)}
              >
                <Play size={16} aria-hidden="true" />
                Skapa {selectedTemplate.label.toLowerCase()}
              </button>
            )}
          </div>
        )}

        {/* HÖGER KOLUMN: Färdigt dokument, 1-klicks mallbyte, Export & Justering */}
        <div className="panel-card result-column">
          {/* Verktygsrad enligt MWG toolbar-guiden */}
          <div
            role="toolbar"
            aria-label="Dokumentverktyg och export"
            className="action-toolbar"
            {...({
              focusgroup: "toolbar wrap",
            } as React.HTMLAttributes<HTMLDivElement>)}
          >
            <div
              className="toolbar-group"
              role="group"
              aria-label="Visning och redigering"
            >
              <button
                type="button"
                className="tb-btn"
                aria-pressed={focusResultView}
                onClick={() => setFocusResultView((v) => !v)}
              >
                {focusResultView ? (
                  <PanelLeftOpen size={14} aria-hidden="true" />
                ) : (
                  <PanelLeftClose size={14} aria-hidden="true" />
                )}
                <span>
                  {focusResultView ? "Visa underlag" : "Fokusläge"}
                </span>
              </button>

              <button
                type="button"
                className="tb-btn"
                disabled={!resultMarkdown.trim() || isRunning}
                aria-pressed={isEditingResult}
                onClick={() => setIsEditingResult((v) => !v)}
              >
                {isEditingResult ? (
                  <Eye size={14} aria-hidden="true" />
                ) : (
                  <Pencil size={14} aria-hidden="true" />
                )}
                <span>
                  {isEditingResult ? "Förhandsgranska" : "Redigera text"}
                </span>
              </button>
            </div>

            <span className="tb-sep" aria-hidden="true" />

            <div
              className="toolbar-group toolbar-group-end"
              role="group"
              aria-label="Kopiera och spara"
            >
              <button
                type="button"
                className="tb-btn-copy"
                disabled={!resultMarkdown.trim() || isRunning}
                onClick={handleCopyRichForWord}
              >
                <Copy size={14} aria-hidden="true" />
                <span>Kopiera för Word / Outlook</span>
              </button>

              <button
                type="button"
                className="tb-btn"
                disabled={!resultMarkdown.trim() || isRunning}
                onClick={handleCopyPlainText}
              >
                <span>Ren text</span>
              </button>

              <button
                type="button"
                className="tb-btn"
                disabled={!resultMarkdown.trim() || isRunning}
                onClick={handleExportDocx}
              >
                <FileDown size={14} aria-hidden="true" />
                <span>Word (.docx)</span>
              </button>

              <button
                type="button"
                className="tb-btn"
                disabled={!resultMarkdown.trim() || isRunning}
                onClick={handleExportTextOrMd}
              >
                <FileText size={14} aria-hidden="true" />
                <span>.md / .txt</span>
              </button>
            </div>
          </div>

          {/* 1-klicks mallbyte på Varm KV-cache när underlag finns */}
          {transcript.trim() && (resultMarkdown.trim() || isRunning) && (
            <div
              className="quick-template-tabs"
              role="group"
              aria-label="Snabbväxla mall på samma möte"
            >
              {allTemplates.map((tpl) => (
                <button
                  key={`qtab-${tpl.id}`}
                  type="button"
                  className="quick-tab-btn"
                  disabled={isRunning}
                  aria-pressed={selectedTemplateId === tpl.id}
                  onClick={() => runSummarizeWithTemplate(tpl)}
                  title={`Skapa ${tpl.label} direkt ur varm mötescache`}
                >
                  {renderTemplateIcon(tpl.iconName)}
                  <span>{tpl.label}</span>
                </button>
              ))}
            </div>
          )}

          {/* Live Förloppsindikator (Native <progress>) */}
          {isRunning && runProgress && (
            <div className="progress-card">
              <div className="progress-card-row">
                <span id="summarize-progress-label">{runProgress.label}</span>
                <span className="progress-meta">
                  {runProgress.phase === "generating" ||
                  runProgress.phase === "thinking"
                    ? `${runProgress.generatedTokens || runProgress.thinkingTokens} tokens · ${runProgress.tokensPerSec.toFixed(1)} tok/s`
                    : runProgress.phase === "prefill"
                      ? `${runProgress.prefillPercent} %`
                      : ""}
                </span>
              </div>
              <progress
                className="loki-progress"
                aria-labelledby="summarize-progress-label"
                value={
                  runProgress.phase === "prefill"
                    ? runProgress.prefillPercent
                    : undefined
                }
                max={100}
              />
            </div>
          )}

          {/* Dokumentyta (Formaterad läsvy eller Direktredigering) */}
          {isEditingResult ? (
            <textarea
              aria-label="Redigera mötesanteckning"
              className="document-edit-textarea"
              value={resultMarkdown}
              onChange={(e) => {
                const val = e.target.value;
                setResultMarkdown(val);
                void persistHistoryMarkdownUpdate(val);
              }}
            />
          ) : (
            <div className="document-canvas">
              {resultMarkdown.trim() ? (
                <>
                  <div className="doc-header-banner">
                    <h2 className="doc-header-title">
                      {meetingName.trim() ||
                        lastResultMeta?.templateLabel ||
                        selectedTemplate.label}
                    </h2>
                    {lastResultMeta && (
                      <div className="doc-header-meta">
                        <span>{lastResultMeta.templateLabel}</span>
                        <span>·</span>
                        <span>{lastResultMeta.modelLabel}</span>
                        {lastResultMeta.durationMs > 0 && (
                          <>
                            <span>·</span>
                            <span>
                              {(lastResultMeta.durationMs / 1000)
                                .toFixed(1)
                                .replace(".", ",")}{" "}
                              s
                            </span>
                          </>
                        )}
                        {lastResultMeta.kvReusedTokens > 500 && (
                          <span className="kv-fast-badge">
                            Varm cache:{" "}
                            {lastResultMeta.kvReusedTokens.toLocaleString(
                              "sv-SE"
                            )}{" "}
                            tokens återanvända
                          </span>
                        )}
                      </div>
                    )}
                  </div>
                  {renderFormattedMarkdown(resultMarkdown)}
                </>
              ) : (
                <div className="empty-result-placeholder">
                  <NotebookPen size={32} aria-hidden="true" />
                  <h2>Mötesdokumentet visas här</h2>
                  <p>
                    Lägg in en transkribering eller ett dokument till vänster,
                    välj mall (t.ex. <strong>Minnesanteckning</strong> eller{" "}
                    <strong>Mötesprotokoll</strong>) och klicka på{" "}
                    <strong>Skapa</strong>. När mötet är inläst kan du växla
                    mellan mallar på en sekund!
                  </p>
                </div>
              )}
            </div>
          )}

          {/* Iterativ "Justera resultatet" med Ångra */}
          {resultMarkdown.trim() && (
            <form className="adjust-bar" onSubmit={handleAdjustResult}>
              <label htmlFor="adjust-directive-input" className="visually-hidden">
                Justera resultatet med fritextdirektiv
              </label>
              <input
                id="adjust-directive-input"
                type="text"
                className="text-input"
                disabled={isRunning}
                value={adjustDirective}
                onChange={(e) => setAdjustDirective(e.target.value)}
                placeholder="Justera detaljer, t.ex. ”Kulturum ska stavas Qulturum” eller ”Lägg Anna som ansvarig på punkt 2”…"
              />
              <button
                type="submit"
                className="tb-btn"
                disabled={!adjustDirective.trim() || isRunning}
              >
                <Wand2 size={14} aria-hidden="true" />
                <span>Justera</span>
              </button>
              {undoStack.length > 0 && (
                <button
                  type="button"
                  className="tb-btn"
                  disabled={isRunning}
                  onClick={handleUndoResult}
                  title="Ångra senaste omformulering eller mallkörning"
                >
                  <Undo2 size={14} aria-hidden="true" />
                  <span>Ångra ({undoStack.length})</span>
                </button>
              )}
            </form>
          )}
        </div>
      </main>

      {/* ── Dolda filväljare för bred dokumentimport ── */}
      <input
        ref={transcriptFileInputRef}
        type="file"
        hidden
        accept={SUPPORTED_DOCUMENT_EXTENSIONS.map((e) => `.${e}`).join(",")}
        onChange={(e) => {
          const f = e.target.files?.[0];
          e.target.value = "";
          if (f) void ingestBrowserFile(f, "transcript");
        }}
      />

      <input
        ref={agendaFileInputRef}
        type="file"
        hidden
        accept={SUPPORTED_DOCUMENT_EXTENSIONS.map((e) => `.${e}`).join(",")}
        onChange={(e) => {
          const f = e.target.files?.[0];
          e.target.value = "";
          if (f) void ingestBrowserFile(f, "agenda");
        }}
      />

      <input
        ref={templateFileInputRef}
        type="file"
        hidden
        accept=".txt,.md,.docx,.odt,.pdf,.json"
        onChange={handleTemplatePromptFilePicked}
      />

      {/* ── DIALOG 1: Språkmodeller (Gemma 4 E2B & E4B QAT) ── */}
      <dialog
        ref={modelDialogRef}
        className="loki-dialog"
        aria-labelledby="model-dialog-title"
        onClick={(e) => {
          if (e.target === e.currentTarget) e.currentTarget.close();
        }}
      >
        <div className="dialog-header">
          <h2 id="model-dialog-title">Lokala språkmodeller (100 % offline)</h2>
          <button
            type="button"
            className="tb-btn"
            onClick={() => modelDialogRef.current?.close()}
          >
            Stäng
          </button>
        </div>

        <div className="dialog-body">
          {modelsOverview && (
            <p className="field-hint" style={{ margin: 0 }}>
              Systemminne:{" "}
              <strong>{Math.round(modelsOverview.totalRamGb)} GB RAM</strong> ·
              Dynamiskt kontextfönster upp till{" "}
              <strong>
                {modelsOverview.safeCtxTokens.toLocaleString("sv-SE")} tokens
              </strong>{" "}
              i ett sammanhängande pass.
            </p>
          )}

          {downloadingModelId && downloadProgress && (
            <div className="progress-card">
              <div className="progress-card-row">
                <span id="dlg-dl-label">{downloadProgress.label}</span>
                <span className="progress-meta">
                  {downloadProgress.phase === "downloading"
                    ? `${formatBytes(downloadProgress.downloadedBytes)} / ${formatBytes(downloadProgress.totalBytes)}`
                    : "Arbetar…"}
                </span>
              </div>
              <progress
                className="loki-progress"
                aria-labelledby="dlg-dl-label"
                value={
                  downloadProgress.phase === "downloading"
                    ? dlPercent
                    : undefined
                }
                max={100}
              />
            </div>
          )}

          {modelsOverview?.models.map((m) => {
            const isSelected = modelsOverview.activeModelId === m.id;
            return (
              <div
                key={m.id}
                className={`model-option-card ${isSelected && m.installed ? "active-model" : ""}`}
              >
                <div className="model-option-top">
                  <strong>{m.label}</strong>
                  {m.installed ? (
                    <span className="kv-fast-badge">
                      {m.source === "maskera"
                        ? "Delad från Maskera"
                        : m.source === "portable"
                          ? "Portabel modell"
                          : "Installerad"}
                    </span>
                  ) : (
                    <span className="field-hint">Ej nedladdad</span>
                  )}
                </div>
                <p className="field-hint" style={{ margin: 0 }}>
                  {m.description}
                </p>

                <div className="model-option-actions">
                  {m.installed ? (
                    <>
                      <button
                        type="button"
                        className="tb-btn"
                        aria-pressed={isSelected}
                        disabled={Boolean(downloadingModelId)}
                        onClick={() => handleSelectModel(m.id)}
                      >
                        {isSelected ? "Aktiv modell" : "Använd denna modell"}
                      </button>
                      {m.canDelete && (
                        <button
                          type="button"
                          className="tb-btn"
                          disabled={Boolean(downloadingModelId) || isRunning}
                          onClick={() => handleDeleteModel(m.id)}
                        >
                          <Trash2 size={13} aria-hidden="true" />
                          Avinstallera ({formatBytes(m.fileSizeBytes)})
                        </button>
                      )}
                    </>
                  ) : downloadingModelId === m.id ? (
                    <button
                      type="button"
                      className="tb-btn"
                      onClick={handleCancelDownload}
                    >
                      <Square size={13} aria-hidden="true" />
                      Avbryt nedladdning
                    </button>
                  ) : (
                    <button
                      type="button"
                      className="tb-btn-copy"
                      disabled={Boolean(downloadingModelId)}
                      onClick={() => handleDownloadModel(m.id)}
                    >
                      <Download size={14} aria-hidden="true" />
                      Hämta ({formatBytes(m.expectedBytes)})
                    </button>
                  )}
                </div>
              </div>
            );
          })}
        </div>

        <div className="dialog-footer">
          <div style={{ display: "flex", gap: "0.4rem", flexWrap: "wrap" }}>
            <button
              type="button"
              className="tb-btn"
              disabled={Boolean(downloadingModelId)}
              onClick={() => handleImportModelOffline(false)}
            >
              <FolderOpen size={14} aria-hidden="true" />
              Importera GGUF-fil…
            </button>
            <button
              type="button"
              className="tb-btn"
              disabled={Boolean(downloadingModelId)}
              onClick={() => handleImportModelOffline(true)}
            >
              Importera från mapp…
            </button>
          </div>
          <button
            type="button"
            className="tb-btn"
            onClick={() => modelDialogRef.current?.close()}
          >
            Klar
          </button>
        </div>
      </dialog>

      {/* ── DIALOG 2: Lokal Möteshistorik ── */}
      <dialog
        ref={historyDialogRef}
        className="loki-dialog"
        aria-labelledby="history-dialog-title"
        onClick={(e) => {
          if (e.target === e.currentTarget) e.currentTarget.close();
        }}
      >
        <div className="dialog-header">
          <h2 id="history-dialog-title">
            Tidigare mötesanteckningar (endast på denna dator)
          </h2>
          <button
            type="button"
            className="tb-btn"
            onClick={() => historyDialogRef.current?.close()}
          >
            Stäng
          </button>
        </div>

        <div className="dialog-body">
          <div className="field-group">
            <label htmlFor="history-search-input" className="visually-hidden">
              Sök i tidigare mötesanteckningar
            </label>
            <input
              id="history-search-input"
              type="search"
              className="text-input"
              value={historyQuery}
              onChange={(e) => setHistoryQuery(e.target.value)}
              placeholder="Sök på mötesnamn, mall eller innehåll…"
            />
          </div>

          {filteredHistory.length === 0 ? (
            <p className="field-hint">
              {history.length === 0
                ? "Inga sparade mötesanteckningar ännu."
                : "Inga träffar matchade din sökning."}
            </p>
          ) : (
            <div className="history-list">
              {filteredHistory.map((item) => (
                <div key={item.id} className="history-item-row">
                  <button
                    type="button"
                    className="history-item-main"
                    onClick={() => handleOpenHistoryEntry(item)}
                  >
                    <span className="history-item-title">
                      {item.meetingName}
                    </span>
                    <span className="history-item-sub">
                      {item.templateLabel} ·{" "}
                      {new Date(item.createdAt).toLocaleString("sv-SE", {
                        dateStyle: "short",
                        timeStyle: "short",
                      })}{" "}
                      · {item.modelLabel}
                    </span>
                  </button>
                  <button
                    type="button"
                    className="tb-btn"
                    aria-label={`Ta bort ${item.meetingName} ur historiken`}
                    onClick={() => handleDeleteHistoryEntry(item.id)}
                  >
                    <Trash2 size={14} aria-hidden="true" />
                  </button>
                </div>
              ))}
            </div>
          )}
        </div>

        <div className="dialog-footer">
          {history.length > 0 ? (
            <button
              type="button"
              className="tb-btn"
              onClick={handleClearAllHistory}
            >
              <Trash2 size={14} aria-hidden="true" />
              Rensa all historik
            </button>
          ) : (
            <span />
          )}
          <button
            type="button"
            className="tb-btn"
            onClick={() => historyDialogRef.current?.close()}
          >
            Stäng
          </button>
        </div>
      </dialog>

      {/* ── DIALOG 2B: Lokal Felsökningslogg ── */}
      <dialog
        ref={logDialogRef}
        className="loki-dialog"
        aria-labelledby="log-dialog-title"
        onClick={(e) => {
          if (e.target === e.currentTarget) e.currentTarget.close();
        }}
      >
        <div className="dialog-header">
          <h2 id="log-dialog-title">Lokal felsökningslogg</h2>
          <button
            type="button"
            className="icon-close-btn"
            onClick={() => logDialogRef.current?.close()}
            aria-label="Stäng loggfönster"
          >
            <X size={18} aria-hidden="true" />
          </button>
        </div>
        <div className="dialog-body">
          <p className="field-hint" style={{ marginTop: 0 }}>
            Accelerator: <strong>{logInfo?.accelerator || "Okänd"}</strong> · Fil:{" "}
            <code>{logInfo?.path || ""}</code>
          </p>
          <pre
            style={{
              maxHeight: "340px",
              overflow: "auto",
              padding: "0.75rem",
              borderRadius: "8px",
              background: "var(--surface-2)",
              border: "1px solid var(--border)",
              fontSize: "0.78rem",
              lineHeight: 1.45,
              whiteSpace: "pre-wrap",
              wordBreak: "break-word",
            }}
          >
            {logInfo?.content || "Ingen logg ännu."}
          </pre>
        </div>
        <div className="dialog-footer dialog-footer-split">
          <div style={{ display: "flex", gap: "0.5rem" }}>
            <button
              type="button"
              className="tb-btn"
              onClick={async () => {
                const info = await invoke<{
                  path: string;
                  content: string;
                  accelerator: string;
                }>("get_log_info");
                setLogInfo(info);
              }}
            >
              <RefreshCw size={14} aria-hidden="true" />
              Uppdatera
            </button>
            <button
              type="button"
              className="tb-btn"
              onClick={async () => {
                if (logInfo?.content) {
                  await navigator.clipboard.writeText(logInfo.content);
                  notifyStatus("Loggen kopierad till urklipp.");
                }
              }}
            >
              <Copy size={14} aria-hidden="true" />
              Kopiera
            </button>
            <button
              type="button"
              className="tb-btn"
              onClick={async () => {
                await invoke("clear_log_file");
                const info = await invoke<{
                  path: string;
                  content: string;
                  accelerator: string;
                }>("get_log_info");
                setLogInfo(info);
              }}
            >
              <Trash2 size={14} aria-hidden="true" />
              Rensa
            </button>
          </div>
          <button
            type="button"
            className="tb-btn"
            onClick={() => logDialogRef.current?.close()}
          >
            Stäng
          </button>
        </div>
      </dialog>

      {/* ── DIALOG 3: Skapa / Redigera Egen Dokumentmall ── */}
      <dialog
        ref={templateDialogRef}
        className="loki-dialog"
        aria-labelledby="template-dialog-title"
        onClick={(e) => {
          if (e.target === e.currentTarget) e.currentTarget.close();
        }}
      >
        <form onSubmit={handleSaveCustomTemplate}>
          <div className="dialog-header">
            <h2 id="template-dialog-title">
              {editingTemplateId ? "Redigera egen mall" : "Ny egen dokumentmall"}
            </h2>
            <button
              type="button"
              className="tb-btn"
              onClick={() => templateDialogRef.current?.close()}
            >
              Avbryt
            </button>
          </div>

          <div className="dialog-body">
            <div className="field-group">
              <label htmlFor="tpl-name-input" className="field-label">
                Mallens namn
              </label>
              <input
                id="tpl-name-input"
                type="text"
                className="text-input"
                value={tplLabel}
                onChange={(e) => setTplLabel(e.target.value)}
                placeholder="t.ex. Veckorapport till ledningsgrupp"
              />
            </div>

            <div className="field-group">
              <label htmlFor="tpl-desc-input" className="field-label">
                Kort beskrivning
              </label>
              <input
                id="tpl-desc-input"
                type="text"
                className="text-input"
                value={tplDesc}
                onChange={(e) => setTplDesc(e.target.value)}
                placeholder="Visas på mallkortet"
              />
            </div>

            <div className="field-group">
              <div className="field-label-row">
                <label htmlFor="tpl-prompt-input" className="field-label">
                  Instruktion till modellen
                </label>
                <button
                  type="button"
                  className="tb-btn"
                  onClick={() => templateFileInputRef.current?.click()}
                >
                  <Upload size={13} aria-hidden="true" />
                  Läs från fil / JSON…
                </button>
              </div>
              <textarea
                id="tpl-prompt-input"
                className="textarea-input"
                rows={7}
                required
                value={tplPrompt}
                onChange={(e) => setTplPrompt(e.target.value)}
                placeholder="Beskriv vilka rubriker, fokusområden och format mallen ska producera…"
              />
              <span className="field-hint">
                Lokis källtrohets- och kallpratsfilter läggs automatiskt till i
                grunden.
              </span>
            </div>

            <div
              style={{
                display: "grid",
                gridTemplateColumns: "1fr 1fr",
                gap: "0.6rem",
              }}
            >
              <div className="field-group">
                <label htmlFor="tpl-temp-input" className="field-label">
                  Temperatur (valfri)
                </label>
                <input
                  id="tpl-temp-input"
                  type="text"
                  className="text-input"
                  value={tplTemp}
                  onChange={(e) => setTplTemp(e.target.value)}
                  placeholder="0.05 (standard)"
                />
              </div>
              <div className="field-group">
                <label htmlFor="tpl-rep-input" className="field-label">
                  Repetitionsstraff (valfri)
                </label>
                <input
                  id="tpl-rep-input"
                  type="text"
                  className="text-input"
                  value={tplRepeat}
                  onChange={(e) => setTplRepeat(e.target.value)}
                  placeholder="1.10 (standard)"
                />
              </div>
            </div>
          </div>

          <div className="dialog-footer">
            {editingTemplateId ? (
              <button
                type="button"
                className="tb-btn"
                onClick={handleDeleteCustomTemplate}
              >
                <Trash2 size={14} aria-hidden="true" />
                Radera mall
              </button>
            ) : (
              <span />
            )}
            <div style={{ display: "flex", gap: "0.4rem" }}>
              <button
                type="button"
                className="tb-btn"
                onClick={() => templateDialogRef.current?.close()}
              >
                Avbryt
              </button>
              <button type="submit" className="tb-btn-copy">
                Spara mall
              </button>
            </div>
          </div>
        </form>
      </dialog>
    </div>
  );
}

