//! Inbound CC-aware republication wiring per task-inbound-cc-
//! republication (REQ-3818..REQ-3823 + ADR-3809 + OBS-3806 +
//! OBS-3807 + T13..T16).
//!
//! Composes `feed::license_resolve`, `feed::republication`,
//! `feed::excerpt`, and the credentials/fetch layers above into a
//! single per-item ingest function. The shell calls
//! [`process_inbound_item`] once per fetched item; the function
//! returns the on-disk frontmatter + body to write under
//! `.zetl/feeds/<sub-id>/inbox/`.

use crate::feed::config::SubscriptionSection;
use crate::feed::excerpt::excerpt;
use crate::feed::license_resolve::{license_resolve, FeedLicenseMetadata, LicenseSource};
use crate::feed::republication::{republication_eligible, SubscriptionPolicy};
use crate::feed::types::{License, RepublicationDecision, RepublicationMode};

/// Wiki-side context used by the eligibility table.
#[derive(Debug, Clone, Default)]
pub struct VaultContext {
    pub self_license: Option<License>,
    pub is_commercial: bool,
}

/// One inbound item handed to [`process_inbound_item`]. Shell-side
/// fetcher already deduped this; we're past identity checks.
#[derive(Debug, Clone)]
pub struct InboundItem {
    pub guid: Option<String>,
    pub title: String,
    pub url: String,
    pub source_feed_url: String,
    pub source_feed_title: String,
    pub original_author: Option<String>,
    pub original_published: String,
    pub content_html: String,
    pub feed_license_metadata: FeedLicenseMetadata,
}

/// Result of processing one inbound item. Carries everything the
/// shell needs to write a Markdown file with attribution frontmatter.
#[derive(Debug, Clone, PartialEq)]
pub struct InboundOutcome {
    /// What the shell should write under `.zetl/feeds/<sub>/inbox/`.
    pub local_file: LocalFileWrite,
    /// What (if anything) the public build should republish on the
    /// next pass.
    pub republication: RepublicationOutcome,
    /// Resolved license; emitted into OBS-3806 alongside the decision.
    pub license: License,
    /// Did an operator override of license declaration drift from the
    /// source feed's declaration? (Audit signal.)
    pub license_drift: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LocalFileWrite {
    /// Markdown body — full content, regardless of republication mode
    /// (the local view always shows the full text per REQ-3818).
    pub body: String,
    /// Frontmatter tree to YAML-serialise atop the body.
    pub frontmatter: AttributionFrontmatter,
}

/// REQ-3822 attribution preservation. Every ingested item carries
/// these keys; the public build refuses to render the page if any are
/// stripped.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AttributionFrontmatter {
    pub source_feed_title: String,
    pub source_feed_url: String,
    pub original_author: Option<String>,
    pub original_published_date: String,
    pub original_item_url: String,
    pub license: String,
    pub license_url: Option<String>,
    /// Set on the next pass when the source feed drops the item.
    /// `None` when the item is still present in the upstream feed.
    pub retracted_by_source: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RepublicationOutcome {
    /// Item is private to the operator's local vault view only.
    Suppress { rationale: String },
    /// Public republication is permitted; carry the rendered body.
    Excerpt { decision: RepublicationDecision, body: String },
    /// Full content republication permitted.
    Full { decision: RepublicationDecision },
}

/// Process one inbound item end-to-end. Pure: takes everything as
/// input, performs no I/O, returns a structured outcome.
pub fn process_inbound_item(
    item: &InboundItem,
    subscription: &SubscriptionSection,
    vault: &VaultContext,
    excerpt_word_count: usize,
) -> InboundOutcome {
    let resolution = license_resolve(
        &item.feed_license_metadata,
        subscription.license.as_deref(),
    );
    let license = resolution.effective.clone();

    let policy = SubscriptionPolicy {
        republish: subscription.republish,
        mode: match subscription.republish_mode.as_deref() {
            Some("full") => RepublicationMode::FullAllowed,
            _ => RepublicationMode::ExcerptOnly,
        },
        i_have_permission: subscription.i_have_permission,
    };
    let decision = republication_eligible(
        &license,
        &policy,
        vault.self_license.as_ref(),
        vault.is_commercial,
    );

    let republication = match decision.mode {
        RepublicationMode::Deny => RepublicationOutcome::Suppress {
            rationale: format!("{:?}", decision.rationale),
        },
        RepublicationMode::ExcerptOnly => {
            let body = excerpt(&item.content_html, excerpt_word_count);
            RepublicationOutcome::Excerpt {
                decision: decision.clone(),
                body,
            }
        }
        RepublicationMode::FullAllowed => RepublicationOutcome::Full {
            decision: decision.clone(),
        },
    };

    let frontmatter = AttributionFrontmatter {
        source_feed_title: item.source_feed_title.clone(),
        source_feed_url: item.source_feed_url.clone(),
        original_author: item.original_author.clone(),
        original_published_date: item.original_published.clone(),
        original_item_url: item.url.clone(),
        license: license.as_spdx(),
        license_url: license_url(&license),
        retracted_by_source: None,
    };
    let local_file = LocalFileWrite {
        body: item.content_html.clone(),
        frontmatter,
    };

    let license_drift = match resolution.source {
        LicenseSource::OperatorOverride => resolution
            .extracted
            .as_ref()
            .map(|e| e != &license)
            .unwrap_or(false),
        _ => false,
    };

    InboundOutcome {
        local_file,
        republication,
        license,
        license_drift,
    }
}

/// Map a [`License`] to its canonical CC URL where one exists.
pub fn license_url(license: &License) -> Option<String> {
    Some(match license {
        License::Cc0_1_0 => "https://creativecommons.org/publicdomain/zero/1.0/".to_string(),
        License::CcBy4_0 => "https://creativecommons.org/licenses/by/4.0/".to_string(),
        License::CcBy3_0 => "https://creativecommons.org/licenses/by/3.0/".to_string(),
        License::CcBySa4_0 => "https://creativecommons.org/licenses/by-sa/4.0/".to_string(),
        License::CcByNc4_0 => "https://creativecommons.org/licenses/by-nc/4.0/".to_string(),
        License::CcByNd4_0 => "https://creativecommons.org/licenses/by-nd/4.0/".to_string(),
        _ => return None,
    })
}

/// REQ-3823 retraction propagation. When the source feed no longer
/// emits an item we previously persisted, the build pass calls this
/// to mark the local file's frontmatter and remove from any
/// republished build.
pub fn mark_retracted(frontmatter: &mut AttributionFrontmatter, at_iso8601: &str) {
    frontmatter.retracted_by_source = Some(at_iso8601.to_string());
}

/// Verify attribution frontmatter is intact per REQ-3822. Returns
/// `Err` listing every required field that's missing.
pub fn assert_attribution_intact(fm: &AttributionFrontmatter) -> Result<(), Vec<&'static str>> {
    let mut missing = Vec::new();
    if fm.source_feed_title.is_empty() {
        missing.push("source_feed_title");
    }
    if fm.source_feed_url.is_empty() {
        missing.push("source_feed_url");
    }
    if fm.original_published_date.is_empty() {
        missing.push("original_published_date");
    }
    if fm.original_item_url.is_empty() {
        missing.push("original_item_url");
    }
    if fm.license.is_empty() {
        missing.push("license");
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(missing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feed::config::parse_config;

    fn item() -> InboundItem {
        InboundItem {
            guid: Some("g".to_string()),
            title: "An Article".to_string(),
            url: "https://upstream.example/x".to_string(),
            source_feed_url: "https://upstream.example/feed.xml".to_string(),
            source_feed_title: "Upstream".to_string(),
            original_author: Some("Alice".to_string()),
            original_published: "2026-05-08T00:00:00Z".to_string(),
            content_html: format!("<p>{}</p>", "word ".repeat(200)),
            feed_license_metadata: FeedLicenseMetadata {
                atom_link_license_href: Some(
                    "https://creativecommons.org/licenses/by-sa/4.0/".to_string(),
                ),
                ..Default::default()
            },
        }
    }

    fn sub(republish: bool, mode: &str) -> SubscriptionSection {
        let body = format!(
            r#"
            [[subscriptions]]
            id = "x"
            source = "https://upstream.example/feed"
            republish = {republish}
            republish_mode = "{mode}"
            i_have_permission = true
            "#
        );
        parse_config(&body).unwrap().subscriptions.into_iter().next().unwrap()
    }

    #[test]
    fn ccbysa_full_only_when_self_license_compatible() {
        let s = sub(true, "full");
        let v = VaultContext {
            self_license: Some(License::CcBySa4_0),
            is_commercial: false,
        };
        let outcome = process_inbound_item(&item(), &s, &v, 200);
        assert!(matches!(outcome.republication, RepublicationOutcome::Full { .. }));
    }

    #[test]
    fn ccbysa_excerpt_when_vault_self_license_missing() {
        let s = sub(true, "full");
        let v = VaultContext::default();
        let outcome = process_inbound_item(&item(), &s, &v, 200);
        assert!(matches!(outcome.republication, RepublicationOutcome::Excerpt { .. }));
    }

    #[test]
    fn republish_false_suppresses() {
        let s = sub(false, "full");
        let v = VaultContext {
            self_license: Some(License::CcBySa4_0),
            is_commercial: false,
        };
        let outcome = process_inbound_item(&item(), &s, &v, 200);
        assert!(matches!(outcome.republication, RepublicationOutcome::Suppress { .. }));
    }

    #[test]
    fn frontmatter_carries_all_required_attribution_fields() {
        let outcome = process_inbound_item(
            &item(),
            &sub(true, "full"),
            &VaultContext {
                self_license: Some(License::CcBySa4_0),
                is_commercial: false,
            },
            200,
        );
        assert_attribution_intact(&outcome.local_file.frontmatter).unwrap();
        assert_eq!(outcome.local_file.frontmatter.license, "CC-BY-SA-4.0");
        assert!(outcome.local_file.frontmatter.license_url.is_some());
    }

    #[test]
    fn mark_retracted_sets_timestamp() {
        let mut fm = AttributionFrontmatter {
            source_feed_title: "U".to_string(),
            source_feed_url: "https://u/".to_string(),
            original_author: None,
            original_published_date: "2026-05-08T00:00:00Z".to_string(),
            original_item_url: "https://u/x".to_string(),
            license: "CC-BY-SA-4.0".to_string(),
            license_url: None,
            retracted_by_source: None,
        };
        mark_retracted(&mut fm, "2026-05-09T00:00:00Z");
        assert_eq!(fm.retracted_by_source.as_deref(), Some("2026-05-09T00:00:00Z"));
    }

    #[test]
    fn license_drift_recorded_when_operator_overrides() {
        let mut s = sub(true, "full");
        s.license = Some("CC-BY-4.0".to_string());
        let outcome = process_inbound_item(
            &item(),
            &s,
            &VaultContext::default(),
            200,
        );
        assert!(outcome.license_drift);
        assert_eq!(outcome.license, License::CcBy4_0);
    }

    #[test]
    fn assert_attribution_intact_lists_missing_fields() {
        let bad = AttributionFrontmatter {
            source_feed_title: "".to_string(),
            source_feed_url: "".to_string(),
            original_author: None,
            original_published_date: "x".to_string(),
            original_item_url: "x".to_string(),
            license: "x".to_string(),
            license_url: None,
            retracted_by_source: None,
        };
        let missing = assert_attribution_intact(&bad).unwrap_err();
        assert!(missing.contains(&"source_feed_title"));
        assert!(missing.contains(&"source_feed_url"));
    }
}
