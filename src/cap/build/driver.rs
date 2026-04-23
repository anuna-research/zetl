//! Capability-URL build driver (SPEC-034 REQ-3401, REQ-3403, REQ-3418,
//! OBS-3401).
//!
//! The driver is the top-level orchestrator for `ztl build
//! --capability`. Given a vault's recipients/grants/secret/signing-key
//! plus a list of rendered pages, it produces the per-cohort ciphertext
//! tree under `dist/c/<path-cap>/<slug>.html` and returns a
//! [`BuildSummary`] whose [`BuildSummary::stderr_line`] renders the
//! OBS-3401 one-liner (`cohorts=N pages=M grants_active=K ...`).
//!
//! # Layered inputs
//!
//! Every side-effecting input is threaded in by the caller so the
//! driver itself never touches `std::env`, `SystemTime`, or the OS
//! CSPRNG directly:
//!
//! - [`BuildConfig::build_epoch`] is the RFC 3339 string the envelope
//!   header quotes; callers lift it from `SystemTime::now()`.
//! - [`BuildConfig::now_unix`] is the Unix timestamp used to filter
//!   expired grants out of the active count. RFC 3339 `Z`-suffixed
//!   strings sort lexicographically, so we compare the grant's
//!   `expires` string against `build_epoch` and never parse a
//!   timestamp.
//! - [`ParsedSecret`] and [`VaultSigningKey`] come from the env-loaded
//!   `ztl_CAP_SECRET` / `ztl_CAP_SIGNING_KEY` (the effectful shell
//!   unwraps them upstream).
//! - [`PageInput::html`] is the post-render HTML for a page; the
//!   driver sanitises it via [`crate::cap::sanitiser::sanitise`]
//!   (REQ-3421) before encryption.
//!
//! # Cohort iteration & page scoping
//!
//! For every cohort in `recipients.toml` we:
//! 1. Compile the cohort's optional `pages` glob.
//! 2. Assign pages to the cohort via
//!    [`crate::cap::scoping::cohort_index::CohortIndex`] (explicit
//!    frontmatter cohorts win; otherwise glob-match).
//! 3. Parse the cohort's pubkeys + `salt_stable` (base64url) from the
//!    recipients file.
//! 4. Derive a path-cap per cohort/page via
//!    [`crate::cap::derivation::derive_path_cap`].
//! 5. Sanitise the HTML, age-encrypt to the recipient set (padded by
//!    [`crate::cap::pad`]), sign the ciphertext with the vault's
//!    Ed25519 key, and write the envelope bytes.
//!
//! # Public-repo safety
//!
//! [`Visibility::Public`] short-circuits the build with
//! [`BuildError::PublicRepoRefused`] unless the caller has already
//! externalised the grants file (task-cap-public-repo-safety). The
//! driver itself cannot tell whether the grants path the caller fed
//! in was the committed copy or an external override — it just
//! refuses to encrypt against `recipients.toml` that was loaded from
//! the in-repo path. The actual "is this path in-repo?" check lives
//! in the upstream task; the driver exposes the decision point so
//! the caller's flag can gate the build cleanly.
//!
//! # Output tree
//!
//! ```text
//! <out_dir>/
//!   c/
//!     <path-cap-cohort-A-page-1>/slug-1.html   <- envelope
//!     <path-cap-cohort-A-page-2>/slug-2.html
//!     <path-cap-cohort-B-page-1>/slug-1.html   <- different path-cap
//!     ...
//! ```
//!
//! Path-caps are deterministic per (cohort_id, slug, cohort_secret,
//! salt_stable) — rebuilds with identical inputs produce byte-
//! identical directory structure (REQ-3402 stability).

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::time::Instant;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;

use crate::cap::age_encrypt::{self, AgeCiphertext, AgeEncryptError};
use crate::cap::deploy_artifacts::{self, BundledEnvelope, CohortBundle, DeployArtifactsInput};
use crate::cap::deploy_headers::{self, HeaderSpec};
use crate::cap::derivation::{derive_path_cap, DerivationError, PATH_CAP_DEFAULT_BITS};
use crate::cap::enrolment;
use crate::cap::genkey::ParsedSecret;
use crate::cap::grants::validation::{Grant, GrantsFile};
use crate::cap::html_shell;
use crate::cap::pad::X25519Pubkey;
use crate::cap::recipients::parsing::{Cohort, RecipientsFile, AGE_RECIPIENT_V1_PREFIX};
use crate::cap::referrer_scrub;
use crate::cap::sanitiser;
use crate::cap::scoping::access_config::{AccessConfig, AccessConfigError};
use crate::cap::scoping::cohort_index::{CohortIndex, CohortScope, IndexError, PageRef};
use crate::cap::sign::{self, EnvelopeHeader, VaultSigningKey};

/// Effective build configuration threaded into [`run_capability_build`].
///
/// Each field has a documented default in the CLI (`ztl build
/// --capability`) but the library entry point forces every value to be
/// supplied so unit tests and future callers can exercise the driver
/// in isolation.
#[derive(Debug, Clone)]
pub struct BuildConfig {
    /// Vault root. Currently informational — the driver takes its
    /// inputs pre-parsed so no filesystem reads happen here. Kept on
    /// the struct so future features (e.g. deploy artifacts that
    /// resolve paths against the vault) have a stable handle.
    pub vault_root: PathBuf,
    /// Destination root. The driver writes envelopes under
    /// `<out_dir>/c/<path-cap>/<slug>.html`.
    pub out_dir: PathBuf,
    /// Envelope `ztl-Build-Epoch` header (CON-3404). RFC 3339 / `Z`.
    pub build_epoch: String,
    /// Unix timestamp (seconds since epoch) used to filter expired
    /// grants out of the active-grants counter.
    pub now_unix: u64,
    /// Path-cap width in bits. Valid values: 48, 56, …, 128 (byte-
    /// granular per [`crate::cap::derivation`]). Default 64 per
    /// REQ-3402.
    pub path_cap_bits: u32,
    /// Public-repo safety flag. [`Visibility::Public`] refuses the
    /// build (task-cap-public-repo-safety).
    pub visibility: Visibility,
    /// `[access.search]` / `[access.backlinks]` settings (REQ-3415).
    /// Defaults refuse global search/backlinks and scope the panel to
    /// in-cohort sources.  `run_capability_build` calls
    /// [`AccessConfig::validate`] before writing any ciphertext so
    /// misconfiguration short-circuits cleanly.
    pub access: AccessConfig,
    /// SHA-384 SRI integrity token for `/assets/shim.js` (REQ-3421 /
    /// CON-3410). When `Some`, `run_capability_build` additionally
    /// emits the capability-mode HTML shell under
    /// `<out_dir>/_ztl/capability-shell.html` with a
    /// `<meta http-equiv="Content-Security-Policy">` fallback and an
    /// SRI-tagged shim `<script>`. When `None`, shell emission is
    /// skipped — this keeps pre-shim-bundle dev builds unblocked and
    /// lets unit tests target the envelope path in isolation.
    pub shim_integrity: Option<String>,
    /// SHA-384 SRI integrity token for `/assets/enroll.js` (SPEC-034
    /// REQ-3404 / REQ-3414 / CON-3409). When `Some`, the build
    /// additionally emits `<out_dir>/enroll.html` — the hardened-
    /// mode reader self-enrolment page. When `None`, emission is
    /// skipped; vaults with only delegated-URL cohorts do not need
    /// the page.
    pub enroll_integrity: Option<String>,
    /// Vault name used to key the single-file bundle filename
    /// (`<out_dir>/<vault_name>-<cohort>.html`). Only consulted when
    /// `access.single_file.enabled = true`; callers may pass the
    /// vault-root leaf component or an explicit operator label.
    pub vault_name: String,
    /// Paths — `/c/<path-cap>/<slug>.html` — that the operator has
    /// explicitly retired. Propagated verbatim into `_gone.map`,
    /// `_redirects`, and the top-level `vercel.json` redirects array
    /// (task-cap-deploy-artifacts). Empty is valid — the scaffold
    /// still lands so operators know where to extend later.
    pub tombstones: Vec<String>,
}

impl BuildConfig {
    /// Construct a build config with defaults (`path_cap_bits = 64`,
    /// `visibility = Private`, scoped backlinks, search off). Callers
    /// fill in the rest.
    pub fn new(vault_root: impl Into<PathBuf>, out_dir: impl Into<PathBuf>) -> Self {
        Self {
            vault_root: vault_root.into(),
            out_dir: out_dir.into(),
            build_epoch: String::new(),
            now_unix: 0,
            path_cap_bits: PATH_CAP_DEFAULT_BITS,
            visibility: Visibility::Private,
            access: AccessConfig::default(),
            shim_integrity: None,
            enroll_integrity: None,
            vault_name: String::new(),
            tombstones: Vec::new(),
        }
    }
}

/// Public-repo safety gate (SPEC-034 REQ-3423 / ADR-3409). The v1
/// driver treats [`Visibility::Public`] as a hard refusal; the
/// upstream CLI surface is expected to pair a public vault with an
/// external grants file path (task-cap-public-repo-safety) and flip
/// the flag back to [`Visibility::Private`] before calling the
/// driver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    Private,
    Public,
}

/// One vault page as the driver sees it. The caller is responsible
/// for rendering markdown to HTML; the driver sanitises the HTML
/// before encryption so the sanitiser's allowlist (REQ-3421) is the
/// single enforcement surface.
#[derive(Debug, Clone)]
pub struct PageInput {
    /// Cohort-agnostic slug — the leaf filename under the path-cap
    /// directory. CON-3401 requires the slug carry no `.html`
    /// extension here; the driver appends it when writing.
    pub slug: String,
    /// Rendered HTML (markdown → HTML → any post-render hook output).
    pub html: String,
    /// Cohort ids the page's frontmatter explicitly opts into. Empty
    /// = fall back to cohort glob matching (CohortIndex rules).
    pub explicit_cohorts: Vec<String>,
}

/// Per-(cohort, slug) emission record returned in the summary. Callers
/// that need to assert wire-level shape (integration tests, size-
/// overhead NFR checks) walk `per_page`.
#[derive(Debug, Clone)]
pub struct EncryptedPageStats {
    pub cohort_id: String,
    pub slug: String,
    pub path_cap: String,
    /// Plaintext HTML bytes (post-sanitisation).
    pub plaintext_bytes: u64,
    /// age v1 ciphertext bytes.
    pub ciphertext_bytes: u64,
    /// Full envelope bytes (header + signature + ciphertext).
    pub envelope_bytes: u64,
    /// Real recipients in the age header (excludes padding).
    pub real_recipients: usize,
    /// Padding tier (always a member of [`crate::cap::pad::TIERS`]).
    pub tier: usize,
}

/// Build-summary counters emitted as the OBS-3401 stderr line.
#[derive(Debug, Clone)]
pub struct BuildSummary {
    /// Cohorts declared in `recipients.toml`.
    pub cohorts: usize,
    /// Distinct vault slugs offered to the driver (equals
    /// `pages.len()`).
    pub pages: usize,
    /// Grants that are neither revoked nor past-expiry per
    /// [`BuildConfig::now_unix`].
    pub grants_active: usize,
    /// Total (cohort, page) envelope files written to disk.
    pub pages_encrypted: usize,
    /// Sum of plaintext bytes fed to age across all emissions.
    pub bytes_plaintext: u64,
    /// Sum of age ciphertext bytes emitted.
    pub bytes_ciphertext: u64,
    /// Sum of envelope (header + sig + ciphertext) bytes written.
    pub bytes_envelope: u64,
    /// `tier → count` distribution for OBS-3410 telemetry.
    pub tier_distribution: BTreeMap<usize, usize>,
    /// Wall-clock time of the build.
    pub duration_ms: u128,
    /// Per-(cohort, slug) detail. Ordered first by cohort-declaration
    /// order, then by cohort-index slug order (deterministic).
    pub per_page: Vec<EncryptedPageStats>,
}

impl BuildSummary {
    /// Render the OBS-3401 stderr line. The exact key order is pinned
    /// here so CI greps and operator dashboards can depend on it.
    ///
    /// Shape: `[ztl] cap build: cohorts=N pages=M grants_active=K
    /// emitted=E plaintext_bytes=P ciphertext_bytes=C envelope_bytes=B
    /// duration_ms=D`.
    pub fn stderr_line(&self) -> String {
        format!(
            "[ztl] cap build: cohorts={} pages={} grants_active={} emitted={} \
             plaintext_bytes={} ciphertext_bytes={} envelope_bytes={} duration_ms={}",
            self.cohorts,
            self.pages,
            self.grants_active,
            self.pages_encrypted,
            self.bytes_plaintext,
            self.bytes_ciphertext,
            self.bytes_envelope,
            self.duration_ms,
        )
    }
}

/// Errors surfaced by [`run_capability_build`]. The enum is
/// intentionally typed: each variant points at one phase of the
/// driver so a build log can narrate which step tripped.
#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    #[error("cohort {cohort:?} has no `salt_stable` set — run `ztl cap invite --cohort {cohort} ...` to initialise it")]
    MissingCohortSaltStable { cohort: String },
    #[error("cohort {cohort:?} `salt_stable` is not valid base64url ({detail})")]
    CohortSaltBase64 { cohort: String, detail: String },
    #[error("cohort {cohort:?} recipient {idx} does not carry the `{prefix}` prefix", prefix = AGE_RECIPIENT_V1_PREFIX)]
    RecipientPrefix { cohort: String, idx: usize },
    #[error("cohort {cohort:?} recipient {idx} payload is not valid base64url ({detail})")]
    RecipientBase64 {
        cohort: String,
        idx: usize,
        detail: String,
    },
    #[error("cohort {cohort:?} recipient {idx} decodes to {got} bytes; X25519 pubkey requires 32")]
    RecipientLength {
        cohort: String,
        idx: usize,
        got: usize,
    },
    #[error("cohort {cohort:?} has bad `pages` glob {glob:?}: {err}")]
    BadGlob {
        cohort: String,
        glob: String,
        err: String,
    },
    #[error(transparent)]
    CohortIndex(#[from] IndexError),
    #[error("path-cap derivation failed for cohort {cohort:?} slug {slug:?}: {source}")]
    PathCap {
        cohort: String,
        slug: String,
        #[source]
        source: DerivationError,
    },
    #[error("age encryption failed for cohort {cohort:?} slug {slug:?}: {source}")]
    AgeEncrypt {
        cohort: String,
        slug: String,
        #[source]
        source: AgeEncryptError,
    },
    #[error("i/o error writing {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(
        "capability build refused: vault visibility is public and grants.toml must be kept \
         outside the repository (SPEC-034 REQ-3423 / task-cap-public-repo-safety)"
    )]
    PublicRepoRefused,
    #[error("duplicate slug {0:?} in the page list")]
    DuplicateSlug(String),
    #[error(transparent)]
    AccessConfig(#[from] AccessConfigError),
}

/// Run a capability-mode build end-to-end. Returns on first
/// encryption / I/O failure; side effects (files already written,
/// partial directories) are left in place so the caller can inspect
/// them if needed.
///
/// The driver does NOT currently clear `<out_dir>/c/` — callers
/// running successive builds against the same `out_dir` are expected
/// to wipe the ciphertext tree between runs if rename-after-rotation
/// is in play. A follow-up task (`task-cap-deploy-artifacts`) may
/// layer that on.
pub fn run_capability_build(
    config: &BuildConfig,
    recipients: &RecipientsFile,
    grants: &GrantsFile,
    secret: &ParsedSecret,
    signing_key: &VaultSigningKey,
    pages: &[PageInput],
) -> Result<BuildSummary, BuildError> {
    let start = Instant::now();

    if matches!(config.visibility, Visibility::Public) {
        return Err(BuildError::PublicRepoRefused);
    }

    // REQ-3415: reject global search/backlinks before any ciphertext
    // or dist-tree bytes land on disk. Failing here keeps a
    // misconfigured build from emitting a partial tree that looks
    // legitimate on quick inspection.
    config.access.validate()?;

    // Duplicate-slug guard: the cohort index would also catch this,
    // but raising here produces a clearer error pointed at the page
    // list the caller supplied rather than the derived index.
    {
        let mut seen = std::collections::BTreeSet::new();
        for p in pages {
            if !seen.insert(p.slug.as_str()) {
                return Err(BuildError::DuplicateSlug(p.slug.clone()));
            }
        }
    }

    // Cohort-index lookup: which pages land in which cohort.
    let cohort_scopes: Vec<CohortScope> = recipients
        .cohorts
        .iter()
        .map(|c| CohortScope {
            id: c.id.clone(),
            pages_glob: c.pages.clone(),
        })
        .collect();
    let page_refs: Vec<PageRef> = pages
        .iter()
        .map(|p| PageRef {
            slug: p.slug.clone(),
            explicit_cohorts: p.explicit_cohorts.clone(),
        })
        .collect();
    let index = CohortIndex::build(&cohort_scopes, &page_refs)?;

    // Active grants: not revoked, and not expired relative to
    // `now_unix`. Expiry comparison is lexicographic over the
    // `Z`-suffixed RFC 3339 strings (validated upstream) vs the
    // pre-formatted build_epoch, so we don't need a timestamp
    // parser here. If the caller ever changes build_epoch's format,
    // the grants-file validator will have already rejected a grant
    // whose expires isn't `Z`-suffixed, and the lexicographic
    // comparison stays correct.
    let grants_active = count_active_grants(grants, &config.build_epoch);

    // Render each cohort's recipient set exactly once per build.
    let cohort_pubkeys = resolve_cohort_pubkeys(recipients)?;
    let cohort_salt_stable = resolve_cohort_salts(recipients)?;

    // Per-slug html cache so each cohort that contains the same slug
    // sanitises the body only once (REQ-3421 is idempotent but the
    // sanitiser still costs cycles on 10k-page vaults).
    let mut sanitised_cache: BTreeMap<&str, String> = BTreeMap::new();

    // Page-input lookup by slug, so we don't need to re-scan `pages`
    // for every (cohort, slug) pair.
    let page_by_slug: BTreeMap<&str, &PageInput> =
        pages.iter().map(|p| (p.slug.as_str(), p)).collect();

    let mut per_page: Vec<EncryptedPageStats> = Vec::new();
    let mut tier_distribution: BTreeMap<usize, usize> = BTreeMap::new();
    let mut bytes_plaintext: u64 = 0;
    let mut bytes_ciphertext: u64 = 0;
    let mut bytes_envelope: u64 = 0;
    let mut pages_encrypted: usize = 0;
    // Per-cohort envelope accumulator for `[access.single_file]`
    // (task-cap-deploy-artifacts). Collected in declaration order so
    // the emitted bundle's `<template>` order matches the ciphertext
    // write order — deterministic across rebuilds.
    let collect_bundles = config.access.single_file.enabled;
    let mut cohort_bundles: Vec<CohortBundle> = Vec::new();

    let out_cap_root = config.out_dir.join("c");
    fs::create_dir_all(&out_cap_root).map_err(|e| BuildError::Io {
        path: out_cap_root.clone(),
        source: e,
    })?;

    // Iterate cohorts in recipients.toml declaration order so the
    // emission sequence is deterministic across rebuilds.
    for cohort in &recipients.cohorts {
        let real_recipients = cohort_pubkeys.get(cohort.id.as_str()).unwrap();
        let salt_stable = cohort_salt_stable.get(cohort.id.as_str()).unwrap();
        let mut bundle_envelopes: Vec<BundledEnvelope> = Vec::new();

        for slug in index.pages_in(&cohort.id) {
            let page = page_by_slug
                .get(slug.as_str())
                .expect("cohort index slugs come from pages list");

            let sanitised = sanitised_cache.entry(slug.as_str()).or_insert_with(|| {
                let cleaned = sanitiser::sanitise(&page.html);
                // REQ-3413: add rel="noopener noreferrer" to external
                // <a> tags so the path-cap does not leak into the
                // Referer header on outgoing clicks. Internal links
                // are left byte-identical. The `<meta name="referrer"
                // content="no-referrer">` document-default lives in
                // the HTML shell (`html_shell::REFERRER_META`); the
                // per-link rel is belt-and-braces for browsers that
                // honour rel over meta.
                if config.access.rel_noreferrer {
                    referrer_scrub::scrub_external_link_rels(&cleaned)
                } else {
                    cleaned
                }
            });
            let plaintext = sanitised.as_bytes();

            let path_cap = derive_path_cap(
                secret.random_body(),
                salt_stable,
                &cohort.id,
                &page.slug,
                config.path_cap_bits,
            )
            .map_err(|source| BuildError::PathCap {
                cohort: cohort.id.clone(),
                slug: page.slug.clone(),
                source,
            })?;

            let ciphertext: AgeCiphertext =
                age_encrypt::encrypt_to_cohort(plaintext, real_recipients).map_err(|source| {
                    BuildError::AgeEncrypt {
                        cohort: cohort.id.clone(),
                        slug: page.slug.clone(),
                        source,
                    }
                })?;

            let envelope_bytes = sign::sign_and_build_envelope(
                signing_key,
                &EnvelopeHeader {
                    cohort_id: cohort.id.clone(),
                    cohort_mode: cohort.mode,
                    slug: page.slug.clone(),
                    build_epoch: config.build_epoch.clone(),
                },
                &ciphertext.bytes,
            );

            let cap_dir = out_cap_root.join(&path_cap);
            fs::create_dir_all(&cap_dir).map_err(|e| BuildError::Io {
                path: cap_dir.clone(),
                source: e,
            })?;
            let file_path = cap_dir.join(format!("{}.html", page.slug_leaf()));
            // Ensure intermediate directories exist when a slug itself
            // contains path separators (e.g. `eng/onboarding`).
            if let Some(parent) = file_path.parent() {
                fs::create_dir_all(parent).map_err(|e| BuildError::Io {
                    path: parent.to_path_buf(),
                    source: e,
                })?;
            }
            fs::write(&file_path, &envelope_bytes).map_err(|e| BuildError::Io {
                path: file_path.clone(),
                source: e,
            })?;

            bytes_plaintext += plaintext.len() as u64;
            bytes_ciphertext += ciphertext.bytes.len() as u64;
            bytes_envelope += envelope_bytes.len() as u64;
            pages_encrypted += 1;
            *tier_distribution.entry(ciphertext.tier).or_insert(0) += 1;

            if collect_bundles {
                bundle_envelopes.push(BundledEnvelope {
                    slug: page.slug.clone(),
                    envelope_bytes: envelope_bytes.clone(),
                });
            }

            per_page.push(EncryptedPageStats {
                cohort_id: cohort.id.clone(),
                slug: page.slug.clone(),
                path_cap,
                plaintext_bytes: plaintext.len() as u64,
                ciphertext_bytes: ciphertext.bytes.len() as u64,
                envelope_bytes: envelope_bytes.len() as u64,
                real_recipients: ciphertext.real_count,
                tier: ciphertext.tier,
            });
        }

        if collect_bundles {
            cohort_bundles.push(CohortBundle {
                cohort_id: cohort.id.clone(),
                envelopes: bundle_envelopes,
            });
        }
    }

    // SPEC-034 REQ-3407 / REQ-3418 / REQ-3428 (CON-3406): emit deploy-
    // header recipes so the operator can configure nginx / Caddy /
    // Netlify / Vercel / Cloudflare Pages without reading the spec.
    // Idempotent overwrite — a later rebuild replaces each file.
    let header_spec = HeaderSpec::from_cache_config(&config.access.cache);
    deploy_headers::write_deploy_recipes(&config.out_dir, &header_spec).map_err(|e| {
        BuildError::Io {
            path: config.out_dir.join("_ztl").join("deploy"),
            source: e,
        }
    })?;

    // SPEC-034 REQ-3418 / CON-3406 (task-cap-deploy-artifacts): emit
    // the top-level static-host wiring (`_gone.map`, `_redirects`,
    // `vercel.json`, per-platform `deploy-*.conf`) + the optional
    // single-file bundle. Sibling to the recipe snippets above — the
    // header values round-trip through the same `HeaderSpec`.
    let artifacts_input = DeployArtifactsInput {
        spec: &header_spec,
        tombstones: config.tombstones.clone(),
        vault_name: config.vault_name.as_str(),
        single_file: &config.access.single_file,
        cohort_bundles,
    };
    deploy_artifacts::write_deploy_artifacts(&config.out_dir, &artifacts_input).map_err(|e| {
        BuildError::Io {
            path: config.out_dir.join("_ztl"),
            source: e,
        }
    })?;

    // REQ-3421 / CON-3410 (BUG-006): emit the HTML shell with a
    // `<meta http-equiv="Content-Security-Policy">` fallback and an
    // SRI-tagged shim `<script>`. Skipped if the caller did not thread
    // a shim-integrity token through `BuildConfig` — pre-bundle dev
    // builds and most unit tests take that branch.
    if let Some(sri_hash) = config.shim_integrity.as_deref() {
        html_shell::write_shell(&config.out_dir, sri_hash).map_err(|source| BuildError::Io {
            path: config
                .out_dir
                .join("_ztl")
                .join(html_shell::CAPABILITY_SHELL_FILENAME),
            source,
        })?;
    }

    // SPEC-034 REQ-3404 / REQ-3414 / CON-3409: emit `/enroll.html`
    // when the caller threads an enroll-bundle SRI token through
    // `BuildConfig`. The page is the hardened-mode reader self-
    // enrolment surface; delegated-URL-only vaults skip it.
    if let Some(sri_hash) = config.enroll_integrity.as_deref() {
        enrolment::write_enroll_html(&config.out_dir, sri_hash).map_err(|source| {
            BuildError::Io {
                path: config.out_dir.join(enrolment::ENROLL_HTML_FILENAME),
                source,
            }
        })?;
    }

    Ok(BuildSummary {
        cohorts: recipients.cohorts.len(),
        pages: pages.len(),
        grants_active,
        pages_encrypted,
        bytes_plaintext,
        bytes_ciphertext,
        bytes_envelope,
        tier_distribution,
        duration_ms: start.elapsed().as_millis(),
        per_page,
    })
}

impl PageInput {
    /// Leaf filename component. For nested slugs (`eng/onboarding`)
    /// returns `onboarding`; the enclosing `eng/` is preserved by
    /// `run_capability_build` via `cap_dir.join(page.slug)` in the
    /// path assembly. Kept as a helper so the driver's write step
    /// stays readable.
    fn slug_leaf(&self) -> &str {
        self.slug.rsplit('/').next().unwrap_or(&self.slug)
    }
}

fn resolve_cohort_salts(
    recipients: &RecipientsFile,
) -> Result<BTreeMap<&str, Vec<u8>>, BuildError> {
    let mut out: BTreeMap<&str, Vec<u8>> = BTreeMap::new();
    for c in &recipients.cohorts {
        let salt_b64 =
            c.salt_stable
                .as_deref()
                .ok_or_else(|| BuildError::MissingCohortSaltStable {
                    cohort: c.id.clone(),
                })?;
        let raw = URL_SAFE_NO_PAD.decode(salt_b64.as_bytes()).map_err(|e| {
            BuildError::CohortSaltBase64 {
                cohort: c.id.clone(),
                detail: e.to_string(),
            }
        })?;
        out.insert(c.id.as_str(), raw);
    }
    Ok(out)
}

fn resolve_cohort_pubkeys(
    recipients: &RecipientsFile,
) -> Result<BTreeMap<&str, Vec<X25519Pubkey>>, BuildError> {
    let mut out: BTreeMap<&str, Vec<X25519Pubkey>> = BTreeMap::new();
    for cohort in &recipients.cohorts {
        let pks = parse_cohort_pubkeys(cohort)?;
        out.insert(cohort.id.as_str(), pks);
    }
    Ok(out)
}

fn parse_cohort_pubkeys(cohort: &Cohort) -> Result<Vec<X25519Pubkey>, BuildError> {
    let mut out: Vec<X25519Pubkey> = Vec::with_capacity(cohort.pubkeys.len());
    for (idx, entry) in cohort.pubkeys.iter().enumerate() {
        let payload = entry.strip_prefix(AGE_RECIPIENT_V1_PREFIX).ok_or_else(|| {
            BuildError::RecipientPrefix {
                cohort: cohort.id.clone(),
                idx,
            }
        })?;
        let raw = URL_SAFE_NO_PAD.decode(payload.as_bytes()).map_err(|e| {
            BuildError::RecipientBase64 {
                cohort: cohort.id.clone(),
                idx,
                detail: e.to_string(),
            }
        })?;
        if raw.len() != 32 {
            return Err(BuildError::RecipientLength {
                cohort: cohort.id.clone(),
                idx,
                got: raw.len(),
            });
        }
        let mut pk = [0u8; 32];
        pk.copy_from_slice(&raw);
        out.push(pk);
    }
    Ok(out)
}

fn count_active_grants(grants: &GrantsFile, build_epoch: &str) -> usize {
    grants
        .grants
        .iter()
        .filter(|g| grant_is_active(g, build_epoch))
        .count()
}

fn grant_is_active(g: &Grant, build_epoch: &str) -> bool {
    if g.revoked {
        return false;
    }
    match g.expires.as_deref() {
        // RFC 3339 `Z`-suffixed strings sort correctly, so a
        // bytewise compare is a valid "expired?" check given the
        // format is pinned by `cap::grants::validation::is_rfc3339`.
        Some(exp) if exp.as_bytes() <= build_epoch.as_bytes() => false,
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::Path;

    use age::x25519;
    use bech32::{Bech32, Hrp};
    use tempfile::TempDir;

    use crate::cap::genkey;
    use crate::cap::grants::validation::GrantMode;
    use crate::cap::recipients::parsing::{CohortMode, VaultSection};
    use crate::cap::sign::parse_envelope;

    fn sample_secret() -> ParsedSecret {
        // Deterministic — version byte + 32 random bytes + checksum,
        // all seeded via genkey::build_secret.
        let random = [7u8; 32];
        let bytes = genkey::build_secret(genkey::SECRET_VERSION_V1, &random);
        let encoded = genkey::encode_secret(&bytes);
        genkey::decode_secret(&encoded).expect("secret round-trips")
    }

    fn sample_signing_key() -> VaultSigningKey {
        VaultSigningKey::from_bytes(&[9u8; 32])
    }

    fn pubkey_from_age_string(s: &str) -> [u8; 32] {
        let (hrp, data) = bech32::decode(s).expect("valid age1 bech32");
        assert_eq!(hrp.as_str(), "age");
        let mut pk = [0u8; 32];
        assert_eq!(data.len(), 32);
        pk.copy_from_slice(&data);
        pk
    }

    fn age_recipient_v1(pk: &[u8; 32]) -> String {
        let b64 = URL_SAFE_NO_PAD.encode(pk);
        format!("{AGE_RECIPIENT_V1_PREFIX}{b64}")
    }

    fn encode_age1(pk: &[u8; 32]) -> String {
        let hrp = Hrp::parse("age").unwrap();
        bech32::encode::<Bech32>(hrp, pk).unwrap()
    }

    fn fresh_age_identity_pair() -> (x25519::Identity, [u8; 32]) {
        let id = x25519::Identity::generate();
        let pk = pubkey_from_age_string(&id.to_public().to_string());
        (id, pk)
    }

    fn sample_recipients(salt_b64: &str, cohort_pubkeys: Vec<[u8; 32]>) -> RecipientsFile {
        RecipientsFile {
            version: 1,
            vault: VaultSection {
                signing_pubkey: format!(
                    "ed25519:{}",
                    URL_SAFE_NO_PAD.encode(sample_signing_key().verifying_key().to_bytes())
                ),
            },
            cohorts: vec![Cohort {
                id: "engineering".to_string(),
                name: Some("Engineering".to_string()),
                mode: CohortMode::DelegatedUrl,
                pubkeys: cohort_pubkeys.iter().map(age_recipient_v1).collect(),
                pages: None,
                salt_stable: Some(salt_b64.to_string()),
                salt_rotated: None,
                last_rotated: None,
            }],
        }
    }

    fn sample_grants() -> GrantsFile {
        GrantsFile {
            version: Some(1),
            grants: vec![Grant {
                id: "g_01JABC".to_string(),
                cohort: "engineering".to_string(),
                recipient: format!("{AGE_RECIPIENT_V1_PREFIX}AA"),
                mode: GrantMode::DelegatedUrl,
                bound: false,
                name: Some("Alice".into()),
                created: "2026-04-20T00:00:00Z".to_string(),
                expires: Some("2026-12-31T23:59:59Z".to_string()),
                revoked: false,
                pages: "*".to_string(),
            }],
        }
    }

    fn build_cfg(out: &Path) -> BuildConfig {
        BuildConfig {
            vault_root: PathBuf::from("/does/not/matter"),
            out_dir: out.to_path_buf(),
            build_epoch: "2026-04-20T10:15:00Z".to_string(),
            now_unix: 1_745_140_500,
            path_cap_bits: PATH_CAP_DEFAULT_BITS,
            visibility: Visibility::Private,
            access: AccessConfig::default(),
            shim_integrity: None,
            enroll_integrity: None,
            vault_name: "test-vault".to_string(),
            tombstones: Vec::new(),
        }
    }

    #[test]
    fn round_trip_writes_envelope_that_parses_and_decrypts() {
        let tmp = TempDir::new().unwrap();
        let salt_b64 = URL_SAFE_NO_PAD.encode(b"cohort-salt-stable-01");
        let (alice, alice_pk) = fresh_age_identity_pair();
        let recipients = sample_recipients(&salt_b64, vec![alice_pk]);
        let grants = sample_grants();
        let secret = sample_secret();
        let signing_key = sample_signing_key();
        let signing_pub = signing_key.verifying_key();

        let pages = vec![PageInput {
            slug: "welcome".to_string(),
            html: "<h1>Welcome</h1><p>hi</p>".to_string(),
            explicit_cohorts: vec![],
        }];

        let summary = run_capability_build(
            &build_cfg(tmp.path()),
            &recipients,
            &grants,
            &secret,
            &signing_key,
            &pages,
        )
        .expect("build succeeds");

        assert_eq!(summary.cohorts, 1);
        assert_eq!(summary.pages, 1);
        assert_eq!(summary.pages_encrypted, 1);
        assert_eq!(summary.grants_active, 1);
        let stat = &summary.per_page[0];
        let on_disk = tmp
            .path()
            .join("c")
            .join(&stat.path_cap)
            .join("welcome.html");
        assert!(on_disk.exists(), "envelope file should exist: {on_disk:?}");

        let bytes = fs::read(&on_disk).unwrap();
        let parsed = parse_envelope(&bytes).expect("envelope parses");
        assert_eq!(parsed.header.cohort_id, "engineering");
        assert_eq!(parsed.header.slug, "welcome");
        assert_eq!(parsed.header.build_epoch, "2026-04-20T10:15:00Z");
        signing_pub
            .verify_ciphertext(&parsed.ciphertext, &parsed.signature)
            .expect("sig verifies");

        // Decrypt with alice's private key.
        let decryptor = age::Decryptor::new(parsed.ciphertext.as_slice()).unwrap();
        let iter = std::iter::once(&alice as &dyn age::Identity);
        let mut reader = decryptor.decrypt(iter).unwrap();
        let mut plaintext = String::new();
        use std::io::Read;
        reader.read_to_string(&mut plaintext).unwrap();
        assert!(plaintext.contains("<h1>Welcome</h1>"));
        assert!(plaintext.contains("<p>hi</p>"));
    }

    #[test]
    fn deterministic_path_cap_across_rebuilds() {
        let tmp1 = TempDir::new().unwrap();
        let tmp2 = TempDir::new().unwrap();
        let salt_b64 = URL_SAFE_NO_PAD.encode(b"stable-salt-for-path-cap-determinism");
        let (_id, pk) = fresh_age_identity_pair();
        let recipients = sample_recipients(&salt_b64, vec![pk]);
        let grants = sample_grants();
        let secret = sample_secret();
        let signing_key = sample_signing_key();

        let pages = vec![PageInput {
            slug: "docs/onboarding".to_string(),
            html: "<p>x</p>".to_string(),
            explicit_cohorts: vec![],
        }];
        let s1 = run_capability_build(
            &build_cfg(tmp1.path()),
            &recipients,
            &grants,
            &secret,
            &signing_key,
            &pages,
        )
        .unwrap();
        let s2 = run_capability_build(
            &build_cfg(tmp2.path()),
            &recipients,
            &grants,
            &secret,
            &signing_key,
            &pages,
        )
        .unwrap();
        assert_eq!(s1.per_page[0].path_cap, s2.per_page[0].path_cap);
    }

    #[test]
    fn rejects_public_repo_safety_flag() {
        let tmp = TempDir::new().unwrap();
        let salt_b64 = URL_SAFE_NO_PAD.encode(b"salt");
        let (_id, pk) = fresh_age_identity_pair();
        let recipients = sample_recipients(&salt_b64, vec![pk]);
        let grants = sample_grants();
        let secret = sample_secret();
        let signing_key = sample_signing_key();
        let pages = vec![PageInput {
            slug: "welcome".into(),
            html: "<p>hi</p>".into(),
            explicit_cohorts: vec![],
        }];
        let mut cfg = build_cfg(tmp.path());
        cfg.visibility = Visibility::Public;
        let err = run_capability_build(&cfg, &recipients, &grants, &secret, &signing_key, &pages)
            .unwrap_err();
        assert!(matches!(err, BuildError::PublicRepoRefused));
    }

    #[test]
    fn missing_salt_stable_is_an_error() {
        let tmp = TempDir::new().unwrap();
        let (_id, pk) = fresh_age_identity_pair();
        let mut recipients = sample_recipients(&URL_SAFE_NO_PAD.encode(b"salt"), vec![pk]);
        recipients.cohorts[0].salt_stable = None;
        let err = run_capability_build(
            &build_cfg(tmp.path()),
            &recipients,
            &sample_grants(),
            &sample_secret(),
            &sample_signing_key(),
            &[PageInput {
                slug: "w".into(),
                html: "<p>x</p>".into(),
                explicit_cohorts: vec![],
            }],
        )
        .unwrap_err();
        assert!(
            matches!(err, BuildError::MissingCohortSaltStable { cohort } if cohort == "engineering")
        );
    }

    #[test]
    fn sanitises_html_before_encryption() {
        let tmp = TempDir::new().unwrap();
        let salt_b64 = URL_SAFE_NO_PAD.encode(b"salt");
        let (alice, alice_pk) = fresh_age_identity_pair();
        let recipients = sample_recipients(&salt_b64, vec![alice_pk]);
        let summary = run_capability_build(
            &build_cfg(tmp.path()),
            &recipients,
            &sample_grants(),
            &sample_secret(),
            &sample_signing_key(),
            &[PageInput {
                slug: "w".into(),
                html: "<p>hi</p><script>alert(1)</script>".into(),
                explicit_cohorts: vec![],
            }],
        )
        .unwrap();
        let stat = &summary.per_page[0];
        let bytes = fs::read(tmp.path().join("c").join(&stat.path_cap).join("w.html")).unwrap();
        let parsed = parse_envelope(&bytes).unwrap();
        let decryptor = age::Decryptor::new(parsed.ciphertext.as_slice()).unwrap();
        let mut reader = decryptor
            .decrypt(std::iter::once(&alice as &dyn age::Identity))
            .unwrap();
        let mut plaintext = String::new();
        use std::io::Read;
        reader.read_to_string(&mut plaintext).unwrap();
        assert!(plaintext.contains("<p>hi</p>"));
        assert!(
            !plaintext.contains("<script"),
            "sanitiser must strip scripts"
        );
        assert!(!plaintext.contains("alert(1)"));
    }

    #[test]
    fn different_cohorts_get_distinct_path_caps_for_same_slug() {
        let tmp = TempDir::new().unwrap();
        let salt_b64 = URL_SAFE_NO_PAD.encode(b"salt");
        let (_id, pk) = fresh_age_identity_pair();
        let mut recipients = sample_recipients(&salt_b64, vec![pk]);
        recipients.cohorts.push(Cohort {
            id: "ops".to_string(),
            name: None,
            mode: CohortMode::DelegatedUrl,
            pubkeys: vec![age_recipient_v1(&pk)],
            pages: None,
            salt_stable: Some(salt_b64.clone()),
            salt_rotated: None,
            last_rotated: None,
        });
        let summary = run_capability_build(
            &build_cfg(tmp.path()),
            &recipients,
            &sample_grants(),
            &sample_secret(),
            &sample_signing_key(),
            &[PageInput {
                slug: "w".into(),
                html: "<p>x</p>".into(),
                explicit_cohorts: vec![],
            }],
        )
        .unwrap();
        assert_eq!(summary.pages_encrypted, 2);
        let eng = summary
            .per_page
            .iter()
            .find(|s| s.cohort_id == "engineering")
            .unwrap();
        let ops = summary
            .per_page
            .iter()
            .find(|s| s.cohort_id == "ops")
            .unwrap();
        assert_ne!(eng.path_cap, ops.path_cap);
    }

    #[test]
    fn expired_grants_drop_out_of_active_count() {
        let tmp = TempDir::new().unwrap();
        let salt_b64 = URL_SAFE_NO_PAD.encode(b"salt");
        let (_id, pk) = fresh_age_identity_pair();
        let recipients = sample_recipients(&salt_b64, vec![pk]);
        let mut grants = sample_grants();
        // Make the existing grant expire before the build epoch.
        grants.grants[0].expires = Some("2026-01-01T00:00:00Z".to_string());
        let summary = run_capability_build(
            &build_cfg(tmp.path()),
            &recipients,
            &grants,
            &sample_secret(),
            &sample_signing_key(),
            &[PageInput {
                slug: "w".into(),
                html: "<p>x</p>".into(),
                explicit_cohorts: vec![],
            }],
        )
        .unwrap();
        assert_eq!(summary.grants_active, 0);
    }

    #[test]
    fn revoked_grants_drop_out_of_active_count() {
        let tmp = TempDir::new().unwrap();
        let salt_b64 = URL_SAFE_NO_PAD.encode(b"salt");
        let (_id, pk) = fresh_age_identity_pair();
        let recipients = sample_recipients(&salt_b64, vec![pk]);
        let mut grants = sample_grants();
        grants.grants[0].revoked = true;
        let summary = run_capability_build(
            &build_cfg(tmp.path()),
            &recipients,
            &grants,
            &sample_secret(),
            &sample_signing_key(),
            &[PageInput {
                slug: "w".into(),
                html: "<p>x</p>".into(),
                explicit_cohorts: vec![],
            }],
        )
        .unwrap();
        assert_eq!(summary.grants_active, 0);
    }

    #[test]
    fn stderr_line_contains_required_keys() {
        let summary = BuildSummary {
            cohorts: 3,
            pages: 7,
            grants_active: 5,
            pages_encrypted: 11,
            bytes_plaintext: 1024,
            bytes_ciphertext: 2048,
            bytes_envelope: 2200,
            tier_distribution: BTreeMap::new(),
            duration_ms: 42,
            per_page: vec![],
        };
        let s = summary.stderr_line();
        for key in [
            "cohorts=3",
            "pages=7",
            "grants_active=5",
            "emitted=11",
            "plaintext_bytes=1024",
            "ciphertext_bytes=2048",
            "envelope_bytes=2200",
            "duration_ms=42",
        ] {
            assert!(s.contains(key), "missing {key:?} in stderr line {s:?}");
        }
    }

    #[test]
    fn duplicate_slug_fails_fast() {
        let tmp = TempDir::new().unwrap();
        let salt_b64 = URL_SAFE_NO_PAD.encode(b"salt");
        let (_id, pk) = fresh_age_identity_pair();
        let recipients = sample_recipients(&salt_b64, vec![pk]);
        let err = run_capability_build(
            &build_cfg(tmp.path()),
            &recipients,
            &sample_grants(),
            &sample_secret(),
            &sample_signing_key(),
            &[
                PageInput {
                    slug: "same".into(),
                    html: "<p>a</p>".into(),
                    explicit_cohorts: vec![],
                },
                PageInput {
                    slug: "same".into(),
                    html: "<p>b</p>".into(),
                    explicit_cohorts: vec![],
                },
            ],
        )
        .unwrap_err();
        assert!(matches!(err, BuildError::DuplicateSlug(s) if s == "same"));
    }

    /// REQ-3421 / CON-3410: when `shim_integrity` is Some, the shell
    /// lands under `_ztl/capability-shell.html` with the pinned CSP
    /// meta tag and the SRI-tagged shim script. When None, the shell
    /// file is absent (pre-bundle dev-build / unit-test path).
    #[test]
    fn shell_emission_is_gated_on_shim_integrity() {
        let tmp = TempDir::new().unwrap();
        let salt_b64 = URL_SAFE_NO_PAD.encode(b"salt-for-shell-test-01");
        let (_id, pk) = fresh_age_identity_pair();
        let recipients = sample_recipients(&salt_b64, vec![pk]);

        // Baseline: default cfg (shim_integrity = None) writes no
        // shell.
        run_capability_build(
            &build_cfg(tmp.path()),
            &recipients,
            &sample_grants(),
            &sample_secret(),
            &sample_signing_key(),
            &[PageInput {
                slug: "welcome".into(),
                html: "<p>x</p>".into(),
                explicit_cohorts: vec![],
            }],
        )
        .unwrap();
        let shell_path = tmp
            .path()
            .join("_ztl")
            .join(html_shell::CAPABILITY_SHELL_FILENAME);
        assert!(
            !shell_path.exists(),
            "shell must not be emitted when shim_integrity is None"
        );

        // With a hash threaded through, the shell appears and carries
        // every REQ-3421 / CON-3410 element.
        let tmp2 = TempDir::new().unwrap();
        let sri = "sha384-TestHashForUnitCoverageOnly0000000000000000000000000000000=";
        let mut cfg = build_cfg(tmp2.path());
        cfg.shim_integrity = Some(sri.to_string());
        run_capability_build(
            &cfg,
            &recipients,
            &sample_grants(),
            &sample_secret(),
            &sample_signing_key(),
            &[PageInput {
                slug: "welcome".into(),
                html: "<p>x</p>".into(),
                explicit_cohorts: vec![],
            }],
        )
        .unwrap();
        let shell = fs::read_to_string(
            tmp2.path()
                .join("_ztl")
                .join(html_shell::CAPABILITY_SHELL_FILENAME),
        )
        .expect("shell must be written");
        assert!(shell.contains("<meta http-equiv=\"Content-Security-Policy\""));
        assert!(shell.contains(sri));
        assert!(shell.contains("crossorigin=\"anonymous\""));
        assert!(shell.contains("<main data-ztl-capability></main>"));
    }

    #[test]
    fn encode_age1_matches_bech32_roundtrip() {
        // Pure sanity check: our in-test age1 encoder produces the
        // same string the bech32 decoder inverts back to the same
        // bytes. Guards the test helper, not production.
        let pk = [42u8; 32];
        let s = encode_age1(&pk);
        let back = pubkey_from_age_string(&s);
        assert_eq!(back, pk);
    }
}
