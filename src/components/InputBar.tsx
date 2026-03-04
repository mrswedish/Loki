import React, { useState, useRef, useEffect } from 'react';
import { Paperclip, Send, X } from 'lucide-react';

interface InputBarProps {
    onSend: (message: string, attachedContent?: string) => void;
    disabled: boolean;
}

export function InputBar({ onSend, disabled }: InputBarProps) {
    const [input, setInput] = useState('');
    const [attachedFile, setAttachedFile] = useState<{ name: string; content: string } | null>(null);
    const textareaRef = useRef<HTMLTextAreaElement>(null);
    const fileInputRef = useRef<HTMLInputElement>(null);

    // Auto-resize textarea
    useEffect(() => {
        const ta = textareaRef.current;
        if (ta) {
            ta.style.height = 'auto';
            ta.style.height = `${Math.min(ta.scrollHeight, 150)}px`;
        }
    }, [input]);

    const handleSend = () => {
        if (!input.trim() && !attachedFile) return;
        if (disabled) return;

        let messageContent = input.trim();
        if (attachedFile) {
            messageContent = `<file name="${attachedFile.name}">\n${attachedFile.content}\n</file>\n\n${messageContent}`;
        }

        onSend(messageContent);
        setInput('');
        setAttachedFile(null);
    };

    const handleKeyDown = (e: React.KeyboardEvent) => {
        if (e.key === 'Enter' && !e.shiftKey) {
            e.preventDefault();
            handleSend();
        }
    };

    const handleFileAttach = async () => {
        fileInputRef.current?.click();
    };

    const handleFileChange = async (e: React.ChangeEvent<HTMLInputElement>) => {
        const file = e.target.files?.[0];
        if (!file) return;

        try {
            const text = await file.text();
            setAttachedFile({ name: file.name, content: text });
        } catch (err) {
            console.error('Failed to read file:', err);
        }

        // Reset file input
        if (fileInputRef.current) {
            fileInputRef.current.value = '';
        }
    };

    return (
        <div className="input-bar">
            {attachedFile && (
                <div className="attached-file">
                    <Paperclip size={12} />
                    <span>{attachedFile.name}</span>
                    <button
                        className="remove-file"
                        onClick={() => setAttachedFile(null)}
                        title="Ta bort bifogad fil"
                    >
                        <X size={14} />
                    </button>
                </div>
            )}
            <div className="input-container">
                <textarea
                    ref={textareaRef}
                    value={input}
                    onChange={(e) => setInput(e.target.value)}
                    onKeyDown={handleKeyDown}
                    placeholder="Skriv ett meddelande..."
                    disabled={disabled}
                    rows={1}
                />
                <div className="input-actions">
                    <input
                        ref={fileInputRef}
                        type="file"
                        accept=".txt,.md,.csv,.json,.log"
                        style={{ display: 'none' }}
                        onChange={handleFileChange}
                    />
                    <button onClick={handleFileAttach} title="Bifoga textfil">
                        <Paperclip size={18} />
                    </button>
                    <button
                        className="send-btn"
                        onClick={handleSend}
                        disabled={disabled || (!input.trim() && !attachedFile)}
                        title="Skicka (Enter)"
                    >
                        <Send size={18} />
                    </button>
                </div>
            </div>
        </div>
    );
}
