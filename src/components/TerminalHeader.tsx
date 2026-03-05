
import { Cpu } from 'lucide-react';

interface TerminalHeaderProps {
    activeModel: string | null;
    sessionName?: string;
}

export function TerminalHeader({ activeModel, sessionName }: TerminalHeaderProps) {
    const modelName = activeModel
        ? activeModel.split('/').pop()?.replace('.gguf', '') || 'Okänd'
        : null;

    return (
        <div className="terminal-header">
            <div className="title">
                {sessionName || 'LOKE TERMINAL'}
            </div>
            <div className="model-indicator">
                <Cpu size={14} />
                <span className={`model-dot ${activeModel ? '' : 'offline'}`} />
                <span>{modelName || 'Ingen modell'}</span>
            </div>
        </div>
    );
}
