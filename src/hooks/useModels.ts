import { useState, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

interface ModelInfo {
    name: string;
    filename: string;
    path: string;
    size_bytes: number;
}

export interface AvailableModel {
    id: string;
    name: string;
    filename: string;
    url: string;
    size_bytes: number;
    description: string;
    is_default: boolean;
    downloaded: boolean;
    local_path: string | null;
}

interface DownloadProgress {
    model_id: string;
    percent: number;
    downloaded_bytes: number;
    total_bytes: number;
}

export function useModels() {
    const [models, setModels] = useState<ModelInfo[]>([]);
    const [availableModels, setAvailableModels] = useState<AvailableModel[]>([]);
    const [downloading, setDownloading] = useState<string | null>(null);
    const [downloadPercent, setDownloadPercent] = useState(0);

    const loadModels = useCallback(async () => {
        try {
            const list: ModelInfo[] = await invoke('list_models');
            setModels(list);
        } catch (e) {
            console.error('Failed to load models:', e);
        }
    }, []);

    const loadAvailableModels = useCallback(async () => {
        try {
            const list: AvailableModel[] = await invoke('list_available_models');
            setAvailableModels(list);
        } catch (e) {
            console.error('Failed to load available models:', e);
        }
    }, []);

    const downloadModel = useCallback(async (modelId: string): Promise<string | null> => {
        setDownloading(modelId);
        setDownloadPercent(0);

        // Listen for progress events
        const unlisten = await listen<DownloadProgress>('download-progress', (event) => {
            if (event.payload.model_id === modelId) {
                setDownloadPercent(Math.round(event.payload.percent));
            }
        });

        try {
            const path: string = await invoke('download_model_cmd', { modelId });
            await loadAvailableModels();
            await loadModels();
            return path;
        } catch (e) {
            console.error('Download failed:', e);
            return null;
        } finally {
            unlisten();
            setDownloading(null);
            setDownloadPercent(0);
        }
    }, [loadAvailableModels, loadModels]);

    const loadModelIntoEngine = useCallback(async (modelPath: string) => {
        try {
            await invoke('load_model_cmd', { modelPath });
            return true;
        } catch (e) {
            console.error('Failed to load model:', e);
            return false;
        }
    }, []);

    const deleteModel = useCallback(async (modelId: string) => {
        try {
            await invoke('delete_model_cmd', { modelId });
            await loadAvailableModels();
            await loadModels();
            return true;
        } catch (e) {
            console.error('Failed to delete model:', e);
            return false;
        }
    }, [loadAvailableModels, loadModels]);

    const formatSize = (bytes: number): string => {
        if (bytes >= 1e9) return `${(bytes / 1e9).toFixed(1)} GB`;
        if (bytes >= 1e6) return `${(bytes / 1e6).toFixed(0)} MB`;
        return `${(bytes / 1e3).toFixed(0)} KB`;
    };

    return {
        models,
        availableModels,
        downloading,
        downloadPercent,
        loadModels,
        loadAvailableModels,
        downloadModel,
        loadModelIntoEngine,
        deleteModel,
        formatSize,
    };
}
