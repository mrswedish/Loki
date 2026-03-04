import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import './styles/theme.css';
import { Sidebar } from './components/Sidebar';
import { ChatArea } from './components/ChatArea';
import { InputBar } from './components/InputBar';
import { SettingsModal } from './components/SettingsModal';
import { TerminalHeader } from './components/TerminalHeader';
import { useSessions } from './hooks/useSessions';
import { useSettings } from './hooks/useSettings';
import { AlertTriangle, Download } from 'lucide-react';

interface ChatTokenPayload {
  session_id: string;
  token: string;
  done: boolean;
}

interface DownloadProgress {
  model_id: string;
  percent: number;
  downloaded_bytes: number;
  total_bytes: number;
}

function App() {
  const {
    sessions,
    activeSessionId,
    messages,
    selectSession,
    createSession,
    deleteSession,
    addMessage,
  } = useSessions();

  const { settings, updateSettings } = useSettings();
  const [showSettings, setShowSettings] = useState(false);
  const [isGenerating, setIsGenerating] = useState(false);
  const [streamingContent, setStreamingContent] = useState('');
  const [needsDownload, setNeedsDownload] = useState(false);
  const [downloadPercent, setDownloadPercent] = useState(0);
  const [isDownloading, setIsDownloading] = useState(false);

  // Check if default model needs download on first run
  useEffect(() => {
    const checkModel = async () => {
      try {
        const hasModel: boolean = await invoke('check_default_model');
        if (!hasModel) {
          setNeedsDownload(true);
        } else {
          // Auto-load the default model
          const models: Array<{ id: string; downloaded: boolean; local_path: string | null; is_default: boolean }> =
            await invoke('list_available_models');
          const defaultModel = models.find((m) => m.is_default && m.downloaded && m.local_path);
          if (defaultModel?.local_path) {
            try {
              await invoke('load_model_cmd', { modelPath: defaultModel.local_path });
              if (!settings.active_model) {
                updateSettings({ active_model: defaultModel.local_path });
              }
            } catch (e) {
              console.error('Failed to auto-load model:', e);
            }
          }
        }
      } catch (e) {
        console.error('Model check error:', e);
      }
    };
    checkModel();
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const handleFirstRunDownload = useCallback(async () => {
    setIsDownloading(true);
    setDownloadPercent(0);

    const unlisten = await listen<DownloadProgress>('download-progress', (event) => {
      setDownloadPercent(Math.round(event.payload.percent));
    });

    try {
      const path: string = await invoke('download_model_cmd', { modelId: 'ministral-3b' });
      await invoke('load_model_cmd', { modelPath: path });
      updateSettings({ active_model: path });
      setNeedsDownload(false);
    } catch (e) {
      console.error('First-run download failed:', e);
    } finally {
      unlisten();
      setIsDownloading(false);
    }
  }, [updateSettings]);

  const activeSession = sessions.find((s) => s.id === activeSessionId);

  const handleSend = async (content: string) => {
    if (!activeSessionId) {
      const session = await createSession();
      if (!session) return;
      setTimeout(() => handleSendToSession(session.id, content), 100);
      return;
    }
    await handleSendToSession(activeSessionId, content);
  };

  const handleSendToSession = async (sessionId: string, content: string) => {
    await addMessage('user', content);

    if (!settings.active_model) {
      await addMessage(
        'assistant',
        'Ingen AI-modell laddad. Gå till Inställningar och välj en modell.'
      );
      return;
    }

    setIsGenerating(true);
    setStreamingContent('');

    // Listen for streaming tokens
    const unlisten = await listen<ChatTokenPayload>('chat-token', (event) => {
      if (event.payload.session_id === sessionId) {
        if (event.payload.done) {
          // Generation complete
          return;
        }
        setStreamingContent((prev) => prev + event.payload.token);
      }
    });

    try {
      const fullResponse: string = await invoke('chat_stream', {
        sessionId,
        message: content,
      });

      // Save full response as a message
      await addMessage('assistant', fullResponse);
    } catch (e) {
      const errorMsg = typeof e === 'string' ? e : 'Inferensfel uppstod.';
      await addMessage('assistant', `⚠️ Fel: ${errorMsg}`);
    } finally {
      unlisten();
      setIsGenerating(false);
      setStreamingContent('');
    }
  };

  const handleNewChat = async () => {
    await createSession();
  };

  // First-run download screen
  if (needsDownload) {
    return (
      <div className="loading-screen">
        <div className="logo-text">SUMRZR</div>
        {isDownloading ? (
          <>
            <p style={{ color: 'var(--text-color)', fontSize: '16px' }}>
              Laddar ner Ministral 3B...
            </p>
            <div className="first-run-progress">
              <div className="progress-bar" style={{ width: '300px', height: '8px' }}>
                <div
                  className="progress-fill"
                  style={{ width: `${downloadPercent}%` }}
                />
              </div>
              <span style={{ color: 'var(--text-color-dim)', fontSize: '14px' }}>
                {downloadPercent}% (~3.2 GB)
              </span>
            </div>
          </>
        ) : (
          <>
            <p style={{
              color: 'var(--text-color-dim)',
              fontSize: '14px',
              textAlign: 'center',
              maxWidth: '400px',
              lineHeight: '1.8',
            }}>
              Välkommen! Sumrzr behöver ladda ner en AI-modell
              <br />för att fungera. Detta görs bara en gång.
            </p>
            <button
              className="new-chat-btn"
              style={{ width: 'auto', padding: '12px 32px', fontSize: '16px', marginTop: '16px' }}
              onClick={handleFirstRunDownload}
            >
              <Download size={18} />
              Ladda ner Ministral 3B (~3.2 GB)
            </button>
            <p style={{
              color: 'var(--text-color-dim)',
              fontSize: '11px',
              opacity: 0.5,
              marginTop: '12px',
            }}>
              Modellen sparas i ~/.sumrzr/models/
            </p>
          </>
        )}
      </div>
    );
  }

  return (
    <div className="app-layout">
      <Sidebar
        sessions={sessions}
        activeSessionId={activeSessionId}
        onSelectSession={selectSession}
        onNewChat={handleNewChat}
        onDeleteSession={deleteSession}
        onOpenSettings={() => setShowSettings(true)}
      />

      <div className="main-area">
        <TerminalHeader
          activeModel={settings.active_model}
          sessionName={activeSession?.name}
        />

        {!settings.active_model && activeSessionId && (
          <div className="no-model-banner">
            <AlertTriangle size={14} />
            Ingen AI-modell vald. Gå till Inställningar för att välja en modell.
          </div>
        )}

        <ChatArea
          messages={messages}
          isGenerating={isGenerating}
          activeSessionId={activeSessionId}
          streamingContent={streamingContent}
        />

        {activeSessionId && (
          <InputBar
            onSend={handleSend}
            disabled={isGenerating}
          />
        )}
      </div>

      {showSettings && (
        <SettingsModal
          settings={settings}
          onUpdate={updateSettings}
          onClose={() => setShowSettings(false)}
          onModelLoaded={(path) => updateSettings({ active_model: path })}
        />
      )}
    </div>
  );
}

export default App;
