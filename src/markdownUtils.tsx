import React from "react";

/**
 * Renderar inline-formatering (**fetstil**, *kursiv*, `kod`) säkert som React-noder utan innerHTML.
 */
export function renderInlineFormattedText(
  text: string,
  keyPrefix: string
): React.ReactNode {
  if (!text) return null;
  if (!text.includes("**") && !text.includes("`") && !text.includes("*")) {
    return text;
  }

  const tokenRe = /(\*\*[^*\n]+\*\*|`[^`\n]+`|\*(?!\s)[^*\n]+(?<!\s)\*)/g;
  const parts: React.ReactNode[] = [];
  let lastIdx = 0;
  let match: RegExpExecArray | null;
  let idx = 0;

  while ((match = tokenRe.exec(text)) !== null) {
    if (match.index > lastIdx) {
      parts.push(text.slice(lastIdx, match.index));
    }
    const tok = match[0];
    if (tok.startsWith("**") && tok.endsWith("**")) {
      parts.push(
        <strong key={`${keyPrefix}-b-${idx++}`}>{tok.slice(2, -2)}</strong>
      );
    } else if (tok.startsWith("`") && tok.endsWith("`")) {
      parts.push(
        <code key={`${keyPrefix}-c-${idx++}`} className="inline-doc-code">
          {tok.slice(1, -1)}
        </code>
      );
    } else if (tok.startsWith("*") && tok.endsWith("*")) {
      parts.push(<em key={`${keyPrefix}-i-${idx++}`}>{tok.slice(1, -1)}</em>);
    } else {
      parts.push(tok);
    }
    lastIdx = tokenRe.lastIndex;
  }

  if (lastIdx < text.length) {
    parts.push(text.slice(lastIdx));
  }

  return parts.length === 1 ? parts[0] : parts;
}

const HR_RE = /^(-{3,}|\*{3,}|_{3,})$/;
const HEADING_RE = /^(#{1,6})(?:\s+(.*))?$/;
const UL_ITEM_RE = /^\s*[-*•]\s+/;
const OL_ITEM_RE = /^\s*\d+[.)]\s+/;

function isStandaloneBoldHeading(trimmed: string): boolean {
  return (
    trimmed.startsWith("**") &&
    trimmed.endsWith("**") &&
    trimmed.length > 4 &&
    !trimmed.slice(2, -2).includes("**")
  );
}

/**
 * Avgör om en rad inleder ett nytt blockelement (och därför ska bryta ett pågående brödtextstycke).
 * Måste exakt matcha blockgrenarna i `renderFormattedMarkdown` och `markdownToCleanHtml`.
 */
function isBlockStarterLine(line: string): boolean {
  const trimmed = line.trim();
  if (!trimmed) return true;
  if (trimmed.startsWith("```")) return true;
  if (HR_RE.test(trimmed)) return true;
  if (HEADING_RE.test(trimmed)) return true;
  if (isStandaloneBoldHeading(trimmed)) return true;
  if (UL_ITEM_RE.test(line)) return true;
  if (OL_ITEM_RE.test(line)) return true;
  if (trimmed.startsWith(">")) return true;
  return false;
}

/**
 * Renderar Markdown-text som ren skandinavisk dokumenttypografi (rubriker, punktlistor,
 * checklistor, numrerade listor, citat, fetstil) helt i React JSX utan XSS-risk.
 */
export function renderFormattedMarkdown(text: string): React.ReactNode {
  if (!text.trim()) return null;

  const lines = text.replace(/\r\n/g, "\n").split("\n");
  const blocks: React.ReactNode[] = [];
  let i = 0;
  let blockKey = 0;

  while (i < lines.length) {
    const prevIndex = i;
    const line = lines[i];
    const trimmed = line.trim();

    if (!trimmed) {
      i++;
      continue;
    }

    if (trimmed.startsWith("```")) {
      const codeLines: string[] = [];
      i++;
      while (i < lines.length && !lines[i].trim().startsWith("```")) {
        codeLines.push(lines[i]);
        i++;
      }
      if (i < lines.length) i++;
      blocks.push(
        <pre key={`md-pre-${blockKey++}`} className="md-code-block">
          <code>{codeLines.join("\n")}</code>
        </pre>
      );
      continue;
    }

    if (HR_RE.test(trimmed)) {
      blocks.push(<hr key={`md-hr-${blockKey++}`} className="md-hr" />);
      i++;
      continue;
    }

    const headingMatch = trimmed.match(HEADING_RE);
    if (headingMatch) {
      const level = headingMatch[1].length;
      const headingText = (headingMatch[2] ?? "").trim();
      i++;
      // Under strömning kan första token vara enbart "###" innan rubriktexten anländer
      if (!headingText) {
        continue;
      }
      const content = renderInlineFormattedText(
        headingText,
        `md-h-${blockKey}`
      );
      if (level === 1) {
        blocks.push(
          <h2 key={`md-h1-${blockKey++}`} className="md-h1">
            {content}
          </h2>
        );
      } else if (level === 2) {
        blocks.push(
          <h3 key={`md-h2-${blockKey++}`} className="md-h2">
            {content}
          </h3>
        );
      } else {
        blocks.push(
          <h4 key={`md-h3-${blockKey++}`} className="md-h3">
            {content}
          </h4>
        );
      }
      continue;
    }

    if (isStandaloneBoldHeading(trimmed)) {
      blocks.push(
        <h3 key={`md-bh-${blockKey++}`} className="md-h2">
          {trimmed.slice(2, -2)}
        </h3>
      );
      i++;
      continue;
    }

    // Punktlista och checklista (- [ ] / - [x] / - / * / •)
    if (UL_ITEM_RE.test(line)) {
      const items: React.ReactNode[] = [];
      let liIdx = 0;
      while (i < lines.length && UL_ITEM_RE.test(lines[i])) {
        const rawItem = lines[i].replace(UL_ITEM_RE, "");
        const unchecked = rawItem.startsWith("[ ] ");
        const checked =
          rawItem.startsWith("[x] ") || rawItem.startsWith("[X] ");
        const bodyText = unchecked || checked ? rawItem.slice(4) : rawItem;
        items.push(
          <li
            key={`md-ul-li-${blockKey}-${liIdx++}`}
            className={unchecked || checked ? "md-task-item" : undefined}
          >
            {(unchecked || checked) && (
              <span className="md-task-box" aria-hidden="true">
                {checked ? "☑" : "☐"}
              </span>
            )}
            <span>
              {renderInlineFormattedText(bodyText, `md-ul-${blockKey}-${liIdx}`)}
            </span>
          </li>
        );
        i++;
      }
      blocks.push(
        <ul key={`md-ul-${blockKey++}`} className="md-list">
          {items}
        </ul>
      );
      continue;
    }

    if (OL_ITEM_RE.test(line)) {
      const items: React.ReactNode[] = [];
      let liIdx = 0;
      while (i < lines.length && OL_ITEM_RE.test(lines[i])) {
        const itemText = lines[i].replace(OL_ITEM_RE, "");
        items.push(
          <li key={`md-ol-li-${blockKey}-${liIdx++}`}>
            {renderInlineFormattedText(itemText, `md-ol-${blockKey}-${liIdx}`)}
          </li>
        );
        i++;
      }
      blocks.push(
        <ol key={`md-ol-${blockKey++}`} className="md-list md-ordered-list">
          {items}
        </ol>
      );
      continue;
    }

    if (trimmed.startsWith(">")) {
      const quoteLines: string[] = [];
      while (i < lines.length && lines[i].trim().startsWith(">")) {
        quoteLines.push(lines[i].trim().replace(/^>\s?/, ""));
        i++;
      }
      blocks.push(
        <blockquote key={`md-q-${blockKey++}`} className="md-blockquote">
          {renderInlineFormattedText(quoteLines.join(" "), `md-qi-${blockKey}`)}
        </blockquote>
      );
      continue;
    }

    // Konsumera ALLTID minst aktuell rad först så att loopen garanterat avancerar oavsett indata!
    const paraLines: string[] = [lines[i]];
    i++;
    while (i < lines.length && !isBlockStarterLine(lines[i])) {
      paraLines.push(lines[i]);
      i++;
    }

    if (i <= prevIndex) {
      i = prevIndex + 1;
    }

    blocks.push(
      <p key={`md-p-${blockKey++}`} className="md-para">
        {paraLines.map((pl, idx) => (
          <React.Fragment key={idx}>
            {idx > 0 && <br />}
            {renderInlineFormattedText(pl, `md-pl-${blockKey}-${idx}`)}
          </React.Fragment>
        ))}
      </p>
    );
  }

  return blocks.length > 0 ? <div className="formatted-md">{blocks}</div> : null;
}

function escapeHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

function formatInlineHtml(text: string): string {
  const escaped = escapeHtml(text);
  return escaped
    .replace(/\*\*([^*\n]+)\*\*/g, "<strong>$1</strong>")
    .replace(/`([^`\n]+)`/g, "<code>$1</code>")
    .replace(/\*(?!\s)([^*\n]+)(?<!\s)\*/g, "<em>$1</em>");
}

/**
 * Konverterar Markdown-text till semantisk HTML för urklipp (`text/html`),
 * så att inklistring i Word, Outlook, Teams eller Google Docs ger riktiga rubriker, fetstil och punktlistor.
 */
export function markdownToCleanHtml(text: string, title?: string): string {
  const lines = text.replace(/\r\n/g, "\n").split("\n");
  const out: string[] = [];
  if (title && title.trim()) {
    out.push(`<h1>${escapeHtml(title.trim())}</h1>`);
  }
  let i = 0;

  while (i < lines.length) {
    const prevIndex = i;
    const line = lines[i];
    const trimmed = line.trim();
    if (!trimmed) {
      i++;
      continue;
    }

    if (trimmed.startsWith("```")) {
      const codeLines: string[] = [];
      i++;
      while (i < lines.length && !lines[i].trim().startsWith("```")) {
        codeLines.push(escapeHtml(lines[i]));
        i++;
      }
      if (i < lines.length) i++;
      out.push(`<pre><code>${codeLines.join("\n")}</code></pre>`);
      continue;
    }

    if (HR_RE.test(trimmed)) {
      out.push("<hr>");
      i++;
      continue;
    }

    const hm = trimmed.match(HEADING_RE);
    if (hm) {
      const headingText = (hm[2] ?? "").trim();
      i++;
      if (!headingText) {
        continue;
      }
      const lvl = Math.min(4, hm[1].length + 1);
      out.push(`<h${lvl}>${formatInlineHtml(headingText)}</h${lvl}>`);
      continue;
    }

    if (isStandaloneBoldHeading(trimmed)) {
      out.push(`<h3>${escapeHtml(trimmed.slice(2, -2))}</h3>`);
      i++;
      continue;
    }

    if (UL_ITEM_RE.test(line)) {
      const items: string[] = [];
      while (i < lines.length && UL_ITEM_RE.test(lines[i])) {
        let itemRaw = lines[i].replace(UL_ITEM_RE, "");
        if (itemRaw.startsWith("[ ] ")) {
          itemRaw = `☐ ${itemRaw.slice(4)}`;
        } else if (itemRaw.startsWith("[x] ") || itemRaw.startsWith("[X] ")) {
          itemRaw = `☑ ${itemRaw.slice(4)}`;
        }
        items.push(`<li>${formatInlineHtml(itemRaw)}</li>`);
        i++;
      }
      out.push(`<ul>${items.join("")}</ul>`);
      continue;
    }

    if (OL_ITEM_RE.test(line)) {
      const items: string[] = [];
      while (i < lines.length && OL_ITEM_RE.test(lines[i])) {
        items.push(
          `<li>${formatInlineHtml(lines[i].replace(OL_ITEM_RE, ""))}</li>`
        );
        i++;
      }
      out.push(`<ol>${items.join("")}</ol>`);
      continue;
    }

    if (trimmed.startsWith(">")) {
      const qLines: string[] = [];
      while (i < lines.length && lines[i].trim().startsWith(">")) {
        qLines.push(formatInlineHtml(lines[i].trim().replace(/^>\s?/, "")));
        i++;
      }
      out.push(`<blockquote>${qLines.join(" ")}</blockquote>`);
      continue;
    }

    const pLines: string[] = [formatInlineHtml(lines[i])];
    i++;
    while (i < lines.length && !isBlockStarterLine(lines[i])) {
      pLines.push(formatInlineHtml(lines[i]));
      i++;
    }
    if (i <= prevIndex) {
      i = prevIndex + 1;
    }
    out.push(`<p>${pLines.join("<br>")}</p>`);
  }

  return out.join("\n");
}

/**
 * Konverterar Markdown till ren läsbar text utan råa `**` eller `###`-tecken.
 */
export function markdownToCleanPlainText(text: string): string {
  return text
    .replace(/\r\n/g, "\n")
    .split("\n")
    .map((line) => {
      let l = line;
      l = l.replace(/^\s*#{1,6}\s*/, "");
      l = l.replace(/^\s*[-*]\s+\[\s\]\s+/, "☐ ");
      l = l.replace(/^\s*[-*]\s+\[[xX]\]\s+/, "☑ ");
      l = l.replace(/^\s*[-*]\s+/, "• ");
      l = l.replace(/\*\*([^*\n]+)\*\*/g, "$1");
      l = l.replace(/`([^`\n]+)`/g, "$1");
      l = l.replace(/\*(?!\s)([^*\n]+)(?<!\s)\*/g, "$1");
      return l;
    })
    .join("\n");
}
