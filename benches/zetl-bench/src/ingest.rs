// Vault generation from LongMemEval data

use anyhow::Result;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::dataset;
use crate::entity;

/// A deduplicated session with its full turn data and date.
struct SessionInfo {
    session_id: String,
    turns: Vec<dataset::Turn>,
    date: String,
}

pub fn run(data: &Path, vault_dir: &Path, no_entities: bool) -> Result<()> {
    let entries = dataset::load(data, None)?;

    // 1. Deduplicate sessions across all questions.
    //    Use BTreeMap for deterministic iteration order.
    let mut session_map: BTreeMap<String, SessionInfo> = BTreeMap::new();
    for entry in &entries {
        for (i, sid) in entry.haystack_session_ids.iter().enumerate() {
            session_map.entry(sid.clone()).or_insert_with(|| SessionInfo {
                session_id: sid.clone(),
                turns: entry.haystack_sessions[i].clone(),
                date: entry.haystack_dates[i].clone(),
            });
        }
    }

    // 2-4. Entity extraction (skipped with --no-entities)
    let entities;
    let entity_sessions: BTreeMap<String, BTreeSet<String>>;
    let session_entities: BTreeMap<String, BTreeSet<String>>;

    if no_entities {
        entities = Vec::new();
        entity_sessions = BTreeMap::new();
        session_entities = BTreeMap::new();
    } else {
        let session_texts: Vec<(String, String)> = session_map
            .values()
            .map(|info| {
                let user_text = info
                    .turns
                    .iter()
                    .filter(|t| t.role == "user")
                    .map(|t| t.content.as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
                (info.session_id.clone(), user_text)
            })
            .collect();

        entities = entity::extract_entities(&session_texts);

        let mut es: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut se: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for ent in &entities {
            for sid in &ent.session_ids {
                es.entry(ent.normalized.clone()).or_default().insert(sid.clone());
                se.entry(sid.clone()).or_default().insert(ent.normalized.clone());
            }
        }
        entity_sessions = es;
        session_entities = se;
    }

    // 5. Create directories.
    let sessions_dir = vault_dir.join("sessions");
    let entities_dir = vault_dir.join("entities");
    std::fs::create_dir_all(&sessions_dir)?;
    if !no_entities {
        std::fs::create_dir_all(&entities_dir)?;
    }

    // 6. Write session files.
    let mut total_wikilinks = 0usize;
    for info in session_map.values() {
        let mut body = String::new();
        for turn in &info.turns {
            if turn.role == "user" {
                let linked = entity::insert_wikilinks(&turn.content, &entities);
                // Count wikilinks inserted in this turn
                total_wikilinks += linked.matches("[[").count();
                body.push_str(&format!("**User:** {}\n\n", linked));
            } else {
                body.push_str(&format!("**Assistant:** {}\n\n", turn.content));
            }
        }

        // Find related sessions (those sharing 2+ entities with this session).
        let my_entities = session_entities
            .get(&info.session_id)
            .cloned()
            .unwrap_or_default();
        let mut related: BTreeSet<String> = BTreeSet::new();
        for ent_norm in &my_entities {
            if let Some(sids) = entity_sessions.get(ent_norm) {
                for sid in sids {
                    if sid != &info.session_id {
                        // Check shared entity count >= 2
                        let other_ents = session_entities
                            .get(sid)
                            .cloned()
                            .unwrap_or_default();
                        let shared = my_entities.intersection(&other_ents).count();
                        if shared >= 2 {
                            related.insert(sid.clone());
                        }
                    }
                }
            }
        }

        let mut related_section = String::new();
        if !related.is_empty() {
            related_section.push_str("## Related\n\n");
            for sid in &related {
                related_section.push_str(&format!("- [[{}]]\n", sid));
            }
            related_section.push('\n');
        }

        let content = format!(
            "---\nsession_id: {}\ntimestamp: \"{}\"\n---\n\n# {}\n\n{}{}",
            info.session_id, info.date, info.session_id, body, related_section
        );

        std::fs::write(sessions_dir.join(format!("{}.md", info.session_id)), content)?;
    }

    // 7. Write entity hub pages.
    let mut hub_count = 0usize;
    for ent in &entities {
        let mut refs = String::new();
        // Sort session_ids for deterministic output
        let mut sorted_sids = ent.session_ids.clone();
        sorted_sids.sort();
        for sid in &sorted_sids {
            refs.push_str(&format!("- [[{}]]\n", sid));
        }

        let type_str = match ent.entity_type {
            entity::EntityType::ProperNoun => "proper_noun",
            entity::EntityType::QuotedTerm => "quoted_term",
            entity::EntityType::RepeatedPhrase => "repeated_phrase",
            entity::EntityType::TemporalMarker => "temporal_marker",
            entity::EntityType::TechnicalTerm => "technical_term",
        };

        let content = format!(
            "---\nentity: {}\ntype: {}\nmentions: {}\n---\n\n# {}\n\nReferenced in:\n{}\n",
            ent.normalized, type_str, ent.mention_count, ent.name, refs
        );

        std::fs::write(
            entities_dir.join(format!("{}.md", ent.normalized)),
            content,
        )?;
        hub_count += 1;
    }

    // 8. Print stats.
    eprintln!("[ingest] sessions generated: {}", session_map.len());
    eprintln!("[ingest] entities extracted: {}", entities.len());
    eprintln!("[ingest] wikilinks inserted: {}", total_wikilinks);
    eprintln!("[ingest] hub pages created: {}", hub_count);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn fixture_json() -> &'static str {
        r#"[
  {
    "question_id": "q1",
    "question_type": "single-session",
    "question": "What is Alice's cat named?",
    "answer": "Whiskers",
    "answer_session_ids": ["s1"],
    "haystack_sessions": [
      [
        {"role": "user", "content": "My cat is named Whiskers."},
        {"role": "assistant", "content": "That's a cute name!"},
        {"role": "user", "content": "He likes tuna."}
      ],
      [
        {"role": "user", "content": "I went hiking today."},
        {"role": "assistant", "content": "How was it?"}
      ]
    ],
    "haystack_session_ids": ["s1", "s2"],
    "haystack_dates": ["2025-01-01", "2025-01-02"]
  },
  {
    "question_id": "q2",
    "question_type": "multi-session",
    "question": "Where did Bob travel?",
    "answer": "Paris and Tokyo",
    "answer_session_ids": ["s3", "s4"],
    "haystack_sessions": [
      [
        {"role": "user", "content": "I visited Paris last summer."},
        {"role": "assistant", "content": "Lovely city."}
      ],
      [
        {"role": "user", "content": "Tokyo was amazing too."},
        {"role": "assistant", "content": "Great food there."},
        {"role": "user", "content": "Especially the ramen."}
      ]
    ],
    "haystack_session_ids": ["s3", "s4"],
    "haystack_dates": ["2025-03-10", "2025-04-15"]
  }
]"#
    }

    #[test]
    fn test_ingest_creates_session_files() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(fixture_json().as_bytes()).unwrap();
        tmp.flush().unwrap();

        let vault = tempfile::tempdir().unwrap();
        run(tmp.path(), vault.path(), false).unwrap();

        // 4 unique sessions: s1, s2, s3, s4
        let sessions_dir = vault.path().join("sessions");
        assert!(sessions_dir.join("s1.md").exists());
        assert!(sessions_dir.join("s2.md").exists());
        assert!(sessions_dir.join("s3.md").exists());
        assert!(sessions_dir.join("s4.md").exists());

        // Verify count
        let count = std::fs::read_dir(&sessions_dir)
            .unwrap()
            .filter(|e| e.as_ref().unwrap().path().extension().map_or(false, |x| x == "md"))
            .count();
        assert_eq!(count, 4, "Expected 4 session files");
    }

    #[test]
    fn test_ingest_wikilinks_in_sessions() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(fixture_json().as_bytes()).unwrap();
        tmp.flush().unwrap();

        let vault = tempfile::tempdir().unwrap();
        run(tmp.path(), vault.path(), false).unwrap();

        // s1 mentions "Whiskers" which should be extracted as a proper noun
        let s1 = std::fs::read_to_string(vault.path().join("sessions/s1.md")).unwrap();
        assert!(
            s1.contains("[["),
            "Session s1 should contain at least one wikilink, got:\n{}",
            s1
        );

        // s3 mentions "Paris" — check for wikilink
        let s3 = std::fs::read_to_string(vault.path().join("sessions/s3.md")).unwrap();
        assert!(
            s3.contains("[["),
            "Session s3 should contain at least one wikilink, got:\n{}",
            s3
        );
    }

    #[test]
    fn test_ingest_entity_hub_pages() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(fixture_json().as_bytes()).unwrap();
        tmp.flush().unwrap();

        let vault = tempfile::tempdir().unwrap();
        run(tmp.path(), vault.path(), false).unwrap();

        let entities_dir = vault.path().join("entities");
        assert!(entities_dir.exists(), "entities/ directory should exist");

        // At least one hub page should exist
        let count = std::fs::read_dir(&entities_dir)
            .unwrap()
            .filter(|e| e.as_ref().unwrap().path().extension().map_or(false, |x| x == "md"))
            .count();
        assert!(count > 0, "Expected at least one entity hub page");

        // Each hub page should have frontmatter and references
        for entry in std::fs::read_dir(&entities_dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().map_or(false, |x| x == "md") {
                let content = std::fs::read_to_string(&path).unwrap();
                assert!(content.contains("entity:"), "Hub page {:?} missing entity frontmatter", path);
                assert!(content.contains("Referenced in:"), "Hub page {:?} missing references", path);
                assert!(content.contains("[["), "Hub page {:?} missing session links", path);
            }
        }
    }

    #[test]
    fn test_ingest_deterministic() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(fixture_json().as_bytes()).unwrap();
        tmp.flush().unwrap();

        // Run 1
        let vault1 = tempfile::tempdir().unwrap();
        run(tmp.path(), vault1.path(), false).unwrap();

        // Run 2
        let vault2 = tempfile::tempdir().unwrap();
        run(tmp.path(), vault2.path(), false).unwrap();

        // Compare all session files
        for name in &["s1.md", "s2.md", "s3.md", "s4.md"] {
            let c1 = std::fs::read_to_string(vault1.path().join("sessions").join(name)).unwrap();
            let c2 = std::fs::read_to_string(vault2.path().join("sessions").join(name)).unwrap();
            assert_eq!(c1, c2, "Session {} differs between runs", name);
        }

        // Compare all entity hub pages
        let mut files1: Vec<String> = std::fs::read_dir(vault1.path().join("entities"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        files1.sort();
        let mut files2: Vec<String> = std::fs::read_dir(vault2.path().join("entities"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        files2.sort();
        assert_eq!(files1, files2, "Entity file lists differ between runs");

        for name in &files1 {
            let c1 = std::fs::read_to_string(vault1.path().join("entities").join(name)).unwrap();
            let c2 = std::fs::read_to_string(vault2.path().join("entities").join(name)).unwrap();
            assert_eq!(c1, c2, "Entity hub {} differs between runs", name);
        }
    }

    #[test]
    fn test_ingest_session_contains_both_roles() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(fixture_json().as_bytes()).unwrap();
        tmp.flush().unwrap();

        let vault = tempfile::tempdir().unwrap();
        run(tmp.path(), vault.path(), false).unwrap();

        let s1 = std::fs::read_to_string(vault.path().join("sessions/s1.md")).unwrap();
        assert!(s1.contains("**User:**"), "Session should contain user turns");
        assert!(s1.contains("**Assistant:**"), "Session should contain assistant turns");
        assert!(
            s1.contains("That's a cute name!"),
            "Assistant content should be present verbatim"
        );
    }

    #[test]
    fn test_ingest_frontmatter() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(fixture_json().as_bytes()).unwrap();
        tmp.flush().unwrap();

        let vault = tempfile::tempdir().unwrap();
        run(tmp.path(), vault.path(), false).unwrap();

        let s1 = std::fs::read_to_string(vault.path().join("sessions/s1.md")).unwrap();
        assert!(s1.starts_with("---\n"), "Should start with frontmatter");
        assert!(s1.contains("session_id: s1"), "Should have session_id");
        assert!(s1.contains("timestamp: \"2025-01-01\""), "Should have timestamp");
    }
}
