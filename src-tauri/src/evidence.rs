use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const EVIDENCE_CACHE_SCHEMA: &str = "loki-evidence-v1";
const MAX_ROLLING_TERMS: usize = 26;
const MAX_RECENT_TOPICS: usize = 4;

// ── Kompakta prompter för Pass 1 (Bevisextrahering) & Pass 2 (Syntes/Temakodning) ──

pub const EVIDENCE_SYSTEM_PROMPT: &str = "\
Du är en kirurgiskt noggrann dataextraherare för svenska mötes- och workshoptranskriberingar från automatisk röstigenkänning (STT).\n\n\
VIKTIGASTE REGELN: Du får INTE skriva en utslätad sammanfattning eller svepande floskler. Din uppgift är att plocka ut konkreta, faktatäta byggstenar ur avsnittet:\n\
- KONTEXTUELL STT-STÄDNING: Texten har hörselfel och engelska facktermer stavade fonetiskt på svenska. Tolkas ett ord logiskt ur sammanhanget, använd den korrekta termen (och notera gärna rättelsen under [DELTAGARE_OCH_TERMER]). Gissa aldrig fritt och hitta ALDRIG på fakta, namn, datum eller siffror.\n\
- BEVARA MOTSÄTTNINGAR: Släta aldrig över oenigheter! Om deltagare säger emot varandra, tvekar eller nämner ett olöst problem i förbifarten MÅSTE det fångas under [FRIKTION_OCH_RISKER].\n\
- FILTRERA KALLPRAT: Ignorera helt småprat om väder, helg, kaffe, kameror och mikrofoner. Om hela avsnittet ENBART är kallprat/teknikstrul utan sakfrågor, svara exakt: [INGET_RELEVANT]\n\n\
Svara ENDAST med följande rubrikblock i exakt denna ordning (skriv \"- Inga\" under en rubrik som saknar innehåll i avsnittet, inga inledande eller avslutande kommentarer):\n\n\
[DELTAGARE_OCH_TERMER]\n\
- Namn (roll om nämnd) eller Hörselfel -> Rätt fackterm\n\
[ÄMNEN]\n\
- Ämnesnamn: Konkreta sakuppgifter, siffror, datum, system och förslag som lyftes.\n\
[BESLUT]\n\
- Konkret beslut som fattades (inklusive motivering om den nämndes).\n\
[ÅTGÄRDER]\n\
- Konkret uppgift — Ansvarig person (tidplan/deadline om nämnd).\n\
[FRIKTION_OCH_RISKER]\n\
- Oenighet, invändning, avvikande åsikt eller olöst risk/antagande (ange gärna vem som lyfte den).\n\
[NYCKELCITAT]\n\
- Talare: \"Kort ordagrant citat (max 25 ord, putsat från grova hörselfel) som fångar en viktig vändpunkt, invändning eller nyans.\"";

pub const THEME_MATRIX_SYSTEM_PROMPT: &str = "\
Du är expert på kvalitativ strukturering av långa möten och workshops.\n\
Du får en kronologisk översikt över alla extraherade avsnitt i ett långt möte. Din uppgift är att gruppera avsnitten i 4 till 8 sammanhängande huvudteman så att diskussioner om samma sakfråga (även om deltagarna återkom till ämnet långt senare i mötet) samlas under rätt tema.\n\n\
Svara ENDAST med rader i exakt detta format (en rad per tema, inga kommentarer före eller efter):\n\
[TEMA: Tydlig svensk rubrik | AVSNITT: 1, 2, 5]\n\n\
REGLER:\n\
- Varje avsnittsnummer i underlaget BÖR finnas med i minst ett tema.\n\
- Ett avsnitt som spänner över två skilda huvudteman får listas under båda.\n\
- Om en agenda bifogas, använd agendans punkter som temarubriker i agendans ordning (och lägg övriga ämnen under \"Övriga frågor\").";

pub const EVIDENCE_SYNTHESIS_SYSTEM_PROMPT: &str = "\
Du skapar ett slutgiltigt, professionellt svenskt mötesdokument utifrån strukturerade bevispaket som extraherats ur mötestranskriberingen.\n\n\
REGLER FÖR HÖGSTA KVALITET:\n\
- Hitta ALDRIG på fakta, namn, beslut, åtgärder eller siffror som inte finns i bevispaketen.\n\
- Bevara alla konkreta detaljer, siffror, datum, invändningar/friktionspunkter och viktiga nyanser som finns i bevispaketen – släta inte ut meningskiljaktigheter om mallen ger utrymme för diskussion.\n\
- Slå ihop dubbletter om samma beslut eller åtgärd fångats vid en avsnittsgräns.\n\
- Säkerställ att SAMTLIGA punkter i kontrollregistret för beslut och åtgärder kommer med där mallen efterfrågar beslut/åtgärder.\n\
- Leverera ENBART det färdiga dokumentet i ren Markdown utan inledande eller avslutande kommentarer.";

pub const SECTION_SYNTHESIS_SYSTEM_PROMPT: &str = "\
Du skriver ett avgränsat avsnitt (ett eller flera huvudteman) av ett längre svenskt mötesdokument utifrån de fullständiga bevispaketen för just dessa teman.\n\n\
REGLER:\n\
- Skriv ENBART de tilldelade temarubrikerna (`### Rubrik`) och deras innehåll – skriv ingen dokumenttitel (`#`), ingen övergripande inledning och ingen slutsummering.\n\
- Bevara konkreta sakuppgifter, siffror, motstridiga åsikter/friktionspunkter, nyckelcitat (om relevant för mallen) samt beslut/åtgärder under temat.\n\
- Hitta ALDRIG på fakta som inte står i bevispaketen. Upprepa inte sådant som redan avhandlats i listan över tidigare skrivna rubriker.";

// ── Datastrukturer ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EvidencePacket {
    pub chunk_index: usize,
    pub total_chunks: usize,
    pub is_empty: bool,
    pub participants_and_terms: Vec<String>,
    pub topics: Vec<String>,
    pub decisions: Vec<String>,
    pub actions: Vec<String>,
    pub frictions_and_risks: Vec<String>,
    pub quotes: Vec<String>,
}

impl EvidencePacket {
    pub fn has_substance(&self) -> bool {
        !self.is_empty
            && (!self.topics.is_empty()
                || !self.decisions.is_empty()
                || !self.actions.is_empty()
                || !self.frictions_and_risks.is_empty()
                || !self.quotes.is_empty())
    }

    /// Kompakt representation för Pass 2 (Syntes) — hög faktatäthet utan onödiga brusord.
    pub fn format_for_synthesis(&self) -> String {
        if !self.has_substance() {
            return String::new();
        }
        let mut out = format!("=== AVSNITT {} av {} ===\n", self.chunk_index, self.total_chunks);
        if !self.topics.is_empty() {
            out.push_str("Ämnen & sakfakta:\n");
            for t in &self.topics {
                out.push_str("  - ");
                out.push_str(t);
                out.push('\n');
            }
        }
        if !self.decisions.is_empty() {
            out.push_str("Beslut i avsnittet:\n");
            for d in &self.decisions {
                out.push_str("  - ");
                out.push_str(d);
                out.push('\n');
            }
        }
        if !self.actions.is_empty() {
            out.push_str("Åtgärder i avsnittet:\n");
            for a in &self.actions {
                out.push_str("  - ");
                out.push_str(a);
                out.push('\n');
            }
        }
        if !self.frictions_and_risks.is_empty() {
            out.push_str("Friktionspunkter, invändningar & risker:\n");
            for f in &self.frictions_and_risks {
                out.push_str("  - ");
                out.push_str(f);
                out.push('\n');
            }
        }
        if !self.quotes.is_empty() {
            out.push_str("Nyckelcitat:\n");
            for q in &self.quotes {
                out.push_str("  - ");
                out.push_str(q);
                out.push('\n');
            }
        }
        out
    }

    /// Extra kompakt rad för Pass 2A (Temainventering) så att även 50+ avsnitt ryms i ett litet 8K-fönster.
    pub fn format_for_theme_index(&self) -> String {
        if !self.has_substance() {
            return String::new();
        }
        let mut items: Vec<&str> = self.topics.iter().map(String::as_str).collect();
        if items.is_empty() {
            items.extend(self.decisions.iter().map(String::as_str));
            items.extend(self.frictions_and_risks.iter().map(String::as_str));
        }
        let summary = items.join(" ; ");
        format!("Avsnitt {}: {}", self.chunk_index, summary)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RollingMeetingState {
    pub participants_and_terms: Vec<String>,
    pub recent_topics: Vec<String>,
}

impl RollingMeetingState {
    pub fn update_from_packet(&mut self, pkt: &EvidencePacket) {
        if pkt.is_empty {
            return;
        }
        for entry in &pkt.participants_and_terms {
            let trimmed = entry.trim();
            if trimmed.is_empty() || trimmed.len() > 110 {
                continue;
            }
            let lower = trimmed.to_lowercase();
            let already = self
                .participants_and_terms
                .iter()
                .any(|existing| existing.to_lowercase() == lower);
            if !already && self.participants_and_terms.len() < MAX_ROLLING_TERMS {
                self.participants_and_terms.push(trimmed.to_string());
            }
        }

        if !pkt.topics.is_empty() {
            self.recent_topics = pkt
                .topics
                .iter()
                .take(MAX_RECENT_TOPICS)
                .map(|t| {
                    // Ta bara ämneshuvudet före ':' eller klipp vid 90 tecken så minneshuvudet förblir litet
                    let short = t.split(':').next().unwrap_or(t).trim();
                    truncate_chars(short, 90)
                })
                .collect();
        }
    }

    pub fn format_header(&self) -> String {
        if self.participants_and_terms.is_empty() && self.recent_topics.is_empty() {
            return String::new();
        }
        let mut s = String::from("--- RULLANDE MÖTESMINNE FRÅN TIDIGARE AVSNITT ---\n");
        if !self.participants_and_terms.is_empty() {
            s.push_str("Kända deltagare & uttydda termer (använd konsekvent stavning): ");
            s.push_str(&self.participants_and_terms.join("; "));
            s.push('\n');
        }
        if !self.recent_topics.is_empty() {
            s.push_str("Närmast föregående ämnen: ");
            s.push_str(&self.recent_topics.join(" | "));
            s.push('\n');
        }
        s.push_str("--- SLUT RULLANDE MÖTESMINNE ---\n\n");
        s
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThemeCluster {
    pub title: String,
    pub chunk_indices: Vec<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheManifest {
    schema: String,
    model_id: String,
    total_chunks: usize,
    created_unix_secs: u64,
}

// ── Diskbaserad Bevisjournal (`EvidenceDiskStore`) ───────────────────────────

pub struct EvidenceDiskStore {
    session_dir: PathBuf,
    total_chunks: usize,
}

impl EvidenceDiskStore {
    pub fn open(
        cache_root: &Path,
        model_id: &str,
        context: Option<&str>,
        total_chunks: usize,
        normalized_transcript: &str,
    ) -> Result<Self, String> {
        ensure_private_dir(cache_root)?;
        let key = compute_evidence_cache_key(
            model_id,
            context.unwrap_or(""),
            total_chunks,
            normalized_transcript,
        );
        let session_dir = cache_root.join(key);
        ensure_private_dir(&session_dir)?;

        let manifest_path = session_dir.join("manifest.json");
        if !manifest_path.exists() {
            let now_secs = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let manifest = CacheManifest {
                schema: EVIDENCE_CACHE_SCHEMA.to_string(),
                model_id: model_id.to_string(),
                total_chunks,
                created_unix_secs: now_secs,
            };
            if let Ok(json) = serde_json::to_vec_pretty(&manifest) {
                let _ = write_atomic(&manifest_path, &json);
            }
        }

        Ok(Self {
            session_dir,
            total_chunks,
        })
    }

    fn chunk_path(&self, chunk_index: usize) -> PathBuf {
        self.session_dir.join(format!("chunk_{chunk_index:04}.json"))
    }

    pub fn load_chunk(&self, chunk_index: usize) -> Option<EvidencePacket> {
        let p = self.chunk_path(chunk_index);
        let bytes = fs::read(p).ok()?;
        let pkt: EvidencePacket = serde_json::from_slice(&bytes).ok()?;
        if pkt.chunk_index == chunk_index && pkt.total_chunks == self.total_chunks {
            Some(pkt)
        } else {
            None
        }
    }

    pub fn save_chunk(&self, packet: &EvidencePacket) -> Result<(), String> {
        let p = self.chunk_path(packet.chunk_index);
        let bytes = serde_json::to_vec_pretty(packet)
            .map_err(|e| format!("Kunde inte serialisera bevispaket {}: {e}", packet.chunk_index))?;
        write_atomic(&p, &bytes)
    }

    pub fn load_all_chunks(&self) -> Result<Vec<EvidencePacket>, String> {
        let mut out = Vec::with_capacity(self.total_chunks);
        for idx in 1..=self.total_chunks {
            if let Some(pkt) = self.load_chunk(idx) {
                out.push(pkt);
            } else {
                return Err(format!(
                    "Saknar sparat bevispaket på disk för avsnitt {idx} av {}.",
                    self.total_chunks
                ));
            }
        }
        Ok(out)
    }
}

/// Rensar hela den lokala `evidence_cache`-katalogen på disk (anropas vid uppstart, nedstängning och historikrensning).
pub fn cleanup_all_evidence_cache(cache_root: &Path) {
    if cache_root.exists() {
        let _ = fs::remove_dir_all(cache_root);
    }
}

fn ensure_private_dir(dir: &Path) -> Result<(), String> {
    fs::create_dir_all(dir)
        .map_err(|e| format!("Kunde inte skapa lokal mellanlagringsmapp ({}): {e}", dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
    }
    Ok(())
}

fn write_atomic(dest: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = dest.with_extension("json.tmp");
    fs::write(&tmp, bytes)
        .map_err(|e| format!("Kunde inte skriva mellanlagringsfil ({}): {e}", tmp.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600));
    }
    if fs::rename(&tmp, dest).is_err() {
        fs::copy(&tmp, dest)
            .map_err(|e| format!("Kunde inte spara mellanlagringsfil ({}): {e}", dest.display()))?;
        let _ = fs::remove_file(&tmp);
    }
    Ok(())
}

// ── Uppdelning vid talar-/radbrytningar ──────────────────────────────────────

/// Delar upp en lång transkribering vid stycke-, rad- (talarbyte!) eller meningsgränser.
pub fn split_transcript_on_turns(
    text: &str,
    token_budget: usize,
    chars_per_token: f64,
    overlap_tokens: usize,
) -> Vec<String> {
    let char_budget = ((token_budget as f64) * chars_per_token).floor().max(600.0) as usize;
    let overlap_chars = ((overlap_tokens as f64) * chars_per_token).floor().max(0.0) as usize;

    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= char_budget {
        let trimmed = text.trim();
        return if trimmed.is_empty() {
            Vec::new()
        } else {
            vec![trimmed.to_string()]
        };
    }

    let mut chunks = Vec::new();
    let mut start = 0usize;
    while start < chars.len() {
        let mut end = (start + char_budget).min(chars.len());
        if end < chars.len() {
            let min_break = start + (char_budget * 62 / 100);
            let mut best_break = None;

            // 1. Styckebrytning (\n\n)
            for pos in (min_break..end).rev() {
                if pos + 1 < chars.len() && chars[pos] == '\n' && chars[pos + 1] == '\n' {
                    best_break = Some(pos + 1);
                    break;
                }
            }
            // 2. Radbrytning / nytt talarinlägg (\n)
            if best_break.is_none() {
                for pos in (min_break..end).rev() {
                    if chars[pos] == '\n' {
                        best_break = Some(pos + 1);
                        break;
                    }
                }
            }
            // 3. Meningsbrytning (. / ! / ?)
            if best_break.is_none() {
                for pos in (min_break..end).rev() {
                    if (chars[pos] == '.' || chars[pos] == '!' || chars[pos] == '?')
                        && pos + 1 < chars.len()
                        && chars[pos + 1].is_whitespace()
                    {
                        best_break = Some(pos + 1);
                        break;
                    }
                }
            }
            if let Some(b) = best_break {
                end = b;
            }
        }

        let slice_str: String = chars[start..end].iter().collect();
        let trimmed = slice_str.trim();
        if !trimmed.is_empty() {
            chunks.push(trimmed.to_string());
        }
        if end >= chars.len() {
            break;
        }
        if overlap_chars == 0 {
            start = end;
        } else {
            let mut raw_next = end.saturating_sub(overlap_chars).max(start + 1);
            // Snäpp start av överlapp till närmaste radbrytning framåt så vi inte klipper mitt i ett ord/namn
            let snap_limit = (raw_next + overlap_chars / 2).min(end);
            for pos in raw_next..snap_limit {
                if chars[pos] == '\n' && pos + 1 < end {
                    raw_next = pos + 1;
                    break;
                }
            }
            start = raw_next;
        }
    }

    chunks
}

// ── Parsning av Bevispaket & Temamatris ──────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
enum EvidenceSection {
    None,
    ParticipantsAndTerms,
    Topics,
    Decisions,
    Actions,
    FrictionsAndRisks,
    Quotes,
}

pub fn parse_evidence_packet(
    raw: &str,
    chunk_index: usize,
    total_chunks: usize,
) -> EvidencePacket {
    let trimmed = raw.trim();
    let lower_whole = trimmed.to_lowercase();
    if trimmed.is_empty()
        || lower_whole.contains("[inget_relevant]")
        || (trimmed.chars().count() < 80 && lower_whole.contains("inget relevant"))
    {
        return EvidencePacket {
            chunk_index,
            total_chunks,
            is_empty: true,
            ..Default::default()
        };
    }

    let mut pkt = EvidencePacket {
        chunk_index,
        total_chunks,
        is_empty: false,
        ..Default::default()
    };

    let mut section = EvidenceSection::None;
    for line in trimmed.lines() {
        let l = line.trim();
        if l.is_empty() {
            continue;
        }

        if let Some(detected) = detect_evidence_header(l) {
            section = detected;
            continue;
        }

        let cleaned = strip_list_marker(l);
        if is_none_placeholder(cleaned) {
            continue;
        }

        match section {
            EvidenceSection::ParticipantsAndTerms => {
                pkt.participants_and_terms.push(cleaned.to_string())
            }
            EvidenceSection::Topics | EvidenceSection::None => pkt.topics.push(cleaned.to_string()),
            EvidenceSection::Decisions => pkt.decisions.push(cleaned.to_string()),
            EvidenceSection::Actions => pkt.actions.push(cleaned.to_string()),
            EvidenceSection::FrictionsAndRisks => pkt.frictions_and_risks.push(cleaned.to_string()),
            EvidenceSection::Quotes => pkt.quotes.push(cleaned.to_string()),
        }
    }

    if !pkt.has_substance() {
        pkt.is_empty = true;
    }
    pkt
}

fn detect_evidence_header(line: &str) -> Option<EvidenceSection> {
    let stripped = line
        .trim_matches(|c: char| matches!(c, '[' | ']' | '#' | '*' | ':' | ' ' | '-'))
        .to_uppercase();

    // Undvik att vanliga innehållsrader med kolon tolkas som rubrik
    if line.starts_with('-') || line.starts_with('*') || line.starts_with('•') {
        if !line.trim_start_matches(['-', '*', '•', ' ']).starts_with('[') {
            return None;
        }
    }
    if stripped.chars().count() > 42 {
        return None;
    }

    if stripped.contains("DELTAGARE") || stripped.contains("TERMER") {
        Some(EvidenceSection::ParticipantsAndTerms)
    } else if stripped == "ÄMNEN"
        || stripped == "AMNEN"
        || stripped.starts_with("ÄMNEN ")
        || stripped.starts_with("SAKFRÅGOR")
    {
        Some(EvidenceSection::Topics)
    } else if stripped == "BESLUT" || stripped.starts_with("FATTADE BESLUT") {
        Some(EvidenceSection::Decisions)
    } else if stripped == "ÅTGÄRDER"
        || stripped == "ATGARDER"
        || stripped.starts_with("ÅTGÄRDSPUNKTER")
    {
        Some(EvidenceSection::Actions)
    } else if stripped.contains("FRIKTION")
        || stripped.contains("OENIGHET")
        || stripped.contains("RISKER")
    {
        Some(EvidenceSection::FrictionsAndRisks)
    } else if stripped.contains("NYCKELCITAT") || stripped == "CITAT" {
        Some(EvidenceSection::Quotes)
    } else {
        None
    }
}

fn strip_list_marker(s: &str) -> &str {
    let mut out = s.trim();
    if let Some(rest) = out
        .strip_prefix("- [ ] ")
        .or_else(|| out.strip_prefix("- [x] "))
        .or_else(|| out.strip_prefix("* [ ] "))
    {
        return rest.trim();
    }
    if let Some(rest) = out
        .strip_prefix("- ")
        .or_else(|| out.strip_prefix("* "))
        .or_else(|| out.strip_prefix("• "))
        .or_else(|| out.strip_prefix("– "))
    {
        out = rest.trim();
    } else {
        // Numerisk lista t.ex. "1. " eller "2) "
        let digits = out.chars().take_while(|c| c.is_ascii_digit()).count();
        if digits > 0 && digits <= 3 {
            let after = &out[digits..];
            if let Some(rest) = after.strip_prefix(". ").or_else(|| after.strip_prefix(") ")) {
                out = rest.trim();
            }
        }
    }
    out
}

fn is_none_placeholder(s: &str) -> bool {
    let clean = s
        .trim_matches(|c: char| matches!(c, '.' | '!' | '*' | '_' | '-' | '–' | ' '))
        .to_lowercase();
    matches!(
        clean.as_str(),
        ""
            | "inga"
            | "ingen"
            | "inget"
            | "saknas"
            | "ej aktuellt"
            | "inga nämnda"
            | "inga beslut"
            | "inga åtgärder"
            | "inga citat"
            | "inga oenigheter"
            | "inga risker"
            | "inga identifierade"
            | "n/a"
    )
}

/// Parsar `[TEMA: Rubrik | AVSNITT: 1, 4, 7]` från Pass 2A och garanterar 100 % täckning av alla icke-tomma avsnitt!
pub fn parse_theme_clusters_with_full_coverage(
    raw: &str,
    packets: &[EvidencePacket],
) -> Vec<ThemeCluster> {
    let valid_indices: Vec<usize> = packets
        .iter()
        .filter(|p| p.has_substance())
        .map(|p| p.chunk_index)
        .collect();

    if valid_indices.is_empty() {
        return Vec::new();
    }

    let mut clusters: Vec<ThemeCluster> = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim().trim_start_matches(['-', '*', '•', ' ']);
        let inner = trimmed
            .trim_start_matches('[')
            .trim_end_matches(']')
            .trim();
        let upper = inner.to_uppercase();
        if !upper.contains("TEMA") || !upper.contains("AVSNITT") {
            continue;
        }
        if let Some((left, right)) = inner.split_once('|') {
            let title = left
                .split_once(':')
                .map(|(_, t)| t.trim())
                .unwrap_or(left.trim())
                .trim_matches(['*', '#', '"', ' '])
                .to_string();
            let nums_part = right
                .split_once(':')
                .map(|(_, n)| n)
                .unwrap_or(right);
            let mut indices = extract_chunk_numbers(nums_part, packets.len());
            indices.retain(|idx| valid_indices.contains(idx));
            indices.sort_unstable();
            indices.dedup();
            if !title.is_empty() && !indices.is_empty() {
                clusters.push(ThemeCluster {
                    title,
                    chunk_indices: indices,
                });
            }
        }
    }

    // Säkerhetsnät: om modellen glömde något icke-tomt avsnitt (eller om Pass 2A gav ogiltigt format),
    // koppla saknade avsnitt till närmaste kluster eller skapa kronologiska kluster så NOLL fakta tappas!
    if clusters.is_empty() {
        for group in valid_indices.chunks(4) {
            let first = group[0];
            let last = *group.last().unwrap_or(&first);
            let title = if first == last {
                format!("Mötesdel {first}")
            } else {
                format!("Mötesdel {first}–{last}")
            };
            clusters.push(ThemeCluster {
                title,
                chunk_indices: group.to_vec(),
            });
        }
        return clusters;
    }

    let mut covered: Vec<usize> = clusters
        .iter()
        .flat_map(|c| c.chunk_indices.iter().copied())
        .collect();
    covered.sort_unstable();
    covered.dedup();

    let missing: Vec<usize> = valid_indices
        .into_iter()
        .filter(|idx| !covered.contains(idx))
        .collect();

    if !missing.is_empty() {
        clusters.push(ThemeCluster {
            title: "Övriga behandlade punkter".to_string(),
            chunk_indices: missing,
        });
    }

    clusters
}

fn extract_chunk_numbers(s: &str, max_chunk: usize) -> Vec<usize> {
    let mut out = Vec::new();
    // Stöd både listor "1, 3, 5" och intervall "2-4" / "2–4"
    for token in s.split(|c: char| c == ',' || c == ';' || c.is_whitespace()) {
        let t = token.trim_matches(|c: char| !c.is_ascii_digit() && c != '-' && c != '–');
        if t.is_empty() {
            continue;
        }
        if let Some((a_str, b_str)) = t.split_once('-').or_else(|| t.split_once('–')) {
            if let (Ok(a), Ok(b)) = (a_str.trim().parse::<usize>(), b_str.trim().parse::<usize>()) {
                let lo = a.max(1);
                let hi = b.min(max_chunk);
                if lo <= hi && hi - lo <= 64 {
                    out.extend(lo..=hi);
                }
            }
        } else if let Ok(n) = t.parse::<usize>() {
            if n >= 1 && n <= max_chunk {
                out.push(n);
            }
        }
    }
    out
}

/// Grupperar `ThemeCluster`s i batchar där de samlade bevispaketen per batch aldrig överskrider
/// `max_evidence_chars` (så att även ett 8K-fönster på svag hårdvara rymmer varje syntessteg med marginal!).
pub fn batch_theme_clusters_for_budget(
    clusters: &[ThemeCluster],
    packets: &[EvidencePacket],
    max_evidence_chars: usize,
) -> Vec<Vec<ThemeCluster>> {
    let mut batches: Vec<Vec<ThemeCluster>> = Vec::new();
    let mut current_batch: Vec<ThemeCluster> = Vec::new();
    let mut current_chars = 0usize;

    for cluster in clusters {
        let cluster_chars: usize = cluster
            .chunk_indices
            .iter()
            .filter_map(|&idx| packets.get(idx.saturating_sub(1)))
            .map(|p| p.format_for_synthesis().len())
            .sum();

        // Om ett enskilt jättetema spänner över extremt många avsnitt, dela upp temat i delkluster
        if cluster_chars > max_evidence_chars && cluster.chunk_indices.len() > 2 {
            if !current_batch.is_empty() {
                batches.push(std::mem::take(&mut current_batch));
                current_chars = 0;
            }
            let mut sub_indices = Vec::new();
            let mut sub_chars = 0usize;
            let mut part_no = 1usize;
            for &idx in &cluster.chunk_indices {
                let p_len = packets
                    .get(idx.saturating_sub(1))
                    .map(|p| p.format_for_synthesis().len())
                    .unwrap_or(0);
                if !sub_indices.is_empty() && sub_chars + p_len > max_evidence_chars {
                    let sub_title = if part_no == 1 {
                        cluster.title.clone()
                    } else {
                        format!("{} (forts.)", cluster.title)
                    };
                    batches.push(vec![ThemeCluster {
                        title: sub_title,
                        chunk_indices: std::mem::take(&mut sub_indices),
                    }]);
                    sub_chars = 0;
                    part_no += 1;
                }
                sub_indices.push(idx);
                sub_chars += p_len;
            }
            if !sub_indices.is_empty() {
                let sub_title = if part_no == 1 {
                    cluster.title.clone()
                } else {
                    format!("{} (forts.)", cluster.title)
                };
                batches.push(vec![ThemeCluster {
                    title: sub_title,
                    chunk_indices: sub_indices,
                }]);
            }
            continue;
        }

        if !current_batch.is_empty() && current_chars + cluster_chars > max_evidence_chars {
            batches.push(std::mem::take(&mut current_batch));
            current_chars = 0;
        }
        current_chars += cluster_chars;
        current_batch.push(cluster.clone());
    }

    if !current_batch.is_empty() {
        batches.push(current_batch);
    }
    batches
}

// ── Prompterbyggare för Bevispipeline ────────────────────────────────────────

pub fn build_evidence_extract_prompt(
    chunk_text: &str,
    chunk_index: usize,
    total_chunks: usize,
    rolling_state: &RollingMeetingState,
    context: Option<&str>,
    thorough: bool,
) -> String {
    let think_prefix = if thorough { "<|think|>" } else { "" };
    let mut user_body = String::new();

    if let Some(ctx) = context.map(str::trim).filter(|s| !s.is_empty()) {
        user_body.push_str("VERKSAMHET/DOMÄN (för rätt uttydning av hörselfel och facktermer): ");
        user_body.push_str(ctx);
        user_body.push_str("\n\n");
    }

    let rolling_hdr = rolling_state.format_header();
    if !rolling_hdr.is_empty() {
        user_body.push_str(&rolling_hdr);
    }

    user_body.push_str(&format!(
        "--- AVSNITT {chunk_index} AV {total_chunks} ---\n{}\n--- SLUT AVSNITT {chunk_index} ---\n\n\
         Extrahera nu det strukturerade bevispaketet för avsnitt {chunk_index} enligt rubrikformatet.",
        chunk_text.trim()
    ));

    format!(
        "<bos><|turn>system\n{think_prefix}{EVIDENCE_SYSTEM_PROMPT}<turn|>\n<|turn>user\n{user_body}<turn|>\n<|turn>model\n"
    )
}

pub fn format_consolidated_decisions_and_actions(packets: &[EvidencePacket]) -> String {
    let mut all_decisions = Vec::new();
    let mut all_actions = Vec::new();
    for p in packets {
        if p.is_empty {
            continue;
        }
        for d in &p.decisions {
            all_decisions.push(format!("[Avsnitt {}] {}", p.chunk_index, d));
        }
        for a in &p.actions {
            all_actions.push(format!("[Avsnitt {}] {}", p.chunk_index, a));
        }
    }

    if all_decisions.is_empty() && all_actions.is_empty() {
        return String::new();
    }

    let mut reg = String::from("\n--- KONTROLLREGISTER ÖVER ALLA BESLUT & ÅTGÄRDER I MÖTET ---\n");
    if !all_decisions.is_empty() {
        reg.push_str("Samtliga identifierade beslut:\n");
        for d in all_decisions {
            reg.push_str("- ");
            reg.push_str(&d);
            reg.push('\n');
        }
    }
    if !all_actions.is_empty() {
        reg.push_str("Samtliga identifierade åtgärder:\n");
        for a in all_actions {
            reg.push_str("- ");
            reg.push_str(&a);
            reg.push('\n');
        }
    }
    reg.push_str("--- SLUT KONTROLLREGISTER ---\n");
    reg
}

pub fn build_theme_matrix_prompt(
    packets: &[EvidencePacket],
    agenda: Option<&str>,
    context: Option<&str>,
) -> String {
    let mut index_lines = String::new();
    for p in packets {
        let line = p.format_for_theme_index();
        if !line.is_empty() {
            index_lines.push_str(&line);
            index_lines.push('\n');
        }
    }

    let mut extra = String::new();
    if let Some(ctx) = context.map(str::trim).filter(|s| !s.is_empty()) {
        extra.push_str(&format!("MÖTETS SAMMANHANG: {ctx}\n\n"));
    }
    if let Some(ag) = agenda.map(str::trim).filter(|s| !s.is_empty()) {
        extra.push_str(&format!(
            "--- BIFOGAD AGENDA (mappa avsnitten mot dessa punkter) ---\n{ag}\n--- SLUT AGENDA ---\n\n"
        ));
    }

    format!(
        "<bos><|turn>system\n{THEME_MATRIX_SYSTEM_PROMPT}<turn|>\n<|turn>user\n{extra}--- ÖVERSIKT ÖVER MÖTETS AVSNITT ---\n{index_lines}--- SLUT ÖVERSIKT ---\n\nSkapa temamatrisen nu.<turn|>\n<|turn>model\n"
    )
}

fn truncate_chars(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max_chars).collect();
        out.push('…');
        out
    }
}

// ── Självständig kryptografisk SHA-256 (RFC 6234, noll externa crate-beroenden) ──

pub fn compute_evidence_cache_key(
    model_id: &str,
    context: &str,
    total_chunks: usize,
    transcript: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(EVIDENCE_CACHE_SCHEMA.as_bytes());
    hasher.update(&[0]);
    hasher.update(model_id.as_bytes());
    hasher.update(&[0]);
    hasher.update(context.trim().as_bytes());
    hasher.update(&[0]);
    hasher.update(&total_chunks.to_le_bytes());
    hasher.update(&[0]);
    hasher.update(transcript.trim().as_bytes());
    hasher.finalize_hex()
}

struct Sha256 {
    state: [u32; 8],
    buf: [u8; 64],
    buf_len: usize,
    total_bytes: u64,
}

impl Sha256 {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];

    fn new() -> Self {
        Self {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c,
                0x1f83d9ab, 0x5be0cd19,
            ],
            buf: [0u8; 64],
            buf_len: 0,
            total_bytes: 0,
        }
    }

    fn update(&mut self, mut data: &[u8]) {
        self.total_bytes = self.total_bytes.wrapping_add(data.len() as u64);
        if self.buf_len > 0 {
            let take = (64 - self.buf_len).min(data.len());
            self.buf[self.buf_len..self.buf_len + take].copy_from_slice(&data[..take]);
            self.buf_len += take;
            data = &data[take..];
            if self.buf_len == 64 {
                let block = self.buf;
                self.compress(&block);
                self.buf_len = 0;
            }
        }
        while data.len() >= 64 {
            let mut block = [0u8; 64];
            block.copy_from_slice(&data[..64]);
            self.compress(&block);
            data = &data[64..];
        }
        if !data.is_empty() {
            self.buf[..data.len()].copy_from_slice(data);
            self.buf_len = data.len();
        }
    }

    fn finalize_hex(mut self) -> String {
        let bit_len = self.total_bytes.wrapping_mul(8);
        self.buf[self.buf_len] = 0x80;
        self.buf_len += 1;
        if self.buf_len > 56 {
            for b in &mut self.buf[self.buf_len..64] {
                *b = 0;
            }
            let block = self.buf;
            self.compress(&block);
            self.buf_len = 0;
        }
        for b in &mut self.buf[self.buf_len..56] {
            *b = 0;
        }
        self.buf[56..64].copy_from_slice(&bit_len.to_be_bytes());
        let final_block = self.buf;
        self.compress(&final_block);

        let mut hex = String::with_capacity(64);
        for word in self.state {
            use std::fmt::Write;
            let _ = write!(&mut hex, "{word:08x}");
        }
        hex
    }

    fn compress(&mut self, block: &[u8; 64]) {
        let mut w = [0u32; 64];
        for (i, chunk) in block.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let mut a = self.state[0];
        let mut b = self.state[1];
        let mut c = self.state[2];
        let mut d = self.state[3];
        let mut e = self.state[4];
        let mut f = self.state[5];
        let mut g = self.state[6];
        let mut h = self.state[7];

        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(Self::K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        self.state[0] = self.state[0].wrapping_add(a);
        self.state[1] = self.state[1].wrapping_add(b);
        self.state[2] = self.state[2].wrapping_add(c);
        self.state[3] = self.state[3].wrapping_add(d);
        self.state[4] = self.state[4].wrapping_add(e);
        self.state[5] = self.state[5].wrapping_add(f);
        self.state[6] = self.state[6].wrapping_add(g);
        self.state[7] = self.state[7].wrapping_add(h);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_nist_vectors() {
        let mut h_empty = Sha256::new();
        h_empty.update(b"");
        assert_eq!(
            h_empty.finalize_hex(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );

        let mut h_abc = Sha256::new();
        h_abc.update(b"abc");
        assert_eq!(
            h_abc.finalize_hex(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn parses_structured_evidence_packets_tolerantly() {
        let raw = "\
[DELTAGARE_OCH_TERMER]
- Maria Holm (CISO)
- Kårkort -> Kortkort (passerkort)
[ÄMNEN]
- Gallringstid i e-tjänsteplattformen: Jämförelse mellan 90 och 180 dagars lagring av loggar.
[BESLUT]
- Automatisk gallring efter 90 dagar införs senast 15 december (juristens rekommendation).
[ÅTGÄRDER]
- Lägg in gallringsjobb i sprint 14 — Erik Sjöberg (fredag)
[FRIKTION_OCH_RISKER]
- Johan på kundtjänst invände att 90 dagar försvårar utredning av kvartalsfakturor.
[NYCKELCITAT]
- Johan: \"Rensar vi efter 90 dagar står vi utan spårbarhet när kvartalsfakturorna ifrågasätts.\"";

        let pkt = parse_evidence_packet(raw, 2, 8);
        assert!(!pkt.is_empty);
        assert_eq!(pkt.chunk_index, 2);
        assert_eq!(pkt.participants_and_terms.len(), 2);
        assert_eq!(pkt.topics.len(), 1);
        assert_eq!(pkt.decisions.len(), 1);
        assert_eq!(pkt.actions.len(), 1);
        assert_eq!(pkt.frictions_and_risks.len(), 1);
        assert_eq!(pkt.quotes.len(), 1);

        let empty_pkt = parse_evidence_packet("[INGET_RELEVANT]", 1, 8);
        assert!(empty_pkt.is_empty);
        assert!(!empty_pkt.has_substance());
    }

    #[test]
    fn evidence_disk_store_persists_resumes_and_cleans_up() {
        let tmp_root = std::env::temp_dir().join(format!(
            "loki_ev_test_{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));

        let transcript = "Anna: Vi beslutar om ny gallringstid.\nErik: Jag fixar sprint 14.";
        let store =
            EvidenceDiskStore::open(&tmp_root, "e2b", Some("IT-avdelning"), 3, transcript)
                .expect("open store");

        assert!(store.load_chunk(1).is_none());

        let p1 = parse_evidence_packet(
            "[ÄMNEN]\n- Gallring\n[BESLUT]\n- 90 dagar\n[ÅTGÄRDER]\n- Inga",
            1,
            3,
        );
        store.save_chunk(&p1).expect("save chunk 1");

        // Simulerar återupptagning / mallbyte i samma session (samma transkript + modell + sammanhang)
        let reopened =
            EvidenceDiskStore::open(&tmp_root, "e2b", Some("IT-avdelning"), 3, transcript)
                .expect("reopen store");
        let loaded_p1 = reopened.load_chunk(1).expect("chunk 1 cached on disk");
        assert_eq!(loaded_p1, p1);
        assert!(reopened.load_chunk(2).is_none());

        cleanup_all_evidence_cache(&tmp_root);
        assert!(!tmp_root.exists());
    }

    #[test]
    fn two_pass_thematic_grouping_batches_within_8k_budget_and_covers_all_chunks() {
        let mut packets = Vec::new();
        for i in 1..=36 {
            packets.push(EvidencePacket {
                chunk_index: i,
                total_chunks: 36,
                is_empty: false,
                participants_and_terms: vec!["Anna Lindqvist".to_string()],
                topics: vec![format!(
                    "Diskussion i del {i} om budgetramar, systemintegrationer och bemanning under Q4."
                )],
                decisions: if i % 6 == 0 {
                    vec![format!("Beslut fattat i del {i}")]
                } else {
                    Vec::new()
                },
                actions: Vec::new(),
                frictions_and_risks: vec![format!(
                    "Risk lyft i del {i} kring leveranstider och testmiljö."
                )],
                quotes: Vec::new(),
            });
        }

        // Modellen glömmer medvetet avsnitt 35 och 36 i sitt svar — säkerhetsnätet ska fånga upp dem!
        let raw_matrix = "\
[TEMA: Budget och ekonomiska ramar | AVSNITT: 1-10]
[TEMA: Systemintegration och teknik | AVSNITT: 11-22]
[TEMA: Bemanning och införandeplan | AVSNITT: 23-34]";

        let clusters = parse_theme_clusters_with_full_coverage(raw_matrix, &packets);
        assert_eq!(clusters.len(), 4);
        assert_eq!(clusters[3].chunk_indices, vec![35, 36]);

        // Max ~4 500 tecken bevis per syntesbatch (~1 300 tokens -> ryms med stor marginal i 8 192-fönster!)
        let max_chars = 4_500;
        let batches = batch_theme_clusters_for_budget(&clusters, &packets, max_chars);
        assert!(batches.len() >= 2);
        for b in &batches {
            let batch_chars: usize = b
                .iter()
                .flat_map(|c| c.chunk_indices.iter())
                .map(|&idx| packets[idx - 1].format_for_synthesis().len())
                .sum();
            assert!(
                batch_chars <= max_chars,
                "Batch överskred budget: {batch_chars} > {max_chars}"
            );
        }
    }
}
