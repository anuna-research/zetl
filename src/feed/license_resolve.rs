//! License-metadata extraction + canonicalisation per REQ-3814 +
//! REQ-3819 + ADR-3809.
//!
//! Pure: works only on already-parsed metadata, never reaches out to the
//! network. Recognises every CC URL form variant documented in
//! `research/SPEC-038-license-policy.md`; unknown signals fall back to
//! `License::Unknown` so REQ-3820's default-deny path engages.
//!
//! Operator overrides (`[[subscriptions]] license = "..."`) are
//! authoritative per CON-3811: when set, the extracted value is recorded
//! only as observability metadata (drift signal) and the operator value
//! wins.

use crate::feed::types::License;

/// Aggregated license-bearing metadata extracted from a parsed feed.
/// Order in this struct matches the priority order [`license_resolve`]
/// walks; downstream callers populate as many fields as the source
/// supplies.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FeedLicenseMetadata {
    /// `<atom:link rel=license href="...">`. Highest priority because
    /// it's the IETF-recommended location.
    pub atom_link_license_href: Option<String>,
    /// `<atom:rights>` body text. May be free-form ("All rights
    /// reserved") or a recognisable license URL or short identifier.
    pub atom_rights: Option<String>,
    /// `<dc:rights>` body text (Dublin Core extension).
    pub dc_rights: Option<String>,
    /// Channel-level RSS `<copyright>` body text.
    pub rss_copyright: Option<String>,
}

/// Decision returned by [`license_resolve`] alongside the chosen
/// [`License`]. The `extracted` field carries whatever the metadata
/// sources said even when an operator override wins, so OBS-3806 can
/// surface drift between operator declaration and feed reality.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LicenseResolution {
    /// The license used by [`crate::feed::republication`].
    pub effective: License,
    /// What the metadata sources actually said. `None` when no source
    /// produced a recognisable signal.
    pub extracted: Option<License>,
    /// Where the effective decision came from (audit trail).
    pub source: LicenseSource,
}

/// Provenance of a [`LicenseResolution::effective`] value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LicenseSource {
    /// Operator override from `[[subscriptions]] license` won.
    OperatorOverride,
    /// Resolved from `<atom:link rel=license>`.
    AtomLinkLicense,
    /// Resolved from `<atom:rights>`.
    AtomRights,
    /// Resolved from `<dc:rights>`.
    DcRights,
    /// Resolved from RSS channel-level `<copyright>`.
    RssCopyright,
    /// Nothing parseable found anywhere; default-deny per REQ-3820.
    NoSignal,
}

/// Resolve the license a feed declares to a canonical SPDX-form
/// [`License`].
///
/// Walks priority order: atom:link rel=license > atom:rights >
/// dc:rights > RSS copyright. The first source producing anything other
/// than `License::Unknown` wins; if every source returns `Unknown` the
/// resolution falls back to `License::Unknown` from `NoSignal` source.
///
/// `operator_override` short-circuits the walk: when `Some`, that value
/// is parsed via [`License::from_spdx`] and used as the effective
/// license. The metadata sources are still walked so [`extracted`] can
/// record drift.
pub fn license_resolve(
    metadata: &FeedLicenseMetadata,
    operator_override: Option<&str>,
) -> LicenseResolution {
    let extracted_walk = walk_metadata(metadata);
    let extracted = extracted_walk.as_ref().map(|(lic, _src)| lic.clone());

    if let Some(spdx) = operator_override {
        return LicenseResolution {
            effective: License::from_spdx(spdx),
            extracted,
            source: LicenseSource::OperatorOverride,
        };
    }

    match extracted_walk {
        Some((lic, src)) => LicenseResolution {
            effective: lic,
            extracted,
            source: src,
        },
        None => LicenseResolution {
            effective: License::Unknown,
            extracted: None,
            source: LicenseSource::NoSignal,
        },
    }
}

fn walk_metadata(m: &FeedLicenseMetadata) -> Option<(License, LicenseSource)> {
    if let Some(href) = &m.atom_link_license_href {
        if let Some(lic) = canonicalise_license_url(href) {
            return Some((lic, LicenseSource::AtomLinkLicense));
        }
    }
    for (text, src) in [
        (&m.atom_rights, LicenseSource::AtomRights),
        (&m.dc_rights, LicenseSource::DcRights),
        (&m.rss_copyright, LicenseSource::RssCopyright),
    ] {
        if let Some(text) = text {
            if let Some(lic) = parse_rights_text(text) {
                return Some((lic, src));
            }
        }
    }
    None
}

/// Canonicalise a CC license URL to a [`License`] variant. Recognises
/// the documented variants from `research/SPEC-038-license-policy.md`:
///   * scheme variations (`http://` / `https://`)
///   * trailing-slash variations
///   * legalcode / deed.<lang> suffixes (the human-readable redirect
///     targets some feeds embed)
///   * www subdomain variations
fn canonicalise_license_url(href: &str) -> Option<License> {
    // Strip scheme + leading "www." to normalise.
    let lower = href.trim().to_ascii_lowercase();
    let body = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))
        .unwrap_or(&lower);
    let body = body.strip_prefix("www.").unwrap_or(body);

    // creativecommons.org/licenses/<id>/<version>/[legalcode|deed.xx]
    // creativecommons.org/publicdomain/zero/<version>/...
    if !body.starts_with("creativecommons.org/") {
        return None;
    }
    let rest = &body["creativecommons.org/".len()..];
    // Drop trailing slash + suffix path segments after version.
    let segs: Vec<&str> = rest.split('/').filter(|s| !s.is_empty()).collect();
    match segs.as_slice() {
        ["publicdomain", "zero", "1.0", ..] => Some(License::Cc0_1_0),
        ["licenses", "by", "4.0", ..] => Some(License::CcBy4_0),
        ["licenses", "by", "3.0", ..] => Some(License::CcBy3_0),
        ["licenses", "by-sa", "4.0", ..] => Some(License::CcBySa4_0),
        ["licenses", "by-nc", "4.0", ..] => Some(License::CcByNc4_0),
        ["licenses", "by-nd", "4.0", ..] => Some(License::CcByNd4_0),
        _ => None,
    }
}

/// Best-effort license parse from free-form rights text. Recognises:
///   * embedded canonical CC URLs
///   * SPDX-style identifiers (`CC-BY-4.0`, `CC0-1.0`, ...)
///   * the literal string `Public Domain` or `CC0`
fn parse_rights_text(text: &str) -> Option<License> {
    // 1. URL embedded somewhere in the text.
    for word in text.split_whitespace() {
        let trimmed = word.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '/' && c != ':' && c != '.' && c != '-');
        if let Some(lic) = canonicalise_license_url(trimmed) {
            return Some(lic);
        }
    }
    // 2. SPDX identifier.
    let upper = text.trim().to_ascii_uppercase();
    if upper.contains("CC0-1.0") || upper.contains("CC0 1.0") || upper == "CC0" {
        return Some(License::Cc0_1_0);
    }
    if upper.contains("CC-BY-SA-4.0") || upper.contains("CC BY-SA 4.0") {
        return Some(License::CcBySa4_0);
    }
    if upper.contains("CC-BY-NC-4.0") || upper.contains("CC BY-NC 4.0") {
        return Some(License::CcByNc4_0);
    }
    if upper.contains("CC-BY-ND-4.0") || upper.contains("CC BY-ND 4.0") {
        return Some(License::CcByNd4_0);
    }
    if upper.contains("CC-BY-4.0") || upper.contains("CC BY 4.0") {
        return Some(License::CcBy4_0);
    }
    if upper.contains("CC-BY-3.0") || upper.contains("CC BY 3.0") {
        return Some(License::CcBy3_0);
    }
    if upper.contains("PUBLIC DOMAIN") {
        return Some(License::Cc0_1_0);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_https_with_trailing_slash() {
        let m = FeedLicenseMetadata {
            atom_link_license_href: Some(
                "https://creativecommons.org/licenses/by-sa/4.0/".to_string(),
            ),
            ..Default::default()
        };
        assert_eq!(license_resolve(&m, None).effective, License::CcBySa4_0);
    }

    #[test]
    fn http_no_trailing_slash() {
        let m = FeedLicenseMetadata {
            atom_link_license_href: Some(
                "http://creativecommons.org/licenses/by/4.0".to_string(),
            ),
            ..Default::default()
        };
        assert_eq!(license_resolve(&m, None).effective, License::CcBy4_0);
    }

    #[test]
    fn legalcode_suffix_variants() {
        for variant in [
            "https://creativecommons.org/licenses/by-nc/4.0/legalcode",
            "https://creativecommons.org/licenses/by-nc/4.0/deed.en",
            "https://www.creativecommons.org/licenses/by-nc/4.0/",
        ] {
            let m = FeedLicenseMetadata {
                atom_link_license_href: Some(variant.to_string()),
                ..Default::default()
            };
            assert_eq!(
                license_resolve(&m, None).effective,
                License::CcByNc4_0,
                "variant: {variant}"
            );
        }
    }

    #[test]
    fn cc0_via_publicdomain_path() {
        let m = FeedLicenseMetadata {
            atom_link_license_href: Some(
                "https://creativecommons.org/publicdomain/zero/1.0/".to_string(),
            ),
            ..Default::default()
        };
        assert_eq!(license_resolve(&m, None).effective, License::Cc0_1_0);
    }

    #[test]
    fn priority_atom_link_beats_atom_rights() {
        let m = FeedLicenseMetadata {
            atom_link_license_href: Some(
                "https://creativecommons.org/licenses/by/4.0/".to_string(),
            ),
            atom_rights: Some("CC-BY-NC-4.0".to_string()),
            ..Default::default()
        };
        let r = license_resolve(&m, None);
        assert_eq!(r.effective, License::CcBy4_0);
        assert_eq!(r.source, LicenseSource::AtomLinkLicense);
    }

    #[test]
    fn priority_atom_rights_beats_dc_rights() {
        let m = FeedLicenseMetadata {
            atom_rights: Some("CC-BY-4.0".to_string()),
            dc_rights: Some("CC-BY-NC-4.0".to_string()),
            ..Default::default()
        };
        let r = license_resolve(&m, None);
        assert_eq!(r.effective, License::CcBy4_0);
        assert_eq!(r.source, LicenseSource::AtomRights);
    }

    #[test]
    fn no_signal_falls_back_to_unknown() {
        let m = FeedLicenseMetadata::default();
        let r = license_resolve(&m, None);
        assert_eq!(r.effective, License::Unknown);
        assert_eq!(r.extracted, None);
        assert_eq!(r.source, LicenseSource::NoSignal);
    }

    #[test]
    fn rights_text_with_url_embedded() {
        let m = FeedLicenseMetadata {
            atom_rights: Some(
                "Released under https://creativecommons.org/licenses/by-sa/4.0/".to_string(),
            ),
            ..Default::default()
        };
        assert_eq!(license_resolve(&m, None).effective, License::CcBySa4_0);
    }

    #[test]
    fn operator_override_wins_and_records_drift() {
        let m = FeedLicenseMetadata {
            atom_link_license_href: Some(
                "https://creativecommons.org/licenses/by-nc/4.0/".to_string(),
            ),
            ..Default::default()
        };
        let r = license_resolve(&m, Some("CC-BY-SA-4.0"));
        assert_eq!(r.effective, License::CcBySa4_0);
        assert_eq!(r.source, LicenseSource::OperatorOverride);
        assert_eq!(r.extracted, Some(License::CcByNc4_0));
    }

    #[test]
    fn unrelated_url_in_rights_text_ignored() {
        let m = FeedLicenseMetadata {
            atom_rights: Some("See https://example.com/legal for details.".to_string()),
            ..Default::default()
        };
        assert_eq!(license_resolve(&m, None).effective, License::Unknown);
    }

    #[test]
    fn public_domain_phrase_resolves_to_cc0() {
        let m = FeedLicenseMetadata {
            rss_copyright: Some("Public Domain".to_string()),
            ..Default::default()
        };
        assert_eq!(license_resolve(&m, None).effective, License::Cc0_1_0);
    }
}
