//! SPEC-032 TEST-3213 — Traceability matrix gate.
//!
//! Parses SPEC-032 §14 and verifies that the requirements-to-tests
//! mapping stays honest as the spec evolves:
//!
//!   1. Every row in §14 names at least one `TEST-XXXX` (or an
//!      explicit `(process — ...)` marker — the only allowed
//!      exception, currently used for NFR-3206 AST-schema
//!      versioning which is enforced via CHANGELOG review).
//!   2. Every `### REQ-XXXX` / `### NFR-XXXX` section header in the
//!      spec has a row in §14 (the spec's own self-check claim is
//!      "every REQ has at least one TEST reference" — this enforces
//!      the claim programmatically).
//!   3. Every row in §14 names an `id` that has a matching
//!      section header in the spec (no phantom rows).
//!   4. Every `### TEST-XXXX` section header in §8 is referenced
//!      from at least one row in §14 (no orphan test strategies
//!      that don't trace back to a requirement).
//!
//! The table-test-cell grammar this file accepts (empirically
//! derived from the spec as of v1 draft):
//!
//! - A cell MAY contain one-or-more comma-separated tokens.
//! - A token MAY be written with or without the `TEST-` prefix —
//!   the first column fixes the prefix, so `TEST-3201, 3201-perf`
//!   normalises to `{TEST-3201, TEST-3201-perf}`.
//! - A token MAY carry a trailing parenthetical annotation
//!   (`TEST-3208 (memory branch)`); the parenthetical is treated
//!   as commentary and stripped.
//! - A cell whose text starts with `(process` is interpreted as
//!   explicit non-test coverage and exempted from rule #1.

use std::collections::BTreeSet;

const SPEC_PATH: &str = "specs/SPEC-032.md";

#[derive(Debug, Clone)]
struct MatrixRow {
    id: String,
    line: usize,
    raw_tests: String,
    tests: BTreeSet<String>,
    process_marker: bool,
}

fn read_spec() -> String {
    std::fs::read_to_string(SPEC_PATH).unwrap_or_else(|e| panic!("cannot read {SPEC_PATH}: {e}"))
}

/// Return the inclusive-start / exclusive-end line indices of §14.
fn traceability_section(src: &str) -> (usize, usize) {
    let total = src.lines().count();
    let mut start: Option<usize> = None;
    let mut end = total;
    for (i, line) in src.lines().enumerate() {
        if line.starts_with("## 14. ") {
            start = Some(i);
        } else if start.is_some() && line.starts_with("## ") {
            end = i;
            break;
        }
    }
    (start.expect("SPEC-032 §14 header `## 14. ` not found"), end)
}

/// Normalise one cell-token into a canonical `TEST-XXXX[-suffix]`
/// identifier, stripping trailing parenthetical annotations like
/// ` (memory branch)` so equality comparisons work against §8
/// section headers.
fn normalise_test_token(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() || raw == "—" || raw == "-" {
        return None;
    }
    // Drop trailing parenthetical annotation (everything from the
    // first `(` onward) then strip residual whitespace.
    let without_paren = raw.split('(').next().unwrap_or(raw).trim();
    if without_paren.is_empty() {
        return None;
    }
    let first_token = without_paren
        .split_whitespace()
        .next()
        .unwrap_or(without_paren);
    let normalised = if first_token.starts_with("TEST-") {
        first_token.to_string()
    } else if first_token
        .chars()
        .next()
        .map_or(false, |c| c.is_ascii_digit())
    {
        // Spec uses comma-list shorthand: `TEST-3201, 3201-perf`.
        format!("TEST-{first_token}")
    } else {
        // Unknown token shape — surface as-is so the test fails
        // loudly on malformed cells rather than silently masking.
        first_token.to_string()
    };
    Some(normalised)
}

fn parse_trace_table(src: &str) -> Vec<MatrixRow> {
    let (start, end) = traceability_section(src);
    let mut rows = Vec::new();
    for (offset, line) in src.lines().skip(start).take(end - start).enumerate() {
        let raw = line.trim_start();
        if !raw.starts_with("| REQ-") && !raw.starts_with("| NFR-") {
            continue;
        }
        let cells: Vec<&str> = raw.split('|').map(str::trim).collect();
        // cells: ["", "REQ-3201", "TEST-3201, ...", "CON-3201", "ADR-3205", "OBS-3201", ""]
        if cells.len() < 3 {
            continue;
        }
        let id = cells[1].to_string();
        let tests_cell = cells[2].to_string();
        let process_marker = tests_cell.starts_with("(process");
        let mut tests = BTreeSet::new();
        if !process_marker {
            for tok in tests_cell.split(',') {
                if let Some(norm) = normalise_test_token(tok) {
                    tests.insert(norm);
                }
            }
        }
        rows.push(MatrixRow {
            id,
            line: start + offset + 1,
            raw_tests: tests_cell,
            tests,
            process_marker,
        });
    }
    rows
}

/// Collect `### <PREFIX>XXXX` section IDs from the spec.
/// Returns the prefix-stripped full id ("REQ-3201", "NFR-3208", "TEST-3212a", ...).
fn section_headers_prefixed(src: &str, prefix: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for line in src.lines() {
        if let Some(rest) = line.strip_prefix("### ") {
            if rest.starts_with(prefix) {
                let id = rest.split(':').next().unwrap_or(rest).trim();
                out.insert(id.to_string());
            }
        }
    }
    out
}

/// Expand combined TEST-headers like `### TEST-3212a/b/c:` into
/// the flat set `{TEST-3212a, TEST-3212b, TEST-3212c}`.
fn test_section_headers(src: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for line in src.lines() {
        let Some(rest) = line.strip_prefix("### TEST-") else {
            continue;
        };
        let title = rest.split(':').next().unwrap_or(rest).trim();
        if title.contains('/') {
            let mut parts = title.split('/');
            let first = parts.next().unwrap_or("");
            // first = "3212a"; stem = "3212" (strip trailing alphabetic).
            let stem: String = first
                .chars()
                .take_while(|c| !c.is_ascii_alphabetic())
                .collect();
            out.insert(format!("TEST-{first}"));
            for suffix in parts {
                out.insert(format!("TEST-{stem}{suffix}"));
            }
        } else {
            out.insert(format!("TEST-{title}"));
        }
    }
    out
}

#[test]
fn test_3213_every_matrix_row_has_at_least_one_test() {
    let src = read_spec();
    let rows = parse_trace_table(&src);
    assert!(
        !rows.is_empty(),
        "SPEC-032 §14: parser found no REQ/NFR rows; \
         check that table row prefixes still match `| REQ-` / `| NFR-`"
    );

    let mut offenders = Vec::new();
    for row in &rows {
        if row.tests.is_empty() && !row.process_marker {
            offenders.push(format!(
                "  {} (SPEC-032:{}): Tests column is `{}` — \
                 add a TEST reference or an explicit `(process — ...)` marker",
                row.id, row.line, row.raw_tests
            ));
        }
    }
    assert!(
        offenders.is_empty(),
        "SPEC-032 §14 orphaned rows (no linked test):\n{}",
        offenders.join("\n")
    );
}

#[test]
fn test_3213_every_req_and_nfr_section_has_matrix_row() {
    let src = read_spec();
    let rows = parse_trace_table(&src);
    let ids_in_table: BTreeSet<String> = rows.iter().map(|r| r.id.clone()).collect();

    let mut sections = section_headers_prefixed(&src, "REQ-");
    sections.extend(section_headers_prefixed(&src, "NFR-"));

    let missing: Vec<String> = sections
        .iter()
        .filter(|id| !ids_in_table.contains(*id))
        .cloned()
        .collect();

    assert!(
        missing.is_empty(),
        "SPEC-032: REQ/NFR section headers without a §14 traceability row: {missing:?}"
    );
}

#[test]
fn test_3213_no_phantom_ids_in_matrix() {
    let src = read_spec();
    let rows = parse_trace_table(&src);
    let mut sections = section_headers_prefixed(&src, "REQ-");
    sections.extend(section_headers_prefixed(&src, "NFR-"));

    let mut phantoms = Vec::new();
    for row in &rows {
        if !sections.contains(&row.id) {
            phantoms.push(format!("  {} (SPEC-032:{})", row.id, row.line));
        }
    }

    assert!(
        phantoms.is_empty(),
        "SPEC-032 §14 rows reference IDs with no matching `### REQ-` / `### NFR-` \
         section header:\n{}",
        phantoms.join("\n")
    );
}

#[test]
fn test_3213_every_test_section_is_referenced_from_matrix() {
    let src = read_spec();
    let defined = test_section_headers(&src);
    let rows = parse_trace_table(&src);

    let mut referenced: BTreeSet<String> = BTreeSet::new();
    for r in &rows {
        referenced.extend(r.tests.iter().cloned());
    }

    let orphans: Vec<String> = defined
        .iter()
        .filter(|t| !referenced.contains(*t))
        .cloned()
        .collect();

    assert!(
        orphans.is_empty(),
        "SPEC-032 §8: TEST-XXXX sections defined but never linked from §14 \
         (add them to a REQ/NFR row, or remove the section if obsolete): {orphans:?}"
    );
}
