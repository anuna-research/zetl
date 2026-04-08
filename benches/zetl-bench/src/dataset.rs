use anyhow::Result;
use serde::{Deserialize, Deserializer};
use std::collections::HashMap;
use std::path::Path;

fn deserialize_string_or_number<'de, D>(deserializer: D) -> std::result::Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value: serde_json::Value = Deserialize::deserialize(deserializer)?;
    match value {
        serde_json::Value::String(s) => Ok(s),
        serde_json::Value::Number(n) => Ok(n.to_string()),
        other => Ok(other.to_string()),
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct Turn {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct LongMemEntry {
    pub question_id: String,
    pub question_type: String,
    pub question: String,
    #[serde(default)]
    pub question_date: Option<String>,
    #[serde(deserialize_with = "deserialize_string_or_number")]
    pub answer: String,
    pub answer_session_ids: Vec<String>,
    pub haystack_sessions: Vec<Vec<Turn>>,
    pub haystack_session_ids: Vec<String>,
    pub haystack_dates: Vec<String>,
}

impl LongMemEntry {
    /// Concatenate all user turns in the session at `idx` with newlines.
    pub fn session_text(&self, idx: usize) -> String {
        self.haystack_sessions[idx]
            .iter()
            .filter(|t| t.role == "user")
            .map(|t| t.content.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Returns (session_id, session_text) pairs for all sessions.
    pub fn all_session_texts(&self) -> Vec<(String, String)> {
        self.haystack_session_ids
            .iter()
            .enumerate()
            .map(|(i, id)| (id.clone(), self.session_text(i)))
            .collect()
    }
}

pub fn load(path: &Path, limit: Option<usize>) -> Result<Vec<LongMemEntry>> {
    let data = std::fs::read_to_string(path)?;
    let mut entries: Vec<LongMemEntry> = serde_json::from_str(&data)?;

    let total = entries.len();
    if let Some(n) = limit {
        entries.truncate(n);
    }

    // Print stats
    let mut type_counts: HashMap<&str, usize> = HashMap::new();
    for e in &entries {
        *type_counts.entry(e.question_type.as_str()).or_default() += 1;
    }
    eprintln!("[dataset] loaded {} / {} entries", entries.len(), total);
    let mut sorted: Vec<_> = type_counts.into_iter().collect();
    sorted.sort_by_key(|(_, c)| std::cmp::Reverse(*c));
    for (qt, count) in &sorted {
        eprintln!("[dataset]   {}: {}", qt, count);
    }

    Ok(entries)
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
    fn test_parse_fixture() {
        let entries: Vec<LongMemEntry> = serde_json::from_str(fixture_json()).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].question_id, "q1");
        assert_eq!(entries[0].question_type, "single-session");
        assert_eq!(entries[0].answer, "Whiskers");
        assert_eq!(entries[0].haystack_session_ids.len(), 2);
        assert_eq!(entries[1].question_id, "q2");
        assert_eq!(entries[1].question_type, "multi-session");
        assert_eq!(entries[1].answer_session_ids, vec!["s3", "s4"]);
    }

    #[test]
    fn test_session_text_user_only() {
        let entries: Vec<LongMemEntry> = serde_json::from_str(fixture_json()).unwrap();
        // Session 0 of q1 has two user turns
        let text = entries[0].session_text(0);
        assert_eq!(text, "My cat is named Whiskers.\nHe likes tuna.");
        // Session 1 of q1 has one user turn
        let text = entries[0].session_text(1);
        assert_eq!(text, "I went hiking today.");
    }

    #[test]
    fn test_all_session_texts() {
        let entries: Vec<LongMemEntry> = serde_json::from_str(fixture_json()).unwrap();
        let pairs = entries[1].all_session_texts();
        assert_eq!(pairs.len(), 2);
        assert_eq!(pairs[0].0, "s3");
        assert_eq!(pairs[0].1, "I visited Paris last summer.");
        assert_eq!(pairs[1].0, "s4");
        assert_eq!(pairs[1].1, "Tokyo was amazing too.\nEspecially the ramen.");
    }

    #[test]
    fn test_load_with_limit() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(fixture_json().as_bytes()).unwrap();
        tmp.flush().unwrap();

        let entries = load(tmp.path(), Some(1)).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].question_id, "q1");
    }

    #[test]
    fn test_load_no_limit() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(fixture_json().as_bytes()).unwrap();
        tmp.flush().unwrap();

        let entries = load(tmp.path(), None).unwrap();
        assert_eq!(entries.len(), 2);
    }
}
