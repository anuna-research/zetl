//! LangSec fuzz / property tests for the SPEC-049 / SPEC-050 trust-boundary recognisers.
//!
//! PROTO-001 Constitutional Principle 14 + §LangSec require fuzzing for every input
//! handler at a trust boundary, and roundtrip/property tests for formats the system both
//! reads and writes. The SPEC-049/050 specs themselves call for executable fuzzing of the
//! sanitiser, the context lint, the topic/type grammars, and the controlled-element
//! renderer before shipping. These property tests are the Rust-side portion; the
//! browser-side controlled-element renderer + value recogniser are tested by
//! `tests/js/islands_runtime.test.mjs` (run under node via `tests/islands_js.rs`), and
//! the capability bridge by the build-side wiring tests.
//!
//! The invariants asserted here are the *security* ones — fail-closed, no-panic,
//! fixed-point, no-executable-output — not functional correctness (that is the
//! example-based integration suites).

#![cfg(all(feature = "content-components", feature = "component-islands"))]

use proptest::prelude::*;
use zetl::web::components::context_lint::lint_component;
use zetl::web::components::directive::{is_valid_directive_name, parse_attr_block, scan, Node};
use zetl::web::components::manifest::{parse_manifest, Manifest};
use zetl::web::components::sanitize::{is_fixed_point, is_safe_url, sanitise_body};
use zetl::web::islands::topic::recognise_topic;
use zetl::web::islands::value_type::parse_type_expr;

/// A content-invocable manifest exposing a scalar (`msg`) and a `url` (`link`) content prop.
fn lint_manifest() -> Manifest {
    parse_manifest(
        "name = \"c\"\ncontent_invocable = true\ncontent_props = [\"msg\", \"link\"]\n\
         [props]\nmsg = { type = \"string\", default = \"\" }\nlink = { type = \"url\", required = false }\n",
        "c",
    )
    .unwrap()
}

// ── CON-4902 body sanitiser ──────────────────────────────────────────────────

proptest! {
    /// The sanitiser never panics on arbitrary input and its output is fixed-point
    /// stable (re-sanitising yields byte-identical output — the mutation-XSS guard).
    #[test]
    fn sanitiser_is_total_and_fixed_point(s in ".{0,400}") {
        let (out, _report) = sanitise_body(&s);
        // re-sanitising the output is a no-op (CON-4902 fixed-point post-condition)
        let (out2, _) = sanitise_body(&out);
        prop_assert_eq!(&out, &out2, "sanitiser not fixed-point on: {:?}", s);
        prop_assert!(is_fixed_point(&s));
    }

    /// No executable vector survives, even embedded in arbitrary surrounding text.
    #[test]
    fn sanitiser_strips_executable_vectors(prefix in ".{0,60}", suffix in ".{0,60}") {
        for vector in [
            "<script>steal()</script>",
            "<img src=x onerror=steal()>",
            "<a href=\"javascript:steal()\">x</a>",
            "<iframe src=\"data:text/html,x\"></iframe>",
            "<svg><script>steal()</script></svg>",
            "<style>@import 'evil'</style>",
            "<a href=\"//evil.example\">x</a>",
        ] {
            let input = format!("{prefix}{vector}{suffix}");
            let (out, _) = sanitise_body(&input);
            let low = out.to_lowercase();
            prop_assert!(!low.contains("<script"), "script survived: {:?}", out);
            prop_assert!(!low.contains("<iframe"), "iframe survived: {:?}", out);
            prop_assert!(!low.contains("<style"), "style survived: {:?}", out);
            prop_assert!(!low.contains("onerror"), "on* survived: {:?}", out);
            prop_assert!(!low.contains("javascript:"), "js scheme survived: {:?}", out);
            // a protocol-relative href must not survive in an anchor
            prop_assert!(!low.contains("href=\"//"), "protocol-relative href survived: {:?}", out);
        }
    }
}

// ── CON-4902 URL recogniser (shared with the url ptype + CON-5007) ───────────

proptest! {
    /// A dangerous scheme is rejected regardless of case, leading whitespace, or
    /// interior control chars (the canonicalisation obfuscation guard).
    #[test]
    fn dangerous_schemes_always_rejected(
        scheme in prop::sample::select(vec!["javascript", "data", "vbscript", "blob", "file"]),
        lead_ws in "[ \t\r\n]{0,5}",
        rest in "[a-z0-9,/:.]{0,20}",
    ) {
        // interleave a tab/newline inside the scheme to simulate `Java\tscript:`
        let mut obf = String::new();
        for (i, c) in scheme.chars().enumerate() {
            obf.push(if i % 2 == 0 { c.to_ascii_uppercase() } else { c });
            if i == scheme.len() / 2 { obf.push('\t'); }
        }
        let url = format!("{lead_ws}{obf}:{rest}");
        prop_assert!(!is_safe_url(&url), "dangerous scheme accepted: {:?}", url);
    }

    /// is_safe_url never panics, and protocol-relative references are always rejected.
    #[test]
    fn url_recogniser_total_and_rejects_scheme_relative(s in ".{0,80}") {
        let _ = is_safe_url(&s); // no panic
        prop_assert!(!is_safe_url(&format!("//{s}")), "protocol-relative accepted");
    }
}

// ── CON-4901 directive recogniser ────────────────────────────────────────────

fn collect_directive_names(nodes: &[Node], out: &mut Vec<String>) {
    for n in nodes {
        if let Node::Directive(d) = n {
            out.push(d.name.clone());
            collect_directive_names(&d.children, out);
        }
    }
}

proptest! {
    /// scan() never panics and every recognised directive has a grammar-valid name
    /// (fail-closed: malformed openers stay text, they never produce a Directive node).
    #[test]
    fn directive_scan_total_and_names_valid(s in ".{0,400}") {
        let nodes = scan(&s);
        let mut names = Vec::new();
        collect_directive_names(&nodes, &mut names);
        for name in names {
            prop_assert!(is_valid_directive_name(&name), "invalid directive name escaped: {:?}", name);
        }
    }

    /// parse_attr_block never panics; accepted keys are grammar-valid kebab identifiers.
    #[test]
    fn attr_block_total(s in ".{0,120}") {
        if let Ok(attrs) = parse_attr_block(&s) {
            use zetl::web::components::directive::Attr;
            for a in attrs {
                if let Attr::KeyValue { key, .. } | Attr::Flag(key) = a {
                    prop_assert!(is_valid_directive_name(&key), "bad attr key accepted: {:?}", key);
                }
            }
        }
    }
}

// ── CON-5001 topic grammar ───────────────────────────────────────────────────

proptest! {
    /// recognise_topic never panics on arbitrary input.
    #[test]
    fn topic_recogniser_total(s in ".{0,140}") {
        let _ = recognise_topic(&s);
    }

    /// Generated grammar-valid topics are accepted; the `content:` prefix decides domain.
    #[test]
    fn valid_topics_accepted(
        segs in prop::collection::vec("[a-z][a-z0-9-]{0,6}[a-z0-9]", 1..6),
    ) {
        let topic = segs.join(":");
        // segments are >=2 chars, lower-first, no trailing '-', <=6 segments → valid
        let kind = recognise_topic(&topic);
        prop_assert!(kind.is_ok(), "valid topic rejected: {:?} ({:?})", topic, kind);
    }

    /// A trailing-hyphen segment is always rejected (grammar negative).
    #[test]
    fn trailing_hyphen_segment_rejected(stem in "[a-z][a-z0-9]{1,6}") {
        let topic = format!("{stem}-");
        prop_assert!(recognise_topic(&topic).is_err());
    }
}

// ── CON-5005 topic value-type language ───────────────────────────────────────

proptest! {
    /// parse_type_expr never panics on arbitrary input.
    #[test]
    fn type_expr_total(s in ".{0,80}") {
        let _ = parse_type_expr(&s);
    }

    /// The scalar keywords always parse; junk never masquerades as a scalar.
    #[test]
    fn scalar_type_exprs(kw in prop::sample::select(vec!["string", "bool", "int", "number"])) {
        prop_assert!(parse_type_expr(kw).is_ok());
        prop_assert!(parse_type_expr(&format!("{kw}x")).is_err(), "junk scalar accepted");
    }
}

// ── CON-4904 static HTML-context lint (soundness: no tainted value in an unsafe context) ──

proptest! {
    /// A content prop placed in any unsafe HTML context (CSS / JS / on* / unquoted attr /
    /// non-url URL-attr) MUST be rejected at build, regardless of surrounding text — the
    /// lint's no-false-negative soundness claim. (Random `[a-z ]` filler cannot turn an
    /// unsafe context safe.)
    #[test]
    fn lint_rejects_tainted_in_unsafe_context(
        pre in "[a-z ]{0,16}",
        suf in "[a-z ]{0,16}",
        idx in 0usize..6,
    ) {
        let wrappers = [
            "<div style=\"color:@V@\">y</div>",      // CSS (style attr)
            "<style>p{color:@V@}</style>",            // CSS (<style>)
            "<script>var a=@V@;</script>",            // JS (<script>)
            "<button onclick=\"@V@\">y</button>",     // JS (on* attr)
            "<div data-x=@V@>y</div>",                // unquoted attribute
            "<a href=\"@V@\">y</a>",                  // non-url scalar in a URL attribute
        ];
        let body = wrappers[idx].replace("@V@", "{{ props.msg }}");
        let tmpl = format!("{pre}{body}{suf}");
        let m = lint_manifest();
        prop_assert!(lint_component(&tmpl, &m).is_err(), "lint must reject: {tmpl}");
    }

    /// A content value in a sanctioned context (scalar in TEXT/double-quoted-attr; a
    /// `url`-typed prop in a URL attribute) MUST pass — no false positives on safe code.
    #[test]
    fn lint_accepts_safe_placement(pre in "[a-z ]{0,16}", idx in 0usize..3) {
        let m = lint_manifest();
        let tmpl = match idx {
            0 => format!("{pre}<p>{{{{ props.msg }}}}</p>"),                 // scalar in TEXT
            1 => format!("{pre}<div title=\"{{{{ props.msg }}}}\">y</div>"), // scalar in dq-attr
            _ => format!("{pre}<a href=\"{{{{ props.link }}}}\">y</a>"),     // url prop in href
        };
        prop_assert!(lint_component(&tmpl, &m).is_ok(), "lint must accept: {tmpl}");
    }
}
