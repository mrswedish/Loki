use std::io::{Cursor, Read, Write};

use calamine::{open_workbook_auto_from_rs, Data, Reader};
use quick_xml::events::Event;
use quick_xml::reader::Reader as XmlReader;
use serde::{Deserialize, Serialize};
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

use crate::transcript::{clean_transcript, TranscriptStats};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedDocument {
    pub file_name: String,
    pub format: String, // "docx" | "xlsx" | "pdf" | "odt" | "vtt" | "srt" | "text"
    pub text: String,
    pub summary: String,
    pub transcript_stats: Option<TranscriptStats>,
}

/// Läser in dokument portabelt helt i Rust (utan installerat Microsoft Office):
/// - Word & OpenDocument: `.docx`, `.odt` (inkl. tabeller, sidhuvud, fotnoter)
/// - Excel & Kalkylblad: `.xlsx`, `.xls`, `.xlsm`, `.xlsb`, `.ods`
/// - PDF: `.pdf`
/// - Mötestranskript: `.vtt`, `.srt`
/// - Text & data: `.txt`, `.md`, `.csv`, `.json`, `.xml` (med svensk Windows-1252-fallback)
pub fn import_document_bytes(
    file_name: &str,
    bytes: &[u8],
    clean_as_transcript: bool,
) -> Result<ImportedDocument, String> {
    let ext = file_name
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();

    let (format, raw_text, base_summary) = match ext.as_str() {
        "docx" => {
            let (text, paras) = extract_docx_text(bytes)?;
            (
                "docx".to_string(),
                text,
                format!("Word-dokument (.docx) · {paras} stycken inlästa lokalt"),
            )
        }
        "odt" => {
            let (text, paras) = extract_odt_text(bytes)?;
            (
                "odt".to_string(),
                text,
                format!("OpenDocument (.odt) · {paras} stycken inlästa lokalt"),
            )
        }
        "xlsx" | "xls" | "xlsm" | "xlsb" | "ods" => {
            let (text, sheets, rows) = extract_spreadsheet_text(bytes)?;
            (
                "xlsx".to_string(),
                text,
                format!("Kalkylblad (.{ext}) · {sheets} flik(ar), {rows} rader inlästa lokalt"),
            )
        }
        "pdf" => {
            let text = extract_pdf_text(bytes)?;
            let char_count = text.chars().count();
            (
                "pdf".to_string(),
                text,
                format!("PDF-dokument (.pdf) · {char_count} tecken extraherade lokalt"),
            )
        }
        "vtt" | "srt" => {
            let text = decode_text_with_swedish_fallback(bytes);
            let lines = text.lines().count();
            (
                ext.clone(),
                text,
                format!(
                    "Mötestranskript (.{ext}) · {lines} rader inlästa lokalt"
                ),
            )
        }
        _ => {
            let text = decode_text_with_swedish_fallback(bytes);
            let lines = text.lines().count();
            (
                "text".to_string(),
                text,
                format!("Textfil · {lines} rader inlästa lokalt"),
            )
        }
    };

    if clean_as_transcript {
        let cleaned = clean_transcript(&raw_text, Some(&ext));
        let summary = if cleaned.stats.chars_saved_percent > 0 {
            format!(
                "{base_summary} · Städad (-{} % tidsstämplar/brus)",
                cleaned.stats.chars_saved_percent
            )
        } else {
            base_summary
        };
        Ok(ImportedDocument {
            file_name: file_name.to_string(),
            format,
            text: cleaned.text,
            summary,
            transcript_stats: Some(cleaned.stats),
        })
    } else {
        Ok(ImportedDocument {
            file_name: file_name.to_string(),
            format,
            text: raw_text,
            summary: base_summary,
            transcript_stats: None,
        })
    }
}

/// Avkodar UTF-8, med automatisk fallback till Windows-1252 / ISO-8859-1 för svenska
/// CSV/TXT-filer exporterade från äldre Windows-program eller Excel.
fn decode_text_with_swedish_fallback(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    if let Ok(utf8) = std::str::from_utf8(bytes) {
        return utf8.to_string();
    }
    bytes.iter().map(|&b| cp1252_byte_to_char(b)).collect()
}

fn cp1252_byte_to_char(b: u8) -> char {
    match b {
        0x80 => '€',
        0x82 => '‚',
        0x83 => 'ƒ',
        0x84 => '„',
        0x85 => '…',
        0x86 => '†',
        0x87 => '‡',
        0x88 => 'ˆ',
        0x89 => '‰',
        0x8A => 'Š',
        0x8B => '‹',
        0x8C => 'Œ',
        0x8E => 'Ž',
        0x91 => '‘',
        0x92 => '’',
        0x93 => '“',
        0x94 => '”',
        0x95 => '•',
        0x96 => '–',
        0x97 => '—',
        0x98 => '˜',
        0x99 => '™',
        0x9A => 'š',
        0x9B => '›',
        0x9C => 'œ',
        0x9E => 'ž',
        0x9F => 'Ÿ',
        other => other as char,
    }
}

/// Extraherar text ur Word (.docx) inkl. sidhuvud, brödtext, tabeller, fotnoter och sidfot.
fn extract_docx_text(bytes: &[u8]) -> Result<(String, usize), String> {
    let cursor = Cursor::new(bytes);
    let mut archive = ZipArchive::new(cursor)
        .map_err(|e| format!("Kunde inte läsa .docx-arkiv (är filen skadad?): {e}"))?;

    let mut xml_parts: Vec<String> = Vec::new();
    for i in 0..archive.len() {
        if let Ok(f) = archive.by_index(i) {
            let name = f.name().to_string();
            if name == "word/document.xml"
                || name.starts_with("word/header")
                || name.starts_with("word/footer")
                || name == "word/footnotes.xml"
                || name == "word/endnotes.xml"
            {
                xml_parts.push(name);
            }
        }
    }

    xml_parts.sort_by_key(|name| {
        if name.starts_with("word/header") {
            0
        } else if name == "word/document.xml" {
            1
        } else if name == "word/footnotes.xml" || name == "word/endnotes.xml" {
            2
        } else {
            3
        }
    });

    if xml_parts.is_empty() {
        return Err("Hittade ingen word/document.xml i .docx-filen.".to_string());
    }

    let mut all_paragraphs: Vec<String> = Vec::new();
    for part_name in xml_parts {
        let mut file = archive
            .by_name(&part_name)
            .map_err(|e| format!("Kunde inte läsa {part_name}: {e}"))?;
        let mut xml_content = String::new();
        file.read_to_string(&mut xml_content)
            .map_err(|e| format!("Kunde inte avkoda {part_name}: {e}"))?;

        let part_paras = parse_word_xml_paragraphs(&xml_content)?;
        all_paragraphs.extend(part_paras);
    }

    let count = all_paragraphs.len();
    Ok((all_paragraphs.join("\n\n"), count))
}

fn parse_word_xml_paragraphs(xml: &str) -> Result<Vec<String>, String> {
    let mut reader = XmlReader::from_str(xml);
    reader.config_mut().trim_text(false);

    let mut paragraphs: Vec<String> = Vec::new();
    let mut current_para = String::new();
    let mut current_row_cells: Vec<String> = Vec::new();
    let mut in_table_cell = false;
    let mut in_text_node = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let local = e.local_name();
                match local.as_ref() {
                    b"t" | b"delText" => in_text_node = true,
                    b"tc" => in_table_cell = true,
                    b"tr" => current_row_cells.clear(),
                    b"tab" => current_para.push('\t'),
                    b"br" | b"cr" => current_para.push('\n'),
                    _ => {}
                }
            }
            Ok(Event::Empty(e)) => {
                let local = e.local_name();
                match local.as_ref() {
                    b"tab" => current_para.push('\t'),
                    b"br" | b"cr" => current_para.push('\n'),
                    _ => {}
                }
            }
            Ok(Event::Text(e)) => {
                if in_text_node {
                    if let Ok(txt) = e.unescape() {
                        current_para.push_str(&txt);
                    }
                }
            }
            Ok(Event::End(e)) => {
                let local = e.local_name();
                match local.as_ref() {
                    b"t" | b"delText" => in_text_node = false,
                    b"p" => {
                        let trimmed = current_para.trim();
                        if !trimmed.is_empty() {
                            if in_table_cell {
                                current_row_cells.push(trimmed.replace('\n', " "));
                            } else {
                                paragraphs.push(trimmed.to_string());
                            }
                        }
                        current_para.clear();
                    }
                    b"tc" => in_table_cell = false,
                    b"tr" => {
                        if !current_row_cells.is_empty() {
                            paragraphs.push(current_row_cells.join(" | "));
                            current_row_cells.clear();
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(format!("XML-tolkningsfel i Word-dokument: {e}")),
            _ => {}
        }
    }

    Ok(paragraphs)
}

/// Extraherar text ur OpenDocument Text (.odt) via content.xml.
fn extract_odt_text(bytes: &[u8]) -> Result<(String, usize), String> {
    let cursor = Cursor::new(bytes);
    let mut archive =
        ZipArchive::new(cursor).map_err(|e| format!("Kunde inte läsa .odt-arkiv: {e}"))?;
    let mut file = archive
        .by_name("content.xml")
        .map_err(|e| format!("Saknar content.xml i .odt-filen: {e}"))?;
    let mut xml = String::new();
    file.read_to_string(&mut xml)
        .map_err(|e| format!("Kunde inte läsa content.xml: {e}"))?;

    let mut reader = XmlReader::from_str(&xml);
    reader.config_mut().trim_text(false);

    let mut paragraphs: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let local = e.local_name();
                if local.as_ref() == b"p" || local.as_ref() == b"h" {
                    depth += 1;
                } else if depth > 0 && local.as_ref() == b"tab" {
                    current.push('\t');
                } else if depth > 0 && local.as_ref() == b"line-break" {
                    current.push('\n');
                }
            }
            Ok(Event::Empty(e)) => {
                if depth > 0 {
                    let local = e.local_name();
                    if local.as_ref() == b"tab" {
                        current.push('\t');
                    } else if local.as_ref() == b"line-break" {
                        current.push('\n');
                    }
                }
            }
            Ok(Event::Text(e)) => {
                if depth > 0 {
                    if let Ok(t) = e.unescape() {
                        current.push_str(&t);
                    }
                }
            }
            Ok(Event::End(e)) => {
                let local = e.local_name();
                if (local.as_ref() == b"p" || local.as_ref() == b"h") && depth > 0 {
                    depth -= 1;
                    if depth == 0 {
                        let trimmed = current.trim();
                        if !trimmed.is_empty() {
                            paragraphs.push(trimmed.to_string());
                        }
                        current.clear();
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(format!("XML-fel i .odt: {e}")),
            _ => {}
        }
    }

    let count = paragraphs.len();
    Ok((paragraphs.join("\n\n"), count))
}

/// Extraherar text ur Excel- och OpenDocument-kalkylblad (.xlsx, .xls, .xlsm, .xlsb, .ods).
fn extract_spreadsheet_text(bytes: &[u8]) -> Result<(String, usize, usize), String> {
    let cursor = Cursor::new(bytes);
    let mut workbook = open_workbook_auto_from_rs(cursor)
        .map_err(|e| format!("Kunde inte läsa kalkylbladet: {e}"))?;

    let sheet_names = workbook.sheet_names().to_vec();
    let mut sections: Vec<String> = Vec::new();
    let mut non_empty_sheets = 0usize;
    let mut total_rows = 0usize;

    for sheet_name in &sheet_names {
        let Ok(range) = workbook.worksheet_range(sheet_name) else {
            continue;
        };

        let mut sheet_rows: Vec<String> = Vec::new();
        for row in range.rows() {
            let cells: Vec<String> = row
                .iter()
                .map(format_cell)
                .filter(|s| !s.is_empty())
                .collect();
            if !cells.is_empty() {
                total_rows += 1;
                sheet_rows.push(cells.join(" | "));
            }
        }

        if !sheet_rows.is_empty() {
            non_empty_sheets += 1;
            let mut section = String::new();
            if sheet_names.len() > 1 {
                section.push_str(&format!("=== Flik: {sheet_name} ===\n\n"));
            }
            section.push_str(&sheet_rows.join("\n"));
            sections.push(section);
        }
    }

    if sections.is_empty() {
        return Err("Kalkylbladet verkar vara helt tomt.".to_string());
    }

    Ok((sections.join("\n\n"), non_empty_sheets, total_rows))
}

fn format_cell(cell: &Data) -> String {
    match cell {
        Data::Empty => String::new(),
        Data::String(s) => s.trim().replace('\n', " "),
        Data::Float(f) => {
            if f.fract() == 0.0 && f.abs() < 1e15 {
                format!("{:.0}", f)
            } else {
                format!("{f}")
            }
        }
        Data::Int(i) => i.to_string(),
        Data::Bool(b) => if *b { "Ja" } else { "Nej" }.to_string(),
        Data::DateTime(dt) => dt.to_string(),
        Data::DateTimeIso(s) | Data::DurationIso(s) => s.trim().to_string(),
        Data::Error(e) => format!("{e:?}"),
    }
}

/// Extraherar text ur digitala PDF-dokument (.pdf) i minnet.
fn extract_pdf_text(bytes: &[u8]) -> Result<String, String> {
    let raw = pdf_extract::extract_text_from_mem(bytes)
        .map_err(|e| format!("Kunde inte extrahera text ur PDF-filen: {e}"))?;

    let cleaned = raw
        .replace('\u{000c}', "\n\n")
        .replace("\r\n", "\n")
        .replace('\r', "\n");

    let mut paragraphs: Vec<String> = Vec::new();
    for block in cleaned.split("\n\n") {
        let lines: Vec<&str> = block
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .collect();
        if !lines.is_empty() {
            paragraphs.push(lines.join(" "));
        }
    }

    let result = paragraphs.join("\n\n");
    if result.trim().is_empty() {
        return Err(
            "PDF-filen innehåller ingen markerbar text (det kan vara en inskannad bild-PDF)."
                .to_string(),
        );
    }

    Ok(result)
}

// ── Äkta Word (.docx) OpenXML-export av genererade mötesanteckningar ─────────

fn escape_xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 16);
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if c.is_control() && c != '\t' && c != '\n' => {}
            c => out.push(c),
        }
    }
    out
}

#[derive(Debug)]
enum InlineSpan {
    Plain(String),
    Bold(String),
    Italic(String),
    Code(String),
}

fn parse_inline_spans(line: &str) -> Vec<InlineSpan> {
    let mut spans = Vec::new();
    let chars: Vec<char> = line.chars().collect();
    let n = chars.len();
    let mut i = 0usize;
    let mut buf = String::new();

    while i < n {
        // **fetstil**
        if i + 1 < n && chars[i] == '*' && chars[i + 1] == '*' {
            if let Some(rel_end) = find_double_star(&chars[i + 2..]) {
                if rel_end > 0 {
                    if !buf.is_empty() {
                        spans.push(InlineSpan::Plain(std::mem::take(&mut buf)));
                    }
                    let inner: String = chars[i + 2..i + 2 + rel_end].iter().collect();
                    spans.push(InlineSpan::Bold(inner));
                    i += 4 + rel_end;
                    continue;
                }
            }
        }

        // `kod`
        if chars[i] == '`' {
            if let Some(rel_end) = chars[i + 1..].iter().position(|&c| c == '`') {
                if rel_end > 0 {
                    if !buf.is_empty() {
                        spans.push(InlineSpan::Plain(std::mem::take(&mut buf)));
                    }
                    let inner: String = chars[i + 1..i + 1 + rel_end].iter().collect();
                    spans.push(InlineSpan::Code(inner));
                    i += 2 + rel_end;
                    continue;
                }
            }
        }

        // *kursiv*
        if chars[i] == '*' && (i + 1 < n && !chars[i + 1].is_whitespace()) {
            if let Some(rel_end) = chars[i + 1..].iter().position(|&c| c == '*') {
                if rel_end > 0 && !chars[i + rel_end].is_whitespace() {
                    if !buf.is_empty() {
                        spans.push(InlineSpan::Plain(std::mem::take(&mut buf)));
                    }
                    let inner: String = chars[i + 1..i + 1 + rel_end].iter().collect();
                    spans.push(InlineSpan::Italic(inner));
                    i += 2 + rel_end;
                    continue;
                }
            }
        }

        buf.push(chars[i]);
        i += 1;
    }

    if !buf.is_empty() {
        spans.push(InlineSpan::Plain(buf));
    }
    spans
}

fn find_double_star(slice: &[char]) -> Option<usize> {
    if slice.len() < 2 {
        return None;
    }
    for idx in 0..(slice.len() - 1) {
        if slice[idx] == '*' && slice[idx + 1] == '*' {
            return Some(idx);
        }
    }
    None
}

fn render_inline_runs_xml(text: &str) -> String {
    let mut xml = String::new();
    for span in parse_inline_spans(text) {
        match span {
            InlineSpan::Plain(s) => {
                xml.push_str(&format!(
                    r#"<w:r><w:t xml:space="preserve">{}</w:t></w:r>"#,
                    escape_xml(&s)
                ));
            }
            InlineSpan::Bold(s) => {
                xml.push_str(&format!(
                    r#"<w:r><w:rPr><w:b/></w:rPr><w:t xml:space="preserve">{}</w:t></w:r>"#,
                    escape_xml(&s)
                ));
            }
            InlineSpan::Italic(s) => {
                xml.push_str(&format!(
                    r#"<w:r><w:rPr><w:i/></w:rPr><w:t xml:space="preserve">{}</w:t></w:r>"#,
                    escape_xml(&s)
                ));
            }
            InlineSpan::Code(s) => {
                xml.push_str(&format!(
                    r#"<w:r><w:rPr><w:rFonts w:ascii="Consolas" w:hAnsi="Consolas"/><w:sz w:val="20"/></w:rPr><w:t xml:space="preserve">{}</w:t></w:r>"#,
                    escape_xml(&s)
                ));
            }
        }
    }
    xml
}

/// Skapar en komplett, professionellt formaterad Word-fil (`.docx` OpenXML) i minnet
/// från mötestitel, valfri metadata-rad (t.ex. mallnamn + datum) och Markdown-dokumentet.
pub fn export_markdown_to_docx_bytes(
    title: &str,
    subtitle: Option<&str>,
    markdown: &str,
) -> Result<Vec<u8>, String> {
    let mut body_xml = String::with_capacity(markdown.len() * 3 + 1024);

    let clean_title = title.trim();
    if !clean_title.is_empty() {
        body_xml.push_str(&format!(
            r#"<w:p><w:pPr><w:pStyle w:val="Title"/></w:pPr><w:r><w:t xml:space="preserve">{}</w:t></w:r></w:p>"#,
            escape_xml(clean_title)
        ));
    }

    if let Some(sub) = subtitle.map(str::trim).filter(|s| !s.is_empty()) {
        body_xml.push_str(&format!(
            r#"<w:p><w:pPr><w:pStyle w:val="Subtitle"/></w:pPr><w:r><w:t xml:space="preserve">{}</w:t></w:r></w:p>"#,
            escape_xml(sub)
        ));
    }

    let normalized = markdown.replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<&str> = normalized.lines().collect();
    let mut i = 0usize;

    while i < lines.len() {
        let raw_line = lines[i];
        let trimmed = raw_line.trim();

        if trimmed.is_empty() {
            i += 1;
            continue;
        }

        // Kodblock ```
        if trimmed.starts_with("```") {
            i += 1;
            while i < lines.len() && !lines[i].trim().starts_with("```") {
                body_xml.push_str(&format!(
                    r#"<w:p><w:pPr><w:pStyle w:val="CodeBlock"/></w:pPr><w:r><w:t xml:space="preserve">{}</w:t></w:r></w:p>"#,
                    escape_xml(lines[i])
                ));
                i += 1;
            }
            if i < lines.len() {
                i += 1;
            }
            continue;
        }

        // Horisontell avdelare ---
        if trimmed.len() >= 3
            && (trimmed.chars().all(|c| c == '-')
                || trimmed.chars().all(|c| c == '*')
                || trimmed.chars().all(|c| c == '_'))
        {
            body_xml.push_str(
                r#"<w:p><w:pPr><w:pBdr><w:bottom w:val="single" w:sz="6" w:space="1" w:color="CBD5E1"/></w:pBdr></w:pPr></w:p>"#,
            );
            i += 1;
            continue;
        }

        // Rubriker # .. ####
        if let Some(stripped_hashes) = trimmed.strip_prefix('#') {
            let hash_count = 1 + stripped_hashes.chars().take_while(|&c| c == '#').count();
            let heading_text = trimmed[hash_count..].trim();
            if !heading_text.is_empty() {
                let style = match hash_count {
                    1 => "Heading1",
                    2 => "Heading2",
                    _ => "Heading3",
                };
                body_xml.push_str(&format!(
                    r#"<w:p><w:pPr><w:pStyle w:val="{style}"/></w:pPr>{}</w:p>"#,
                    render_inline_runs_xml(heading_text)
                ));
                i += 1;
                continue;
            }
        }

        // Fristående fetstilt rubrikrad: **Rubrik**
        if trimmed.starts_with("**")
            && trimmed.ends_with("**")
            && trimmed.len() > 4
            && !trimmed[2..trimmed.len() - 2].contains("**")
        {
            let inner = &trimmed[2..trimmed.len() - 2];
            body_xml.push_str(&format!(
                r#"<w:p><w:pPr><w:pStyle w:val="Heading2"/></w:pPr><w:r><w:t xml:space="preserve">{}</w:t></w:r></w:p>"#,
                escape_xml(inner)
            ));
            i += 1;
            continue;
        }

        // Checklista (- [ ] eller - [x]) och punktlista (- / * / •)
        if let Some(bullet_body) = strip_bullet_prefix(raw_line) {
            let formatted = if let Some(rest) = bullet_body.strip_prefix("[ ] ") {
                format!("☐ {rest}")
            } else if let Some(rest) = bullet_body
                .strip_prefix("[x] ")
                .or_else(|| bullet_body.strip_prefix("[X] "))
            {
                format!("☑ {rest}")
            } else {
                bullet_body.to_string()
            };
            body_xml.push_str(&format!(
                r#"<w:p><w:pPr><w:pStyle w:val="ListParagraph"/><w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr></w:pPr>{}</w:p>"#,
                render_inline_runs_xml(&formatted)
            ));
            i += 1;
            continue;
        }

        // Numrerad lista (1. / 1))
        if let Some(num_body) = strip_numbered_prefix(raw_line) {
            body_xml.push_str(&format!(
                r#"<w:p><w:pPr><w:pStyle w:val="ListParagraph"/><w:numPr><w:ilvl w:val="0"/><w:numId w:val="2"/></w:numPr></w:pPr>{}</w:p>"#,
                render_inline_runs_xml(num_body)
            ));
            i += 1;
            continue;
        }

        // Citatblock (> )
        if let Some(q) = trimmed.strip_prefix('>') {
            body_xml.push_str(&format!(
                r#"<w:p><w:pPr><w:pStyle w:val="Quote"/></w:pPr>{}</w:p>"#,
                render_inline_runs_xml(q.trim_start())
            ));
            i += 1;
            continue;
        }

        // Vanligt brödtextstycke
        let mut para_lines = Vec::new();
        while i < lines.len() {
            let l = lines[i];
            let lt = l.trim();
            if lt.is_empty()
                || lt.starts_with('#')
                || lt.starts_with("```")
                || lt.starts_with('>')
                || strip_bullet_prefix(l).is_some()
                || strip_numbered_prefix(l).is_some()
            {
                break;
            }
            para_lines.push(lt);
            i += 1;
        }

        let joined_para = para_lines.join(" ");
        body_xml.push_str(&format!(
            r#"<w:p>{}</w:p>"#,
            render_inline_runs_xml(&joined_para)
        ));
    }

    // A4-sidlayout med luftiga skandinaviska marginaler (2,5 cm)
    body_xml.push_str(
        r#"<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1417" w:right="1417" w:bottom="1417" w:left="1417" w:header="708" w:footer="708" w:gutter="0"/></w:sectPr>"#,
    );

    let document_xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:body>{body_xml}</w:body>
</w:document>"#
    );

    let content_types_xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
  <Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/>
  <Override PartName="/word/numbering.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml"/>
</Types>"#;

    let root_rels_xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>"#;

    let doc_rels_xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering" Target="numbering.xml"/>
</Relationships>"#;

    let styles_xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:docDefaults>
    <w:rPrDefault>
      <w:rPr>
        <w:rFonts w:ascii="Aptos" w:hAnsi="Aptos" w:cs="Calibri"/>
        <w:sz w:val="22"/>
        <w:szCs w:val="22"/>
        <w:color w:val="0F172A"/>
        <w:lang w:val="sv-SE"/>
      </w:rPr>
    </w:rPrDefault>
    <w:pPrDefault>
      <w:pPr>
        <w:spacing w:after="140" w:line="276" w:lineRule="auto"/>
      </w:pPr>
    </w:pPrDefault>
  </w:docDefaults>
  <w:style w:type="paragraph" w:default="1" w:styleId="Normal">
    <w:name w:val="Normal"/>
  </w:style>
  <w:style w:type="paragraph" w:styleId="Title">
    <w:name w:val="Title"/>
    <w:basedOn w:val="Normal"/>
    <w:pPr><w:spacing w:before="0" w:after="80"/></w:pPr>
    <w:rPr><w:b/><w:sz w:val="40"/><w:color w:val="1E1B4B"/></w:rPr>
  </w:style>
  <w:style w:type="paragraph" w:styleId="Subtitle">
    <w:name w:val="Subtitle"/>
    <w:basedOn w:val="Normal"/>
    <w:pPr><w:spacing w:before="0" w:after="240"/><w:pBdr><w:bottom w:val="single" w:sz="6" w:space="6" w:color="CBD5E1"/></w:pBdr></w:pPr>
    <w:rPr><w:sz w:val="19"/><w:color w:val="475569"/></w:rPr>
  </w:style>
  <w:style w:type="paragraph" w:styleId="Heading1">
    <w:name w:val="heading 1"/>
    <w:basedOn w:val="Normal"/>
    <w:pPr><w:keepNext/><w:spacing w:before="280" w:after="100"/></w:pPr>
    <w:rPr><w:b/><w:sz w:val="30"/><w:color w:val="1E293B"/></w:rPr>
  </w:style>
  <w:style w:type="paragraph" w:styleId="Heading2">
    <w:name w:val="heading 2"/>
    <w:basedOn w:val="Normal"/>
    <w:pPr><w:keepNext/><w:spacing w:before="240" w:after="80"/></w:pPr>
    <w:rPr><w:b/><w:sz w:val="26"/><w:color w:val="312E81"/></w:rPr>
  </w:style>
  <w:style w:type="paragraph" w:styleId="Heading3">
    <w:name w:val="heading 3"/>
    <w:basedOn w:val="Normal"/>
    <w:pPr><w:keepNext/><w:spacing w:before="200" w:after="60"/></w:pPr>
    <w:rPr><w:b/><w:sz w:val="23"/><w:color w:val="1E293B"/></w:rPr>
  </w:style>
  <w:style w:type="paragraph" w:styleId="ListParagraph">
    <w:name w:val="List Paragraph"/>
    <w:basedOn w:val="Normal"/>
    <w:pPr><w:ind w:left="540" w:hanging="300"/><w:spacing w:after="80" w:line="264" w:lineRule="auto"/></w:pPr>
  </w:style>
  <w:style w:type="paragraph" w:styleId="Quote">
    <w:name w:val="Quote"/>
    <w:basedOn w:val="Normal"/>
    <w:pPr><w:ind w:left="480"/><w:pBdr><w:left w:val="single" w:sz="12" w:space="10" w:color="6366F1"/></w:pBdr></w:pPr>
    <w:rPr><w:i/><w:color w:val="334155"/></w:rPr>
  </w:style>
  <w:style w:type="paragraph" w:styleId="CodeBlock">
    <w:name w:val="Code Block"/>
    <w:basedOn w:val="Normal"/>
    <w:pPr><w:spacing w:after="40" w:line="240" w:lineRule="auto"/></w:pPr>
    <w:rPr><w:rFonts w:ascii="Consolas" w:hAnsi="Consolas"/><w:sz w:val="19"/></w:rPr>
  </w:style>
</w:styles>"#;

    let numbering_xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:abstractNum w:abstractNumId="0">
    <w:lvl w:ilvl="0">
      <w:start w:val="1"/>
      <w:numFmt w:val="bullet"/>
      <w:lvlText w:val="•"/>
      <w:lvlJc w:val="left"/>
      <w:pPr><w:ind w:left="540" w:hanging="300"/></w:pPr>
    </w:lvl>
  </w:abstractNum>
  <w:abstractNum w:abstractNumId="1">
    <w:lvl w:ilvl="0">
      <w:start w:val="1"/>
      <w:numFmt w:val="decimal"/>
      <w:lvlText w:val="%1."/>
      <w:lvlJc w:val="left"/>
      <w:pPr><w:ind w:left="540" w:hanging="300"/></w:pPr>
    </w:lvl>
  </w:abstractNum>
  <w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>
  <w:num w:numId="2"><w:abstractNumId w:val="1"/></w:num>
</w:numbering>"#;

    let mut out_buf: Vec<u8> = Vec::with_capacity(16_384);
    {
        let cursor = Cursor::new(&mut out_buf);
        let mut zip = ZipWriter::new(cursor);
        let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

        for (path, content) in [
            ("[Content_Types].xml", content_types_xml),
            ("_rels/.rels", root_rels_xml),
            ("word/_rels/document.xml.rels", doc_rels_xml),
            ("word/styles.xml", styles_xml),
            ("word/numbering.xml", numbering_xml),
            ("word/document.xml", document_xml.as_str()),
        ] {
            zip.start_file(path, opts).map_err(|e| e.to_string())?;
            zip.write_all(content.as_bytes()).map_err(|e| e.to_string())?;
        }

        zip.finish().map_err(|e| e.to_string())?;
    }

    Ok(out_buf)
}

fn strip_bullet_prefix(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    for p in ["- ", "* ", "• "] {
        if let Some(rest) = trimmed.strip_prefix(p) {
            return Some(rest.trim());
        }
    }
    None
}

fn strip_numbered_prefix(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let digits = trimmed.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 || digits > 4 {
        return None;
    }
    let after = &trimmed[digits..];
    if let Some(rest) = after.strip_prefix(". ").or_else(|| after.strip_prefix(") ")) {
        return Some(rest.trim());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exports_markdown_to_valid_docx_and_reimports_swedish_content() {
        let md = "## Sammanfattning\n\nStyrgruppen beslutade att lansera Loki 3.0 i oktober.\n\n### Åtgärder\n- [ ] Uppdatera intranätet — **Anna Lindqvist**\n- [x] Verifiera lokal modell på Windows och Mac\n1. Första milstolpen klar";
        let docx_bytes = export_markdown_to_docx_bytes(
            "Styrgruppsmöte Digitalisering",
            Some("Mötesprotokoll · 2026-11-12"),
            md,
        )
        .expect("skapa docx");

        let reimported =
            import_document_bytes("protokoll.docx", &docx_bytes, false).expect("läs tillbaka docx");
        assert_eq!(reimported.format, "docx");
        assert!(reimported.text.contains("Styrgruppsmöte Digitalisering"));
        assert!(reimported.text.contains("Sammanfattning"));
        assert!(reimported
            .text
            .contains("☐ Uppdatera intranätet — Anna Lindqvist"));
        assert!(reimported
            .text
            .contains("☑ Verifiera lokal modell på Windows och Mac"));
    }

    #[test]
    fn decodes_windows_1252_swedish_text_fallback() {
        // "Mötesåtgärd – Övrigt" i Windows-1252 (ö=0xF6, å=0xE5, ä=0xE4, –=0x96, Ö=0xD6)
        let cp1252: Vec<u8> = vec![
            b'M', 0xF6, b't', b'e', b's', 0xE5, b't', b'g', 0xE4, b'r', b'd', b' ', 0x96, b' ',
            0xD6, b'v', b'r', b'i', b'g', b't',
        ];
        let doc = import_document_bytes("anteckning.txt", &cp1252, false).unwrap();
        assert_eq!(doc.text, "Mötesåtgärd – Övrigt");
    }
}

