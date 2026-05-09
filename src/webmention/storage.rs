//! On-disk storage primitives per CON-3906 + ADR-3904.
//!
//! Layout:
//!
//! ```text
//! .zetl/webmentions/
//!   received.jsonl  — accepted ExternalEdge records (live + tombstones)
//!   sent.jsonl      — sender idempotency log (SentRecord)
//!   queue.jsonl     — pending moderation (IncomingMention)
//! ```
//!
//! All three files are append-only JSONL. Compaction is documented
//! future work (NFR-3907); v1 grows linearly. Reads are tolerant of
//! malformed lines: a corrupted line is skipped with a warn-level log
//! line so a partial-write torn record cannot wedge the loader.

use serde::{de::DeserializeOwned, Serialize};
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

/// Subdirectory of the vault root where webmention state lives.
pub const SUBDIR: &str = "webmentions";

/// File names within [`SUBDIR`].
pub const RECEIVED_FILE: &str = "received.jsonl";
pub const SENT_FILE: &str = "sent.jsonl";
pub const QUEUE_FILE: &str = "queue.jsonl";

/// `.zetl/webmentions/` for the given vault root. Does NOT create the
/// directory; use [`ensure_dir`] when you need it on disk.
pub fn vault_dir(vault_root: &Path) -> PathBuf {
    vault_root.join(".zetl").join(SUBDIR)
}

/// Idempotently create `.zetl/webmentions/`. Returns the directory path.
pub fn ensure_dir(vault_root: &Path) -> std::io::Result<PathBuf> {
    let dir = vault_dir(vault_root);
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Append a single JSONL record to `path`. Creates the file if missing.
/// One record per line; embedded newlines in the JSON body are escaped
/// by `serde_json` so the line-oriented format stays intact.
pub fn append_jsonl<T: Serialize>(path: &Path, record: &T) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    let mut line = serde_json::to_string(record)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    line.push('\n');
    file.write_all(line.as_bytes())?;
    file.sync_data()?;
    Ok(())
}

/// Read all JSONL records from `path`. Missing file is treated as empty
/// (no error). Malformed lines are skipped with a warn line on stderr;
/// callers must not panic on partial reads.
pub fn read_jsonl<T: DeserializeOwned>(path: &Path) -> std::io::Result<Vec<T>> {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let reader = BufReader::new(file);
    let mut out = Vec::new();
    for (idx, line) in reader.lines().enumerate() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                eprintln!(
                    "warn: webmention storage: io error on line {} of {}: {}",
                    idx + 1,
                    path.display(),
                    e
                );
                continue;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<T>(&line) {
            Ok(rec) => out.push(rec),
            Err(e) => {
                eprintln!(
                    "warn: webmention storage: skipping malformed line {} of {}: {}",
                    idx + 1,
                    path.display(),
                    e
                );
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use tempfile::tempdir;

    #[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
    struct Row {
        a: u32,
        b: String,
    }

    #[test]
    fn roundtrip_jsonl() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("rows.jsonl");
        let rows = vec![
            Row {
                a: 1,
                b: "x".into(),
            },
            Row {
                a: 2,
                b: "y".into(),
            },
            Row {
                a: 3,
                b: "z".into(),
            },
        ];
        for r in &rows {
            append_jsonl(&path, r).unwrap();
        }
        let back: Vec<Row> = read_jsonl(&path).unwrap();
        assert_eq!(back, rows);
    }

    #[test]
    fn missing_file_is_empty() {
        let dir = tempdir().unwrap();
        let back: Vec<Row> = read_jsonl(&dir.path().join("nope.jsonl")).unwrap();
        assert!(back.is_empty());
    }

    #[test]
    fn malformed_middle_line_is_skipped() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("rows.jsonl");
        append_jsonl(
            &path,
            &Row {
                a: 1,
                b: "x".into(),
            },
        )
        .unwrap();
        // Inject a torn / corrupt line in the middle.
        {
            let mut f = OpenOptions::new().append(true).open(&path).unwrap();
            f.write_all(b"{this is not valid json\n").unwrap();
        }
        append_jsonl(
            &path,
            &Row {
                a: 2,
                b: "y".into(),
            },
        )
        .unwrap();
        let back: Vec<Row> = read_jsonl(&path).unwrap();
        assert_eq!(
            back,
            vec![
                Row {
                    a: 1,
                    b: "x".into()
                },
                Row {
                    a: 2,
                    b: "y".into()
                },
            ]
        );
    }

    #[test]
    fn ensure_dir_idempotent() {
        let dir = tempdir().unwrap();
        let p1 = ensure_dir(dir.path()).unwrap();
        let p2 = ensure_dir(dir.path()).unwrap();
        assert_eq!(p1, p2);
        assert!(p1.is_dir());
    }
}
