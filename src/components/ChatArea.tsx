import { useEffect, useRef, useState } from 'react';
import ReactMarkdown from 'react-markdown';
import { Copy, Check } from 'lucide-react';

interface Message {
    role: string;
    content: string;
    timestamp: string;
}

interface ChatAreaProps {
    messages: Message[];
    isGenerating: boolean;
    activeSessionId: string | null;
    streamingContent?: string;
}

export function ChatArea({ messages, isGenerating, activeSessionId, streamingContent }: ChatAreaProps) {
    const messagesEndRef = useRef<HTMLDivElement>(null);
    const [copiedIndex, setCopiedIndex] = useState<number | null>(null);

    const handleCopy = async (content: string, index: number) => {
        // Strip out the file attachment XML tags when copying
        const cleanContent = content.replace(/<file name="(.+?)">[\s\S]+?<\/file>\n\n/g, '');
        await navigator.clipboard.writeText(cleanContent);
        setCopiedIndex(index);
        setTimeout(() => setCopiedIndex(null), 2000);
    };

    useEffect(() => {
        messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' });
    }, [messages, isGenerating, streamingContent]);

    if (!activeSessionId) {
        return (
            <div className="empty-state">
                <div className="logo-text">SUMRZR</div>
                <p className="tagline">
                    Lokal AI-mötessummerare. Dina data stannar på din dator.
                    <br />
                    Välj en session eller skapa en ny chatt för att börja.
                </p>
                <p className="hint">
                    Modeller laddas ned automatiskt vid första starten.
                </p>
            </div>
        );
    }

    return (
        <div className="chat-messages">
            {messages.length === 0 && (
                <div className="empty-state" style={{ minHeight: '200px' }}>
                    <p className="tagline">
                        Skriv ett meddelande nedan för att börja.
                    </p>
                </div>
            )}

            {messages.map((msg, index) => (
                <div key={index} className={`message ${msg.role}`}>
                    <div className="message-header">
                        <span className="message-prefix">
                            {msg.role === 'user' ? '> användare' : '$ sumrzr'}
                        </span>

                        <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
                            <span style={{ opacity: 0.5 }}>
                                {new Date(msg.timestamp).toLocaleTimeString('sv-SE')}
                            </span>
                            {msg.role === 'assistant' && (
                                <button
                                    className="copy-btn"
                                    onClick={() => handleCopy(msg.content, index)}
                                    title="Kopiera text"
                                >
                                    {copiedIndex === index ? <Check size={14} color="var(--accent-green)" /> : <Copy size={14} />}
                                </button>
                            )}
                        </div>
                    </div>
                    <div className="message-content">
                        <ReactMarkdown>
                            {msg.role === 'user'
                                ? msg.content.replace(/<file name="(.+?)">[\s\S]+?<\/file>\n\n/g, '📎 **Bifogad fil:** $1 _(Innehåll dolt för läsbarhet)_\n\n')
                                : msg.content}
                        </ReactMarkdown>
                    </div>
                </div>
            ))
            }

            {
                isGenerating && (
                    <div className="message assistant">
                        <div className="message-header">
                            <span className="message-prefix">$ sumrzr</span>
                        </div>
                        {streamingContent ? (
                            <div className="message-content">
                                <ReactMarkdown>{streamingContent}</ReactMarkdown>
                                <span className="cursor" />
                            </div>
                        ) : (
                            <div className="typing-indicator">
                                <span className="cursor" />
                            </div>
                        )}
                    </div>
                )
            }

            <div ref={messagesEndRef} />
        </div >
    );
}

