use std::borrow::Cow;
use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

static RE_INLINE_DATA_URI: OnceLock<Regex> = OnceLock::new();
static RE_CUE_TIMING: OnceLock<Regex> = OnceLock::new();
static RE_UUID_OR_CUE_ID: OnceLock<Regex> = OnceLock::new();
static RE_VTT_VOICE_OPEN: OnceLock<Regex> = OnceLock::new();
static RE_HTML_TAGS: OnceLock<Regex> = OnceLock::new();
static RE_BRACKET_TS_PREFIX: OnceLock<Regex> = OnceLock::new();
static RE_SPEAKER_WITH_TS_HEADER: OnceLock<Regex> = OnceLock::new();
static RE_SPEAKER_COLON: OnceLock<Regex> = OnceLock::new();

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptStats {
    pub raw_chars: usize,
    pub cleaned_chars: usize,
    pub chars_saved_percent: u8,
    pub word_count: usize,
    pub estimated_tokens: usize,
    pub approx_minutes: u32,
    pub was_cleaned: bool,
    pub consolidated_turns: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanedTranscript {
    pub text: String,
    pub stats: TranscriptStats,
}

fn strip_inline_data_uris(input: &str) -> Cow<'_, str> {
    if !input.contains(";base64,") {
        return Cow::Borrowed(input);
    }
    let re = RE_INLINE_DATA_URI.get_or_init(|| {
        Regex::new(r"data:[a-zA-Z0-9/+.-]+;base64,[A-Za-z0-9+/=\s]{64,}")
            .expect("valid data URI regex")
    });
    re.replace_all(input, "[bild]")
}

fn cue_timing_re() -> &'static Regex {
    RE_CUE_TIMING.get_or_init(|| {
        Regex::new(
            r"^\s*(?:\d{1,2}:)?\d{1,2}:\d{2}[.,]\d{1,3}\s*-->\s*(?:\d{1,2}:)?\d{1,2}:\d{2}[.,]\d{1,3}",
        )
        .expect("valid cue timing regex")
    })
}

fn uuid_or_cue_id_re() -> &'static Regex {
    RE_UUID_OR_CUE_ID.get_or_init(|| {
        Regex::new(r"^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}(?:/\d+-\d+)?$")
            .expect("valid uuid cue regex")
    })
}

fn vtt_voice_open_re() -> &'static Regex {
    RE_VTT_VOICE_OPEN.get_or_init(|| {
        Regex::new(r"<v(?:\.[^>\s]+)*\s+([^>]+)>").expect("valid vtt voice tag regex")
    })
}

fn html_tags_re() -> &'static Regex {
    RE_HTML_TAGS
        .get_or_init(|| Regex::new(r"</?[a-zA-Z0-9:.#_-]+(?:\s+[^>]*)?>").expect("valid tag regex"))
}

fn bracket_ts_prefix_re() -> &'static Regex {
    RE_BRACKET_TS_PREFIX.get_or_init(|| {
        Regex::new(r"^\s*[\[\(]?(?:\d{1,2}:)?\d{1,2}:\d{2}(?:[.,]\d{1,3})?[\]\)]?\s+(.*)$")
            .expect("valid bracket ts regex")
    })
}

/// Matchar t.ex. Microsoft Teams Word-transkript: "Anna Lindqvist   0:04" eller "Erik Sjöberg  12:31"
fn speaker_with_ts_header_re() -> &'static Regex {
    RE_SPEAKER_WITH_TS_HEADER.get_or_init(|| {
        Regex::new(
            r"^([A-ZÅÄÖÉÜ][A-Za-zÅÄÖåäöÉéÜü.\-']+(?:\s+[A-ZÅÄÖÉÜa-zåäö.\-']+){0,4})\s{2,}(?:\d{1,2}:)?\d{1,2}:\d{2}(?::\d{2})?\s*$",
        )
        .expect("valid speaker header regex")
    })
}

fn speaker_colon_re() -> &'static Regex {
    RE_SPEAKER_COLON.get_or_init(|| {
        Regex::new(
            r"^([A-ZÅÄÖÉÜ][A-Za-zÅÄÖåäöÉéÜü0-9.\-']+(?:\s+[A-Za-zÅÄÖåäöÉéÜü0-9.\-']+){0,3}):\s+(.+)$",
        )
        .expect("valid speaker colon regex")
    })
}

fn is_pure_cue_number(line: &str) -> bool {
    let t = line.trim();
    !t.is_empty() && t.len() <= 7 && t.chars().all(|c| c.is_ascii_digit())
}

/// Deterministisk Lager 0-städning (< 5 ms) av mötestranskriberingar (.vtt, .srt, Teams, Zoom, råtext).
/// Tar bort tidsstämplar, cue-ID/UUID och XML-taggar samt slår ihop intilliggande repliker från samma talare.
pub fn clean_transcript(raw: &str, file_ext: Option<&str>) -> CleanedTranscript {
    let without_bom = raw.strip_prefix('\u{feff}').unwrap_or(raw);
    let raw_chars = without_bom.chars().count();
    if without_bom.trim().is_empty() {
        return CleanedTranscript {
            text: String::new(),
            stats: TranscriptStats {
                raw_chars: 0,
                cleaned_chars: 0,
                chars_saved_percent: 0,
                word_count: 0,
                estimated_tokens: 0,
                approx_minutes: 0,
                was_cleaned: false,
                consolidated_turns: 0,
            },
        };
    }

    let stripped_b64 = strip_inline_data_uris(without_bom);
    let normalized = stripped_b64.replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<&str> = normalized.lines().collect();

    let ext_lower = file_ext.unwrap_or("").to_ascii_lowercase();
    let has_cue_arrows = lines.iter().take(60).any(|l| cue_timing_re().is_match(l));
    let starts_with_webvtt = lines
        .first()
        .map(|l| l.trim().starts_with("WEBVTT"))
        .unwrap_or(false);
    let has_teams_headers = lines
        .iter()
        .take(60)
        .filter(|l| speaker_with_ts_header_re().is_match(l.trim()))
        .count()
        >= 2;
    let is_subtitle_mode =
        ext_lower == "vtt" || ext_lower == "srt" || starts_with_webvtt || has_cue_arrows;

    if !is_subtitle_mode && !has_teams_headers {
        // Vanlig text / mötesanteckning: städa eventuella tidsstämpelprefix [00:12:34] och slå ihop påföljande identiska talarrader
        let cleaned = clean_plain_or_speaker_text(&lines);
        return finalize_stats(raw_chars, cleaned.0, cleaned.1 > 0, cleaned.1);
    }

    // VTT / SRT / Teams-transkriptläge
    let mut turns: Vec<(Option<String>, String)> = Vec::new();
    let mut active_header_speaker: Option<String> = None;
    let mut skip_metadata_block = false;
    let mut consolidated_turns = 0usize;

    for idx in 0..lines.len() {
        let trimmed = lines[idx].trim();

        if trimmed.is_empty() {
            skip_metadata_block = false;
            continue;
        }

        if skip_metadata_block {
            continue;
        }

        if trimmed.starts_with("WEBVTT") {
            continue;
        }

        if is_subtitle_mode && (trimmed.starts_with("NOTE") || trimmed.starts_with("STYLE")) {
            skip_metadata_block = true;
            continue;
        }

        if cue_timing_re().is_match(trimmed) || uuid_or_cue_id_re().is_match(trimmed) {
            continue;
        }

        // Hoppa över fristående cue-nummer om nästa icke-tomma rad är en tidsstämpel
        if is_subtitle_mode && is_pure_cue_number(trimmed) {
            let next_non_empty = lines[idx + 1..].iter().find(|l| !l.trim().is_empty());
            if next_non_empty
                .map(|nl| cue_timing_re().is_match(nl.trim()))
                .unwrap_or(true)
            {
                continue;
            }
        }

        // Microsoft Teams Word-rubrik: "Anna Lindqvist   0:04"
        if let Some(caps) = speaker_with_ts_header_re().captures(trimmed) {
            active_header_speaker = Some(caps[1].trim().to_string());
            continue;
        }

        let mut working = trimmed.to_string();
        let mut line_speaker: Option<String> = None;

        // WebVTT <v Talare>Text</v>
        if let Some(caps) = vtt_voice_open_re().captures(&working) {
            line_speaker = Some(caps[1].trim().to_string());
            working = vtt_voice_open_re().replace(&working, "").into_owned();
        }

        // Ta bort kvarvarande VTT/HTML-taggar (<c>, </v>, <00:01:02.000>)
        if working.contains('<') && working.contains('>') {
            working = html_tags_re().replace_all(&working, "").into_owned();
        }

        // Ta bort inledande tidsstämpel på raden, t.ex. "[00:04:12] Erik: ..."
        if let Some(caps) = bracket_ts_prefix_re().captures(&working) {
            let rest = caps[1].trim();
            if !rest.is_empty() {
                working = rest.to_string();
            }
        }

        let working_trimmed = working.trim();
        if working_trimmed.is_empty() {
            continue;
        }

        // Kontrollera "Talare: replik" om vi inte redan fick talare från <v ...>
        let (speaker, utterance) = if let Some(spk) = line_speaker {
            (Some(spk), working_trimmed.to_string())
        } else if let Some(caps) = speaker_colon_re().captures(working_trimmed) {
            active_header_speaker = None;
            (
                Some(caps[1].trim().to_string()),
                caps[2].trim().to_string(),
            )
        } else if let Some(ref hdr_spk) = active_header_speaker {
            (Some(hdr_spk.clone()), working_trimmed.to_string())
        } else {
            (None, working_trimmed.to_string())
        };

        push_or_merge_turn(&mut turns, speaker, &utterance, &mut consolidated_turns);
    }

    let mut out_lines: Vec<String> = Vec::with_capacity(turns.len());
    for (spk, text) in turns {
        if let Some(s) = spk {
            out_lines.push(format!("{s}: {text}"));
        } else {
            out_lines.push(text);
        }
    }

    let final_text = out_lines.join("\n\n");
    finalize_stats(raw_chars, final_text, true, consolidated_turns)
}

fn clean_plain_or_speaker_text(lines: &[&str]) -> (String, usize) {
    let mut out_blocks: Vec<String> = Vec::new();
    let mut turns: Vec<(Option<String>, String)> = Vec::new();
    let mut consolidated = 0usize;

    // Avgör om texten huvudsakligen består av "[00:01:12] Talare: ..." eller "Talare: ..."
    let speaker_like_count = lines
        .iter()
        .take(50)
        .filter(|l| {
            let t = l.trim();
            !t.is_empty()
                && (bracket_ts_prefix_re().is_match(t) || speaker_colon_re().is_match(t))
        })
        .count();

    if speaker_like_count >= 3 {
        for raw_line in lines {
            let mut trimmed = raw_line.trim().to_string();
            if trimmed.is_empty() {
                continue;
            }
            if let Some(caps) = bracket_ts_prefix_re().captures(&trimmed) {
                let rest = caps[1].trim();
                if !rest.is_empty() {
                    trimmed = rest.to_string();
                    consolidated += 1;
                }
            }
            if let Some(caps) = speaker_colon_re().captures(&trimmed) {
                push_or_merge_turn(
                    &mut turns,
                    Some(caps[1].trim().to_string()),
                    caps[2].trim(),
                    &mut consolidated,
                );
            } else {
                push_or_merge_turn(&mut turns, None, &trimmed, &mut consolidated);
            }
        }
        for (spk, txt) in turns {
            if let Some(s) = spk {
                out_blocks.push(format!("{s}: {txt}"));
            } else {
                out_blocks.push(txt);
            }
        }
        return (out_blocks.join("\n\n"), consolidated);
    }

    // Annars behåll naturlig styckeindelning (t.ex. kopierade anteckningar eller protokollunderlag)
    let mut result = String::with_capacity(lines.len() * 48);
    let mut consecutive_blank = 0usize;
    for line in lines {
        let trimmed_end = line.trim_end();
        if trimmed_end.trim().is_empty() {
            consecutive_blank += 1;
            if consecutive_blank <= 1 && !result.is_empty() {
                result.push('\n');
            }
        } else {
            if !result.is_empty() {
                result.push('\n');
            }
            consecutive_blank = 0;
            result.push_str(trimmed_end);
        }
    }
    (result.trim().to_string(), 0)
}

fn push_or_merge_turn(
    turns: &mut Vec<(Option<String>, String)>,
    speaker: Option<String>,
    utterance: &str,
    consolidated_turns: &mut usize,
) {
    let clean_utt = utterance.split_whitespace().collect::<Vec<_>>().join(" ");
    if clean_utt.is_empty() {
        return;
    }

    if let Some((last_spk, last_txt)) = turns.last_mut() {
        let same_speaker = match (last_spk.as_deref(), speaker.as_deref()) {
            (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
            (None, None) => true,
            _ => false,
        };
        if same_speaker {
            // Undvik dubbelreplik som ofta uppstår vid överlappande undertexter
            if !last_txt.ends_with(&clean_utt) {
                if !last_txt.ends_with(' ') {
                    last_txt.push(' ');
                }
                last_txt.push_str(&clean_utt);
            }
            *consolidated_turns += 1;
            return;
        }
    }

    turns.push((speaker, clean_utt));
}

fn finalize_stats(
    raw_chars: usize,
    cleaned_text: String,
    was_cleaned: bool,
    consolidated_turns: usize,
) -> CleanedTranscript {
    let cleaned_chars = cleaned_text.chars().count();
    let saved = if raw_chars > cleaned_chars && raw_chars > 0 {
        (((raw_chars - cleaned_chars) as f64 / raw_chars as f64) * 100.0).round() as u8
    } else {
        0
    };
    let word_count = cleaned_text.split_whitespace().count();
    // Kalibrerad svensk tokenuppskattning för Gemma 4 (262k vokabulär ≈ 3,7 tecken/token på svenska)
    let estimated_tokens = if cleaned_chars == 0 {
        0
    } else {
        ((cleaned_chars as f64) / 3.7).ceil() as usize
    };
    let approx_minutes = if word_count == 0 {
        0
    } else {
        ((word_count as f64) / 135.0).round().max(1.0) as u32
    };

    CleanedTranscript {
        text: cleaned_text,
        stats: TranscriptStats {
            raw_chars,
            cleaned_chars,
            chars_saved_percent: saved,
            word_count,
            estimated_tokens,
            approx_minutes,
            was_cleaned: was_cleaned || saved > 0,
            consolidated_turns,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_teams_webvtt_and_consolidates_consecutive_speaker_turns() {
        let vtt = "\u{feff}WEBVTT

f4a1b2c3-1111-2222-3333-444455556666/12-0
00:00:01.120 --> 00:00:04.500
<v Anna Lindqvist>Hej allihopa, hörs jag bra?</v>

f4a1b2c3-1111-2222-3333-444455556666/12-1
00:00:04.600 --> 00:00:09.200
<v Anna Lindqvist>Toppen, då öppnar vi mötet om Q3-budgeten.</v>

f4a1b2c3-1111-2222-3333-444455556666/13-0
00:00:09.800 --> 00:00:14.000
<v Erik Sjöberg>Vi ser att licenskostnaderna sjönk med 12 procent.</v>
";

        let res = clean_transcript(vtt, Some("vtt"));
        assert!(res.stats.was_cleaned);
        assert!(res.stats.chars_saved_percent > 30);
        assert_eq!(res.stats.consolidated_turns, 1);
        assert!(!res.text.contains("WEBVTT"));
        assert!(!res.text.contains("-->"));
        assert!(!res.text.contains("f4a1b2c3"));
        assert!(res.text.contains(
            "Anna Lindqvist: Hej allihopa, hörs jag bra? Toppen, då öppnar vi mötet om Q3-budgeten."
        ));
        assert!(res
            .text
            .contains("Erik Sjöberg: Vi ser att licenskostnaderna sjönk med 12 procent."));
    }

    #[test]
    fn cleans_srt_subtitles_and_preserves_content() {
        let srt = "1\r\n00:00:01,000 --> 00:00:03,500\r\nVälkomna till styrgruppen.\r\n\r\n2\r\n00:00:03,600 --> 00:00:07,200\r\nIdag ska vi fatta beslut om pilotprojektet.\r\n";
        let res = clean_transcript(srt, Some("srt"));
        assert!(!res.text.contains("-->"));
        assert_eq!(
            res.text,
            "Välkomna till styrgruppen. Idag ska vi fatta beslut om pilotprojektet."
        );
    }

    #[test]
    fn cleans_teams_docx_timestamp_headers() {
        let teams_txt = "Anna Lindqvist   0:04\nVi börjar med statusrundan.\n\nAnna Lindqvist   0:19\nAlla leveranser ligger i fas.\n\nKarin Berg   0:42\nJag tar ansvar för uppföljningen på fredag.";
        let res = clean_transcript(teams_txt, None);
        assert!(res
            .text
            .contains("Anna Lindqvist: Vi börjar med statusrundan. Alla leveranser ligger i fas."));
        assert!(res
            .text
            .contains("Karin Berg: Jag tar ansvar för uppföljningen på fredag."));
    }
}

