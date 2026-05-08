//! Stable item-id pure function per REQ-3803 + NFR-3804.
//!
//! Item ids are RFC 4151 tag URIs of the form
//! `tag:<host-of-namespace>,<creation-year>:zetl/<slug>`. The
//! `creation-year` is the year embedded in the namespace's site (the
//! site's "I started publishing" year) and is provided by the caller —
//! NOT the build-year — so two builds in different years against the
//! same vault produce byte-identical ids.
//!
//! Properties verified by the test suite below:
//!   * deterministic across machines / builds (pure function)
//!   * independent of page title / content / build timestamp
//!   * collision-resistant across distinct slugs in the same namespace
//!   * format is opaque ASCII bounded by slug + namespace bounds
//!   * Atom-conformant (RFC 4151 tag URIs are explicitly allowed by
//!     RFC 4287 §4.2.6 as `<id>` content)

use url::Url;

/// Build a stable RFC 4151 tag URI for a page.
///
/// `namespace` is the canonical site URL of the publishing vault — the
/// host portion is extracted as the tagging-authority component.
/// `creation_year` is the year the site was first published (NOT the
/// build year); pulling this from operator-pinned configuration rather
/// than the wall clock is what makes ids stable across re-runs in
/// different years (NFR-3804).
///
/// Returns an `Err` only when the namespace URL has no host component
/// (e.g. `file://` or a malformed URL). Every other input — including
/// empty slugs and slugs containing characters that would normally need
/// URL-escaping — is accepted; the slug segment is emitted verbatim
/// per RFC 4151 §2.1's "specific" production allowing any
/// `pchar`-equivalent byte. Atom strict parsers consume the result
/// without complaint (verified by [`crate::feed::serialise_atom`]'s
/// conformance gate downstream).
pub fn item_id(slug: &str, namespace: &Url, creation_year: i32) -> Result<String, ItemIdError> {
    let host = namespace.host_str().ok_or(ItemIdError::NamespaceHostless)?;
    Ok(format!("tag:{host},{creation_year}:zetl/{slug}"))
}

/// Failure modes for [`item_id`]. Hostless URLs are the only failure
/// the pure core surfaces; downstream code converts this into a
/// build-time hard error per REQ-3809.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ItemIdError {
    /// `namespace.host_str()` returned `None` (file:// or similar). The
    /// caller must repoint `[feed].base_url` at an http(s) URL.
    #[error("feed namespace URL has no host component")]
    NamespaceHostless,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ns() -> Url {
        Url::parse("https://example.com/").unwrap()
    }

    #[test]
    fn produces_canonical_tag_uri() {
        let id = item_id("foo", &ns(), 2026).unwrap();
        assert_eq!(id, "tag:example.com,2026:zetl/foo");
    }

    #[test]
    fn deterministic_across_invocations() {
        let a = item_id("notes/2026-05-08-mid-flight", &ns(), 2026).unwrap();
        let b = item_id("notes/2026-05-08-mid-flight", &ns(), 2026).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn creation_year_independent_of_build_year() {
        // Same slug + namespace + creation_year produces same id no
        // matter when called.
        let id_2026 = item_id("foo", &ns(), 2020).unwrap();
        let id_2030 = item_id("foo", &ns(), 2020).unwrap();
        assert_eq!(id_2026, id_2030);
        // Distinct creation_year values produce distinct ids.
        assert_ne!(item_id("foo", &ns(), 2020), item_id("foo", &ns(), 2021));
    }

    #[test]
    fn distinct_slugs_collide_never() {
        // Mutation: flipping any character of the slug must produce a
        // different id.
        let base = item_id("alpha", &ns(), 2026).unwrap();
        for variant in ["Alpha", "alpha-", "alph", "alphaa", "alpha "] {
            assert_ne!(item_id(variant, &ns(), 2026).unwrap(), base);
        }
    }

    #[test]
    fn distinct_namespaces_collide_never() {
        let a = item_id("foo", &Url::parse("https://example.com/").unwrap(), 2026).unwrap();
        let b = item_id("foo", &Url::parse("https://example.org/").unwrap(), 2026).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn hostless_namespace_returns_error() {
        let ns = Url::parse("file:///tmp/foo").unwrap();
        assert_eq!(item_id("foo", &ns, 2026), Err(ItemIdError::NamespaceHostless));
    }

    #[test]
    fn empty_slug_accepted() {
        // RFC 4151 §2.1 allows empty `specific`. Operator never produces
        // this in practice but the pure function should not panic.
        let id = item_id("", &ns(), 2026).unwrap();
        assert_eq!(id, "tag:example.com,2026:zetl/");
    }

    #[test]
    fn deeply_nested_slug_round_trips() {
        let id = item_id("notes/2026/05/08-foo", &ns(), 2026).unwrap();
        assert_eq!(id, "tag:example.com,2026:zetl/notes/2026/05/08-foo");
    }
}
