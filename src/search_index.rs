use std::path::Path;

use anyhow::{Context, Result};
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::{Field, Schema, SchemaBuilder, OwnedValue, STORED, STRING, TEXT};
use tantivy::{Index, IndexReader, IndexWriter, ReloadPolicy, TantivyDocument};

use crate::scanner::body_text_ranges;
use crate::types::ParsedFile;

/// A single result from a Tantivy full-text search query.
///
/// REQ-013-001.
#[derive(Debug, Clone)]
pub struct SearchHit {
    pub page_name: String,
    pub path: String,
    pub score: f64,
}

struct Fields {
    page_name: Field,
    path: Field,
    body: Field,
}

/// Tantivy-backed full-text search index stored at `.zetl/search/`.
///
/// REQ-013-001, ADR-013-001.
pub struct SearchIndex {
    index: Index,
    fields: Fields,
}

impl SearchIndex {
    /// Build or rebuild the full-text index from the supplied parsed files.
    ///
    /// Creates `.zetl/search/` under `vault_root` if it does not exist. The index is
    /// always rebuilt from scratch; incremental updates are handled by the caller via
    /// the file cache.
    ///
    /// Body text is extracted using [`crate::scanner::body_text_ranges`], reusing the
    /// same exclusion logic as SPEC-002 (no frontmatter, fenced code blocks, inline
    /// code, HTML comments).
    pub fn build(vault_root: &Path, files: &[ParsedFile]) -> Result<Self> {
        let index_dir = vault_root.join(".zetl").join("search");
        std::fs::create_dir_all(&index_dir)
            .with_context(|| format!("creating search index directory {index_dir:?}"))?;

        let index = if index_dir.join("meta.json").exists() {
            Index::open_in_dir(&index_dir)
                .with_context(|| format!("opening existing search index at {index_dir:?}"))?
        } else {
            Index::create_in_dir(&index_dir, make_schema())
                .with_context(|| format!("creating search index at {index_dir:?}"))?
        };

        let fields = fields_from_index(&index)?;

        let mut writer: IndexWriter = index.writer(50_000_000).context("creating index writer")?;
        writer.delete_all_documents().context("clearing search index")?;

        for file in files {
            let abs_path = vault_root.join(&file.path);
            let content = std::fs::read_to_string(&abs_path)
                .with_context(|| format!("reading {:?} for indexing", file.path))?;
            let body = extract_body_text(&content);

            let mut doc = TantivyDocument::default();
            doc.add_text(fields.page_name, &file.page_name);
            doc.add_text(fields.path, &file.path.to_string_lossy());
            doc.add_text(fields.body, &body);
            writer
                .add_document(doc)
                .with_context(|| format!("indexing {:?}", file.path))?;
        }

        writer.commit().context("committing search index")?;

        Ok(Self { index, fields })
    }

    /// Open an existing index from `.zetl/search/`.
    ///
    /// Returns `None` if the directory does not exist.
    pub fn open(vault_root: &Path) -> Result<Option<Self>> {
        let index_dir = vault_root.join(".zetl").join("search");
        if !index_dir.exists() {
            return Ok(None);
        }

        let index = Index::open_in_dir(&index_dir)
            .with_context(|| format!("opening search index at {index_dir:?}"))?;
        let fields = fields_from_index(&index)?;

        Ok(Some(Self { index, fields }))
    }

    /// Execute a full-text query against the body field.
    ///
    /// Returns up to `limit` hits sorted by relevance score descending.
    pub fn query(&self, query_str: &str, limit: usize) -> Result<Vec<SearchHit>> {
        let reader = self
            .index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()
            .context("creating index reader")?;

        let searcher = reader.searcher();
        let query_parser = QueryParser::for_index(&self.index, vec![self.fields.body]);
        let query = query_parser
            .parse_query(query_str)
            .with_context(|| format!("parsing query {query_str:?}"))?;

        let top_docs = searcher
            .search(&query, &TopDocs::with_limit(limit))
            .with_context(|| format!("searching for {query_str:?}"))?;

        let mut hits = Vec::with_capacity(top_docs.len());
        for (score, doc_addr) in top_docs {
            let doc: TantivyDocument = searcher
                .doc(doc_addr)
                .with_context(|| format!("retrieving document for query {query_str:?}"))?;

            let page_name = owned_str(doc.get_first(self.fields.page_name)).to_string();
            let path = owned_str(doc.get_first(self.fields.path)).to_string();

            hits.push(SearchHit {
                page_name,
                path,
                score: score as f64,
            });
        }

        Ok(hits)
    }

    /// Return an `IndexReader` suitable for use across multiple requests in serve mode.
    pub fn reader(&self) -> Result<IndexReader> {
        self.index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()
            .context("creating index reader")
    }
}

/// Build the Tantivy schema.
///
/// - `page_name`: STRING | STORED — indexed (raw token) for filtering, stored for retrieval.
/// - `path`:      STORED          — stored only, not indexed.
/// - `body`:      TEXT | STORED   — full-text indexed with default tokenizer, stored.
fn make_schema() -> Schema {
    let mut builder = SchemaBuilder::new();
    builder.add_text_field("page_name", STRING | STORED);
    builder.add_text_field("path", STORED);
    builder.add_text_field("body", TEXT | STORED);
    builder.build()
}

/// Derive field handles from an already-open index.
fn fields_from_index(index: &Index) -> Result<Fields> {
    let schema = index.schema();
    Ok(Fields {
        page_name: schema
            .get_field("page_name")
            .context("field 'page_name' missing from search schema")?,
        path: schema
            .get_field("path")
            .context("field 'path' missing from search schema")?,
        body: schema
            .get_field("body")
            .context("field 'body' missing from search schema")?,
    })
}

/// Concatenate body-text byte ranges of `content` into a single string, excluding
/// frontmatter, fenced code blocks, inline code spans, and HTML comments.
fn extract_body_text(content: &str) -> String {
    body_text_ranges(content)
        .iter()
        .map(|&(start, end)| &content[start..end])
        .collect::<Vec<_>>()
        .join("")
}

/// Extract a `&str` from an optional `&OwnedValue`, returning `""` on failure.
fn owned_str(value: Option<&OwnedValue>) -> &str {
    match value {
        Some(OwnedValue::Str(s)) => s.as_str(),
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;
    use tempfile::TempDir;

    fn make_parsed_file(dir: &Path, rel_path: &str, content: &str) -> ParsedFile {
        let abs = dir.join(rel_path);
        if let Some(parent) = abs.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&abs, content).unwrap();
        let page_name = std::path::Path::new(rel_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        ParsedFile {
            path: rel_path.into(),
            page_name,
            links: vec![],
            spl_blocks: vec![],
            diagnostics: vec![],
            mtime: SystemTime::UNIX_EPOCH,
            merkle_leaves: vec![],
            file_merkle: None,
        }
    }

    #[test]
    fn open_returns_none_when_no_index() {
        let dir = TempDir::new().unwrap();
        let result = SearchIndex::open(dir.path()).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn build_and_query_basic() {
        let dir = TempDir::new().unwrap();
        let file = make_parsed_file(
            dir.path(),
            "notes.md",
            "# Title\n\nRust programming language.\n",
        );
        let index = SearchIndex::build(dir.path(), &[file]).unwrap();
        let hits = index.query("rust", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].page_name, "notes");
        assert_eq!(hits[0].path, "notes.md");
        assert!(hits[0].score > 0.0);
    }

    #[test]
    fn build_excludes_fenced_code_blocks() {
        let dir = TempDir::new().unwrap();
        let content = "Body text here.\n\n```rust\nfn secret_func() {}\n```\n";
        let file = make_parsed_file(dir.path(), "code.md", content);
        let index = SearchIndex::build(dir.path(), &[file]).unwrap();

        // Content inside a code block must not be indexed.
        let hits = index.query("secret_func", 10).unwrap();
        assert!(hits.is_empty(), "code block content should not be indexed");

        // Body text outside the code block must be indexed.
        let hits = index.query("body", 10).unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn open_finds_existing_index() {
        let dir = TempDir::new().unwrap();
        let file = make_parsed_file(dir.path(), "page.md", "Some content here.");
        SearchIndex::build(dir.path(), &[file]).unwrap();

        let opened = SearchIndex::open(dir.path()).unwrap();
        assert!(opened.is_some());
    }

    #[test]
    fn build_empty_file_list() {
        let dir = TempDir::new().unwrap();
        let index = SearchIndex::build(dir.path(), &[]).unwrap();
        let hits = index.query("anything", 10).unwrap();
        assert!(hits.is_empty());
    }

    #[test]
    fn reader_returns_ok() {
        let dir = TempDir::new().unwrap();
        let index = SearchIndex::build(dir.path(), &[]).unwrap();
        assert!(index.reader().is_ok());
    }

    #[test]
    fn rebuild_clears_previous_documents() {
        let dir = TempDir::new().unwrap();

        // First build with one file.
        let file1 = make_parsed_file(dir.path(), "alpha.md", "Alpha content unique_alpha_word.");
        SearchIndex::build(dir.path(), &[file1]).unwrap();

        // Rebuild with a different file; alpha should no longer be in the index.
        let file2 = make_parsed_file(dir.path(), "beta.md", "Beta content unique_beta_word.");
        let index = SearchIndex::build(dir.path(), &[file2]).unwrap();

        let alpha_hits = index.query("unique_alpha_word", 10).unwrap();
        assert!(alpha_hits.is_empty(), "rebuild should have removed alpha document");

        let beta_hits = index.query("unique_beta_word", 10).unwrap();
        assert_eq!(beta_hits.len(), 1);
    }

    #[test]
    fn query_respects_limit() {
        let dir = TempDir::new().unwrap();
        let files: Vec<ParsedFile> = (0..5)
            .map(|i| {
                make_parsed_file(
                    dir.path(),
                    &format!("file{i}.md"),
                    "shared keyword present here.",
                )
            })
            .collect();
        let index = SearchIndex::build(dir.path(), &files).unwrap();

        let hits = index.query("keyword", 3).unwrap();
        assert!(hits.len() <= 3);
    }
}
