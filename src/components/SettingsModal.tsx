import { useEffect, useState } from 'react';
import { X, Download, Check, Loader, Trash2 } from 'lucide-react';
import { useModels, AvailableModel } from '../hooks/useModels';

interface Settings {
    active_model: string | null;
    font_size: number;
    scanline_intensity: number;
    text_color: string;
}

interface SettingsModalProps {
    settings: Settings;
    onUpdate: (partial: Partial<Settings>) => void;
    onClose: () => void;
    onModelLoaded?: (path: string) => void;
}

const COLOR_PRESETS = [
    { color: '#ffb000', label: 'Amber' },
    { color: '#33ff33', label: 'Grön' },
    { color: '#ff6633', label: 'Orange' },
    { color: '#00ccff', label: 'Cyan' },
    { color: '#ffffff', label: 'Vit' },
];

export function SettingsModal({ settings, onUpdate, onClose, onModelLoaded }: SettingsModalProps) {
    const {
        availableModels,
        downloading,
        downloadPercent,
        loadAvailableModels,
        downloadModel,
        loadModelIntoEngine,
        deleteModel,
        formatSize,
    } = useModels();

    const [loadingModel, setLoadingModel] = useState<string | null>(null);

    useEffect(() => {
        loadAvailableModels();
    }, [loadAvailableModels]);

    // Close on Escape
    useEffect(() => {
        const handleKeyDown = (e: KeyboardEvent) => {
            if (e.key === 'Escape') onClose();
        };
        window.addEventListener('keydown', handleKeyDown);
        return () => window.removeEventListener('keydown', handleKeyDown);
    }, [onClose]);

    const handleSelectModel = async (model: AvailableModel) => {
        if (!model.downloaded) {
            // Download first
            const path = await downloadModel(model.id);
            if (path) {
                setLoadingModel(model.id);
                const success = await loadModelIntoEngine(path);
                if (success) {
                    onUpdate({ active_model: path });
                    onModelLoaded?.(path);
                }
                setLoadingModel(null);
            }
        } else if (model.local_path) {
            // Already downloaded, just load
            setLoadingModel(model.id);
            const success = await loadModelIntoEngine(model.local_path);
            if (success) {
                onUpdate({ active_model: model.local_path });
                onModelLoaded?.(model.local_path);
            }
            setLoadingModel(null);
        }
    };

    const isActive = (model: AvailableModel) => {
        return settings.active_model === model.local_path;
    };

    return (
        <div className="modal-overlay" onClick={(e) => {
            if (e.target === e.currentTarget) onClose();
        }}>
            <div className="settings-modal">
                <div className="settings-header">
                    <h2>⚙ Inställningar</h2>
                    <button className="settings-close" onClick={onClose}>
                        <X size={20} />
                    </button>
                </div>

                <div className="settings-body">
                    {/* AI Models */}
                    <div className="settings-section">
                        <h3>AI-Modeller</h3>
                        <div className="model-list">
                            {availableModels.map((model) => (
                                <div
                                    key={model.id}
                                    className={`model-card ${isActive(model) ? 'active' : ''}`}
                                >
                                    <div className="model-info">
                                        <div className="model-name">
                                            {model.name}
                                            {model.is_default && (
                                                <span className="model-badge">Standard</span>
                                            )}
                                        </div>
                                        <div className="model-desc">{model.description}</div>
                                        <div className="model-size">{formatSize(model.size_bytes)}</div>
                                    </div>
                                    <div className="model-action">
                                        {downloading === model.id ? (
                                            <div className="download-progress">
                                                <div className="progress-bar">
                                                    <div
                                                        className="progress-fill"
                                                        style={{ width: `${downloadPercent}%` }}
                                                    />
                                                </div>
                                                <span className="progress-text">{downloadPercent}%</span>
                                            </div>
                                        ) : loadingModel === model.id ? (
                                            <button className="model-btn loading" disabled>
                                                <Loader size={14} className="spin" /> Laddar...
                                            </button>
                                        ) : isActive(model) ? (
                                            <button className="model-btn active" disabled>
                                                <Check size={14} /> Aktiv
                                            </button>
                                        ) : model.downloaded ? (
                                            <>
                                                <button
                                                    className="model-btn"
                                                    onClick={() => handleSelectModel(model)}
                                                >
                                                    Välj
                                                </button>
                                                <button
                                                    className="model-btn danger"
                                                    style={{ marginLeft: '8px', padding: '6px', background: 'transparent', color: '#ff4444' }}
                                                    title="Ta bort modell"
                                                    onClick={() => {
                                                        if (confirm(`Är du säker på att du vill ta bort ${model.name}?`)) {
                                                            deleteModel(model.id);
                                                        }
                                                    }}
                                                >
                                                    <Trash2 size={16} />
                                                </button>
                                            </>
                                        ) : (
                                            <button
                                                className="model-btn download"
                                                onClick={() => handleSelectModel(model)}
                                            >
                                                <Download size={14} /> Ladda ner
                                            </button>
                                        )}
                                    </div>
                                </div>
                            ))}
                        </div>
                    </div>

                    {/* Display Settings */}
                    <div className="settings-section">
                        <h3>Grafik</h3>

                        <div className="setting-row">
                            <label>Textstorlek</label>
                            <div style={{ display: 'flex', alignItems: 'center', gap: '10px' }}>
                                <input
                                    type="range"
                                    min={12}
                                    max={24}
                                    value={settings.font_size}
                                    onChange={(e) => onUpdate({ font_size: parseInt(e.target.value) })}
                                />
                                <span className="range-value">{settings.font_size}px</span>
                            </div>
                        </div>

                        <div className="setting-row">
                            <label>Scanline-intensitet</label>
                            <div style={{ display: 'flex', alignItems: 'center', gap: '10px' }}>
                                <input
                                    type="range"
                                    min={0}
                                    max={100}
                                    value={settings.scanline_intensity}
                                    onChange={(e) => onUpdate({ scanline_intensity: parseInt(e.target.value) })}
                                />
                                <span className="range-value">{settings.scanline_intensity}%</span>
                            </div>
                        </div>

                        <div className="setting-row">
                            <label>Textfärg</label>
                            <div className="color-presets">
                                {COLOR_PRESETS.map((preset) => (
                                    <button
                                        key={preset.color}
                                        className={`color-preset ${settings.text_color === preset.color ? 'active' : ''}`}
                                        style={{ backgroundColor: preset.color }}
                                        onClick={() => onUpdate({ text_color: preset.color })}
                                        title={preset.label}
                                    />
                                ))}
                            </div>
                        </div>
                    </div>

                    {/* About */}
                    <div className="settings-section">
                        <h3>Om</h3>
                        <p style={{ fontSize: '12px', color: 'var(--text-color-dim)', lineHeight: '1.8' }}>
                            Loke v0.1.2
                            <br />
                            Lokala Kontext-Expert
                            <br />
                            Byggd med Tauri + React + llama.cpp
                            <br />
                            Dina data stannar på din dator.
                        </p>
                    </div>
                </div>
            </div>
        </div>
    );
}
