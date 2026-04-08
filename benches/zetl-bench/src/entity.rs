// Entity extraction — deterministic heuristics for wikilink generation

use regex::Regex;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct Entity {
    pub name: String,
    pub normalized: String,
    pub entity_type: EntityType,
    pub mention_count: usize,
    pub session_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum EntityType {
    ProperNoun,
    QuotedTerm,
    RepeatedPhrase,
    TemporalMarker,
    TechnicalTerm,
}

/// Words to skip when detecting proper nouns (common sentence-interior capitals, pronouns, etc.)
const SKIP_WORDS: &[&str] = &[
    "I", "The", "This", "My", "It", "He", "She", "We", "They", "You",
    "His", "Her", "Its", "Our", "Their", "Your", "A", "An", "And", "But",
    "Or", "So", "If", "In", "On", "At", "To", "For", "Of", "By", "Is",
    "Are", "Was", "Were", "Be", "Do", "Did", "Has", "Had", "Not", "No",
    "Yes", "That", "What", "When", "Where", "Who", "How", "All", "Each",
    "Some", "Any", "Than", "Then", "Also", "Just", "Can", "May", "Will",
    "Would", "Could", "Should", "Much", "Many", "Most", "Very", "Here",
    "There", "With", "From", "About", "After", "Before", "Between",
];

/// Normalize entity name: lowercase, collapse whitespace, trim.
pub fn normalize_entity_name(name: &str) -> String {
    name.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Extract entities from all sessions. Input: (session_id, session_text) pairs.
pub fn extract_entities(sessions: &[(String, String)]) -> Vec<Entity> {
    // Accumulator: normalized_name -> (best_original_name, entity_type, set of session_ids, total_mentions)
    let mut acc: HashMap<String, (String, EntityType, Vec<String>, usize)> = HashMap::new();

    let re_quoted = Regex::new(r#""([^"]+)""#).unwrap();
    let re_backtick = Regex::new(r"`([^`]+)`").unwrap();
    // Temporal: "Month YYYY", "Month DDth", "YYYY-MM-DD", "last/next/this Weekday"
    let re_temporal = Regex::new(
        r"(?x)
        (?:January|February|March|April|May|June|July|August|September|October|November|December)
        \s+\d{1,2}(?:st|nd|rd|th)?,?\s+\d{4}
        |
        (?:January|February|March|April|May|June|July|August|September|October|November|December)
        \s+\d{4}
        |
        \d{4}-\d{2}-\d{2}
        |
        (?:last|next|this)\s+(?:Monday|Tuesday|Wednesday|Thursday|Friday|Saturday|Sunday)
        |
        (?:January|February|March|April|May|June|July|August|September|October|November|December)
        \s+\d{1,2}(?:st|nd|rd|th)?
        "
    )
    .unwrap();
    // Proper nouns mid-sentence: after ". X" or start-of-line we skip; we look for capitalized
    // words that are NOT at sentence start.
    let re_proper = Regex::new(r"(?m)(?:^|[.!?]\s+)(\S+)").unwrap(); // sentence starters
    // Match capitalized words: starts with uppercase, may contain lowercase/uppercase mix.
    // Multi-word: sequences of capitalized words separated by spaces.
    let re_cap_word = Regex::new(r"\b([A-Z][A-Za-z]*(?:\s+[A-Z][A-Za-z]*)*)").unwrap();

    let skip_set: std::collections::HashSet<&str> = SKIP_WORDS.iter().copied().collect();

    for (session_id, text) in sessions {
        // --- Quoted terms ---
        for cap in re_quoted.captures_iter(text) {
            let inner = cap[1].trim();
            if inner.is_empty() || inner.len() < 2 {
                continue;
            }
            insert_entity(
                &mut acc,
                inner.to_string(),
                EntityType::QuotedTerm,
                session_id,
            );
        }

        // --- Technical terms (backtick) ---
        for cap in re_backtick.captures_iter(text) {
            let inner = cap[1].trim();
            if inner.is_empty() {
                continue;
            }
            insert_entity(
                &mut acc,
                inner.to_string(),
                EntityType::TechnicalTerm,
                session_id,
            );
        }

        // --- Temporal markers ---
        for cap in re_temporal.captures_iter(text) {
            let matched = cap[0].trim();
            if matched.is_empty() {
                continue;
            }
            insert_entity(
                &mut acc,
                matched.to_string(),
                EntityType::TemporalMarker,
                session_id,
            );
        }

        // --- Proper nouns ---
        // Find sentence start positions so we can skip words there.
        let mut sentence_start_positions: Vec<usize> = vec![0];
        for m in re_proper.find_iter(text) {
            sentence_start_positions.push(m.start());
        }

        for cap in re_cap_word.captures_iter(text) {
            let m = cap.get(0).unwrap();
            let word = m.as_str();

            // Skip if at sentence start (within 3 chars of a sentence boundary)
            let pos = m.start();
            let at_sentence_start = sentence_start_positions
                .iter()
                .any(|&sp| pos <= sp + 2 && pos >= sp);

            // Also check if this position is right at the start of a line
            let at_line_start = pos == 0
                || text.as_bytes().get(pos.saturating_sub(1)) == Some(&b'\n')
                || (pos >= 2
                    && text.as_bytes().get(pos.saturating_sub(2)) == Some(&b'\n')
                    && text.as_bytes().get(pos.saturating_sub(1)) == Some(&b' '));

            if at_sentence_start || at_line_start {
                // Only skip single words at sentence start; multi-word sequences are likely proper
                if !word.contains(' ') {
                    continue;
                }
            }

            // Skip common words
            let first_word = word.split_whitespace().next().unwrap_or(word);
            if skip_set.contains(first_word) {
                continue;
            }

            // Skip single short words (likely not entities)
            if !word.contains(' ') && word.len() <= 1 {
                continue;
            }

            // Skip if it's inside quotes or backticks (already captured)
            let before = &text[..pos];
            let after = &text[pos..];
            let in_quotes = before.matches('"').count() % 2 == 1;
            let in_backticks = before.matches('`').count() % 2 == 1
                && after.contains('`');
            if in_quotes || in_backticks {
                continue;
            }

            insert_entity(
                &mut acc,
                word.to_string(),
                EntityType::ProperNoun,
                session_id,
            );
        }
    }

    // --- Second pass: repeated noun phrases across 3+ sessions become RepeatedPhrase ---
    // Collect phrase candidates from proper nouns that appear in 3+ sessions
    let mut entities: Vec<Entity> = acc
        .into_iter()
        .map(|(_, (name, etype, session_ids, mention_count))| {
            let entity_type = if etype == EntityType::ProperNoun && session_ids.len() >= 3 {
                EntityType::RepeatedPhrase
            } else {
                etype
            };
            Entity {
                normalized: normalize_entity_name(&name),
                name,
                entity_type,
                mention_count,
                session_ids,
            }
        })
        .collect();

    // Sort for deterministic output: by name
    entities.sort_by(|a, b| a.normalized.cmp(&b.normalized));
    entities
}

/// Insert or update an entity in the accumulator.
fn insert_entity(
    acc: &mut HashMap<String, (String, EntityType, Vec<String>, usize)>,
    name: String,
    entity_type: EntityType,
    session_id: &str,
) {
    let norm = normalize_entity_name(&name);
    let entry = acc
        .entry(norm)
        .or_insert_with(|| (name.clone(), entity_type, Vec::new(), 0));
    entry.3 += 1;
    if !entry.2.contains(&session_id.to_string()) {
        entry.2.push(session_id.to_string());
    }
    // Keep the longest original form
    if name.len() > entry.0.len() {
        entry.0 = name;
    }
}

/// Insert wikilinks into session text. First mention of each entity gets [[wikilink]].
pub fn insert_wikilinks(text: &str, entities: &[Entity]) -> String {
    // Sort entities by name length descending so longer matches take priority
    let mut sorted: Vec<&Entity> = entities.iter().collect();
    sorted.sort_by(|a, b| b.name.len().cmp(&a.name.len()));

    let mut result = text.to_string();
    let mut linked: std::collections::HashSet<String> = std::collections::HashSet::new();

    for entity in &sorted {
        let norm = &entity.normalized;
        if linked.contains(norm) {
            continue;
        }

        // Case-insensitive search on the original string using regex
        // This avoids byte-position mismatches from to_lowercase() on non-ASCII
        let pattern = regex::escape(norm);
        let re = match Regex::new(&format!("(?i){}", pattern)) {
            Ok(r) => r,
            Err(_) => continue,
        };

        if let Some(m) = re.find(&result) {
            let pos = m.start();
            let end = m.end();

            // Check not already inside wikilink brackets
            let before = &result[..pos];
            let after = &result[end..];
            if before.ends_with("[[") || after.starts_with("]]") {
                continue;
            }

            let matched_text = &result[pos..end];
            let replacement = format!("[[{}]]", matched_text);
            result = format!("{}{}{}", &result[..pos], replacement, &result[end..]);
            linked.insert(norm.clone());
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_proper_nouns() {
        let sessions = vec![(
            "s1".to_string(),
            "I switched to PostgreSQL last year.".to_string(),
        )];
        let entities = extract_entities(&sessions);
        let names: Vec<&str> = entities.iter().map(|e| e.name.as_str()).collect();
        assert!(
            names.contains(&"PostgreSQL"),
            "Expected 'PostgreSQL' in {:?}",
            names
        );
    }

    #[test]
    fn extracts_quoted_terms() {
        let sessions = vec![(
            "s1".to_string(),
            r#"He said "machine learning" is cool"#.to_string(),
        )];
        let entities = extract_entities(&sessions);
        let names: Vec<&str> = entities.iter().map(|e| e.name.as_str()).collect();
        assert!(
            names.contains(&"machine learning"),
            "Expected 'machine learning' in {:?}",
            names
        );
        let ent = entities.iter().find(|e| e.name == "machine learning").unwrap();
        assert_eq!(ent.entity_type, EntityType::QuotedTerm);
    }

    #[test]
    fn extracts_temporal_markers() {
        let sessions = vec![(
            "s1".to_string(),
            "In March 2024 I started a new project.".to_string(),
        )];
        let entities = extract_entities(&sessions);
        let names: Vec<&str> = entities.iter().map(|e| e.name.as_str()).collect();
        assert!(
            names.contains(&"March 2024"),
            "Expected 'March 2024' in {:?}",
            names
        );
        let ent = entities.iter().find(|e| e.name == "March 2024").unwrap();
        assert_eq!(ent.entity_type, EntityType::TemporalMarker);
    }

    #[test]
    fn extracts_backtick_terms() {
        let sessions = vec![(
            "s1".to_string(),
            "Using `Redis` for caching".to_string(),
        )];
        let entities = extract_entities(&sessions);
        let names: Vec<&str> = entities.iter().map(|e| e.name.as_str()).collect();
        assert!(
            names.contains(&"Redis"),
            "Expected 'Redis' in {:?}",
            names
        );
        let ent = entities.iter().find(|e| e.name == "Redis").unwrap();
        assert_eq!(ent.entity_type, EntityType::TechnicalTerm);
    }

    #[test]
    fn normalizes_entity_names() {
        assert_eq!(normalize_entity_name("PostgreSQL"), "postgresql");
        assert_eq!(normalize_entity_name("  hello   world  "), "hello world");
        assert_eq!(
            normalize_entity_name("PostgreSQL"),
            normalize_entity_name("postgresql")
        );
    }

    #[test]
    fn insert_wikilinks_first_mention_only() {
        let entities = vec![Entity {
            name: "PostgreSQL".to_string(),
            normalized: "postgresql".to_string(),
            entity_type: EntityType::TechnicalTerm,
            mention_count: 2,
            session_ids: vec!["s1".to_string()],
        }];
        let text = "I use PostgreSQL daily. PostgreSQL is great.";
        let result = insert_wikilinks(text, &entities);
        assert_eq!(result, "I use [[PostgreSQL]] daily. PostgreSQL is great.");
    }

    #[test]
    fn deduplicates_across_sessions() {
        let sessions = vec![
            ("s1".to_string(), "I use PostgreSQL for work.".to_string()),
            ("s2".to_string(), "My favorite database is PostgreSQL for sure.".to_string()),
        ];
        let entities = extract_entities(&sessions);
        let pg_entities: Vec<_> = entities
            .iter()
            .filter(|e| e.normalized == "postgresql")
            .collect();
        assert_eq!(
            pg_entities.len(),
            1,
            "Should have exactly one PostgreSQL entity, got {:?}",
            pg_entities
        );
        let pg = &pg_entities[0];
        assert_eq!(pg.mention_count, 2);
        assert!(pg.session_ids.contains(&"s1".to_string()));
        assert!(pg.session_ids.contains(&"s2".to_string()));
    }
}
