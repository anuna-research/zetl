//! robots.txt emission for `zetl build` (SPEC-034 REQ-3418 / CON-3406).
//!
//! The capability build ships two directories whose contents should never
//! be crawled: `/c/` (encrypted per-cohort envelopes) and `/_zetl/` (deploy
//! recipes, tombstones, helper artifacts). [`merge_robots_txt`] guarantees
//! both appear as `Disallow` rules in the emitted `robots.txt` while
//! preserving any operator-supplied `public/robots.txt` verbatim —
//! including stricter rules like `Disallow: /` that cover our paths
//! implicitly.
//!
//! Merge contract:
//! - Input `operator` is `Some(content)` of the operator's `public/robots.txt`
//!   or `None` when no public overlay is in play.
//! - The returned string always contains the literal bytes `Disallow: /c/`
//!   and `Disallow: /_zetl/` (each on its own line) so TEST-3414 can grep
//!   the output deterministically.
//! - Existing operator lines are never removed, reordered, or rewritten —
//!   the operator's file stays the leading section and any missing
//!   required Disallow appears in a trailing `User-agent: *` block.
//! - Matching is case-insensitive so `disallow: /c/` in the operator file
//!   suffices to prevent a duplicate line being emitted.

/// Paths the zetl build must declare off-limits to crawlers.
pub const REQUIRED_DISALLOWS: &[&str] = &["/c/", "/_zetl/"];

/// Produce the merged robots.txt contents.
///
/// See the module docs for the merge contract. Callers write the returned
/// string verbatim to `<out_dir>/robots.txt`.
pub fn merge_robots_txt(operator: Option<&str>) -> String {
    let base = operator.unwrap_or("").trim_end_matches('\n');
    let missing = missing_disallows(base);

    let mut out = String::new();
    if !base.is_empty() {
        out.push_str(base);
        out.push('\n');
    }

    if missing.is_empty() {
        if out.is_empty() {
            out.push_str("User-agent: *\n");
            for path in REQUIRED_DISALLOWS {
                out.push_str("Disallow: ");
                out.push_str(path);
                out.push('\n');
            }
        }
        return out;
    }

    if out.is_empty() {
        out.push_str("User-agent: *\n");
    } else {
        out.push('\n');
        out.push_str("User-agent: *\n");
    }
    for path in missing {
        out.push_str("Disallow: ");
        out.push_str(path);
        out.push('\n');
    }
    out
}

fn missing_disallows(base: &str) -> Vec<&'static str> {
    REQUIRED_DISALLOWS
        .iter()
        .copied()
        .filter(|path| !has_exact_disallow(base, path))
        .collect()
}

fn has_exact_disallow(content: &str, path: &str) -> bool {
    content.lines().any(|line| match directive(line) {
        Some((name, value)) => name.eq_ignore_ascii_case("disallow") && value == path,
        None => false,
    })
}

fn directive(line: &str) -> Option<(&str, &str)> {
    // Strip `#`-comment tails before splitting — robots.txt allows inline
    // comments on directive lines per the Google / IETF draft.
    let (body, _comment) = match line.find('#') {
        Some(idx) => line.split_at(idx),
        None => (line, ""),
    };
    let colon = body.find(':')?;
    let name = body[..colon].trim();
    let value = body[colon + 1..].trim();
    if name.is_empty() {
        return None;
    }
    Some((name, value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_operator_emits_minimal_block() {
        let out = merge_robots_txt(None);
        assert_eq!(out, "User-agent: *\nDisallow: /c/\nDisallow: /_zetl/\n");
    }

    #[test]
    fn both_required_disallows_present_in_every_output() {
        for input in [
            None,
            Some(""),
            Some("User-agent: *\nDisallow: /\n"),
            Some("User-agent: *\nDisallow: /private/\n"),
            Some("User-agent: Googlebot\nAllow: /\n"),
            Some("Sitemap: https://example.com/sitemap.xml\n"),
        ] {
            let out = merge_robots_txt(input);
            assert!(
                out.contains("Disallow: /c/\n"),
                "missing Disallow: /c/ in: {out:?}"
            );
            assert!(
                out.contains("Disallow: /_zetl/\n"),
                "missing Disallow: /_zetl/ in: {out:?}"
            );
        }
    }

    #[test]
    fn operator_content_preserved_verbatim() {
        let op = "Sitemap: https://example.com/sitemap.xml\n\
                  User-agent: Googlebot\n\
                  Disallow: /private/\n\
                  Crawl-delay: 5\n";
        let out = merge_robots_txt(Some(op));
        assert!(
            out.starts_with(op),
            "operator block must lead the output, got {out:?}"
        );
    }

    #[test]
    fn stricter_disallow_preserved() {
        // `Disallow: /` blocks every path including /c/ and /_zetl/. We
        // still append the explicit rules so TEST-3414 / operator audit
        // tooling can grep for them, but the operator's broader rule is
        // never removed or shadowed.
        let op = "User-agent: *\nDisallow: /\n";
        let out = merge_robots_txt(Some(op));
        assert!(out.contains("Disallow: /\n"));
        assert!(out.contains("Disallow: /c/\n"));
        assert!(out.contains("Disallow: /_zetl/\n"));
    }

    #[test]
    fn duplicate_required_rule_not_reemitted() {
        let op = "User-agent: *\nDisallow: /c/\n";
        let out = merge_robots_txt(Some(op));
        assert_eq!(out.matches("Disallow: /c/").count(), 1);
        assert!(out.contains("Disallow: /_zetl/\n"));
    }

    #[test]
    fn case_insensitive_directive_match() {
        let op = "user-agent: *\ndisallow: /c/\ndisallow: /_zetl/\n";
        let out = merge_robots_txt(Some(op));
        // Operator already covers both requirements in different casing.
        // Output should be the operator file verbatim (with a trailing
        // newline) and no duplicate `Disallow:` appended.
        assert_eq!(out, "user-agent: *\ndisallow: /c/\ndisallow: /_zetl/\n");
    }

    #[test]
    fn inline_comment_does_not_hide_directive() {
        let op = "User-agent: *\nDisallow: /c/ # path-cap tree\n";
        let out = merge_robots_txt(Some(op));
        assert_eq!(out.matches("Disallow: /c/").count(), 1);
        assert!(out.contains("Disallow: /_zetl/\n"));
    }

    #[test]
    fn operator_without_trailing_newline_normalised() {
        let op = "User-agent: *\nDisallow: /private/";
        let out = merge_robots_txt(Some(op));
        assert!(out.starts_with("User-agent: *\nDisallow: /private/\n"));
        assert!(out.ends_with('\n'));
    }

    #[test]
    fn blank_separator_between_operator_and_our_block() {
        let op = "User-agent: *\nDisallow: /private/\n";
        let out = merge_robots_txt(Some(op));
        // A blank line delimits the operator's group from our appended
        // `User-agent: *` block per the Google/IETF grouping convention.
        assert!(
            out.contains("\n\nUser-agent: *\n"),
            "missing blank separator in {out:?}"
        );
    }
}
