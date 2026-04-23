//! Public-repo safety (SPEC-034 REQ-3423 / ADR-3409 / BUG-013).
//!
//! The committed `grants.toml` carries per-reader recipient pubkeys
//! and metadata (names, expiry, cohort membership). On a public git
//! host, every historical commit of that file is world-readable —
//! leaking the reader list, the cohort roster, and the timing of each
//! enrolment. None of that disclosure is cryptographically fatal (the
//! pubkeys are not secret and the age ciphertexts stay safe), but it
//! reliably surprises operators who assumed "private wiki" meant "no
//! reader metadata on the open web".
//!
//! ADR-3409's resolution is a refuse-to-operate gate:
//!
//!   1. Detect whether the repository is published to a public host.
//!   2. If public, refuse to read a `.ztl/caps/grants.toml` that sits
//!      inside the repository tree.
//!   3. Require the operator to declare an out-of-tree path via
//!      `[access] grants_file_external = "..."` (a directory they
//!      control, e.g. `~/.config/ztl/<repo>/grants.toml`) and read
//!      grants from there instead.
//!   4. Emit a committed stub `grants-committed.toml` that the build
//!      driver writes back into the repo tree. The stub carries only
//!      the data the public build *must* have reproducible in git:
//!      cohort ids and their path-cap salts (so URLs stay stable
//!      across operator machines). It carries NO recipient pubkeys,
//!      NO grant names, NO expiry timestamps — nothing that
//!      identifies a reader.
//!
//! This module is the pure-core portion: visibility detection, policy
//! gating, and stub construction. Filesystem reads and writes live in
//! the effectful shell (`src/main.rs`).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::cap::recipients::parsing::{Cohort, RecipientsFile};

/// Known public-host domains. The match is exact-suffix on the host
/// component of a git origin URL (after stripping user/port), so
/// `git@github.com:org/repo` and `https://github.com/org/repo.git`
/// both match. Private self-hosted instances (`gitlab.acme.corp`) are
/// NOT matched and will fall through to `Visibility::Private` — the
/// heuristic is conservative-on-public, meaning it errs toward
/// flagging as public only when the host is on the hard-coded list.
///
/// Adding a host here is a security-relevant change: it flips the
/// gate from "allow in-repo grants" to "require external grants" for
/// every vault hosted there.
pub const KNOWN_PUBLIC_HOSTS: &[&str] = &[
    "github.com",
    "gitlab.com",
    "codeberg.org",
    "bitbucket.org",
    "sr.ht", // sourcehut — `git.sr.ht` canonical, `sr.ht` alias
    "git.sr.ht",
    "gitea.com",
    "notabug.org",
    "framagit.org",
];

/// Whether a vault's hosting makes its git history world-readable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    /// Config-declared or heuristic-detected public hosting. The
    /// in-repo `grants.toml` gate engages.
    Public,
    /// Config-declared private hosting, a private-org PAT covered by
    /// the heuristic opts, or an unknown/self-hosted origin. The gate
    /// does not engage.
    Private,
}

/// Caller-supplied knobs for the git-origin heuristic. Tests pass
/// deterministic values; the CLI harness wires these from environment
/// (e.g. `ztl_CAP_PUBLIC_HOST_TOKENS=github.com:org=ghp_xxx`).
#[derive(Debug, Default, Clone)]
pub struct GitHeuristicOpts {
    /// `(host, token_present)` overrides. Providing `(h, true)`
    /// means "the operator has proven to `ztl` that this host's
    /// access is token-gated to a private org" and flips a match
    /// from public→private. Matching is exact-host.
    pub private_tokens: Vec<(String, bool)>,
    /// Extra hosts to treat as public beyond `KNOWN_PUBLIC_HOSTS`.
    /// Useful for organisations with internal GitHub Enterprise
    /// mirrors that are, in practice, open to employees (which is
    /// still public-by-our-definition: the reader-list surface is
    /// exposed to everyone with mirror access).
    pub extra_public_hosts: Vec<String>,
}

/// Outcome of reconciling (a) vault config's `[vault] visibility`
/// declaration with (b) the git-origin heuristic. Config wins when
/// present; the heuristic only fires for unset config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VisibilitySource {
    /// `[vault] visibility = "public"|"private"` in
    /// `.ztl/config.toml`.
    ExplicitConfig,
    /// Matched an entry in `KNOWN_PUBLIC_HOSTS` (or
    /// `opts.extra_public_hosts`).
    HeuristicHost(String),
    /// No `[vault]` declaration AND no matching host — the gate does
    /// not engage. Callers should surface this clearly so operators
    /// know they are relying on the default.
    DefaultPrivate,
}

/// Resolved visibility with its source, for diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedVisibility {
    pub visibility: Visibility,
    pub source: VisibilitySource,
}

/// `.ztl/config.toml` lens for this module. We deserialise only the
/// `[vault]` and `[access]` sections; other sections (the `[parse]`
/// block handled by `crate::parsers`) round-trip through a flat
/// `toml::Value` elsewhere.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ztlConfigLens {
    #[serde(default)]
    pub vault: Option<VaultConfig>,
    #[serde(default)]
    pub access: Option<AccessConfig>,
}

/// `[vault]` block. Only the visibility declaration is consumed
/// today; additional fields may land as later SPEC-034 requirements
/// wire in (e.g. per-vault policy flags).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VaultConfig {
    #[serde(default)]
    pub visibility: Option<VisibilityDecl>,
}

/// Accepted values for `[vault] visibility`. Deserialises from
/// kebab-case so TOML reads `visibility = "public"`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum VisibilityDecl {
    Public,
    Private,
}

impl From<VisibilityDecl> for Visibility {
    fn from(v: VisibilityDecl) -> Self {
        match v {
            VisibilityDecl::Public => Visibility::Public,
            VisibilityDecl::Private => Visibility::Private,
        }
    }
}

/// `[access]` block.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AccessConfig {
    /// Filesystem path (absolute or `~`-prefixed) pointing at a
    /// grants.toml that lives OUTSIDE the repository tree. Required
    /// when the vault is public. The pure core accepts any string
    /// here; the shell is responsible for expanding `~` and resolving
    /// against the operator's home directory.
    #[serde(default)]
    pub grants_file_external: Option<String>,
    /// `[access.split_key]` — SPEC-034 REQ-3417 / REQ-3430. Controls
    /// whether `ztl cap invite --split-key` is permitted and how the
    /// second factor (`half2`) should be conveyed to the reader.
    /// Missing block deserialises as `None` (feature disabled).
    #[serde(default)]
    pub split_key: Option<SplitKeyConfig>,
}

/// `[access.split_key]` block. Opt-in per REQ-3430; operators set
/// `enabled = true` to allow `ztl cap invite --split-key`. The
/// `second_factor` knob picks which transport the shim prompts for on
/// first click (a human-typed phrase vs a camera-scanned QR code). The
/// pure core only parses + validates; the shell decides render format
/// (for CLI operator hints) and the shim reads it from runtime config.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SplitKeyConfig {
    /// Whether split-key mode is permitted at all. Default `false` so
    /// an operator who did not opt in cannot accidentally mint
    /// split-key URLs by passing `--split-key`.
    #[serde(default)]
    pub enabled: bool,
    /// How the reader receives `half2`. Required when `enabled=true`;
    /// when absent the CLI falls back to `SplitKeySecondFactor::default()`
    /// (spoken-phrase) and emits an operator hint on stdout.
    #[serde(default)]
    pub second_factor: Option<SplitKeySecondFactor>,
}

/// Acceptable values for `[access.split_key] second_factor`. Kebab-
/// case on the TOML side keeps the schema consistent with
/// `VisibilityDecl`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum SplitKeySecondFactor {
    /// Reader types `half2` into a text input. Operator convenience:
    /// short, dictatable, fits in a chat message or voice call.
    #[default]
    SpokenPhrase,
    /// Reader points a camera at a QR code the operator displayed or
    /// printed separately. Higher-friction onboarding but robust
    /// against typos.
    Qr,
}

impl SplitKeySecondFactor {
    /// Canonical wire token used by the CLI hint banner and the shim
    /// bundle. Matches the TOML form.
    pub fn as_wire_str(self) -> &'static str {
        match self {
            SplitKeySecondFactor::SpokenPhrase => "spoken-phrase",
            SplitKeySecondFactor::Qr => "qr",
        }
    }
}

impl SplitKeyConfig {
    /// Convenience accessor used by the invite driver when it has
    /// already decided to render the operator hint but wants to use
    /// the explicit config value when present. Returns the default
    /// when the operator opted in without picking a factor.
    pub fn effective_second_factor(&self) -> SplitKeySecondFactor {
        self.second_factor.unwrap_or_default()
    }
}

/// Parse `.ztl/config.toml` through the module's lens. Missing
/// sections deserialise as `None`; unknown top-level keys are allowed
/// (the wider ztl config has other consumers).
pub fn parse_config_lens(toml_body: &str) -> Result<ztlConfigLens, ConfigParseError> {
    toml::from_str::<ztlConfigLens>(toml_body).map_err(|e| ConfigParseError(e.to_string()))
}

/// TOML parse failure specific to this module. A thin wrapper around
/// the underlying error so callers don't have to depend on the
/// `toml` crate's error types.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid .ztl/config.toml: {0}")]
pub struct ConfigParseError(pub String);

/// Normalise a git origin URL to the bare host component. Handles
/// the three forms `git` emits for `remote.origin.url`:
///
///   * `git@github.com:org/repo.git`            (scp-style SSH)
///   * `ssh://git@github.com:22/org/repo.git`   (explicit SSH scheme)
///   * `https://user:pass@github.com/org/repo`  (HTTPS)
///
/// Returns `None` if the input is unparseable or does not carry a
/// host (which can happen for filesystem remotes like
/// `/srv/git/foo`).
pub fn origin_host(url: &str) -> Option<&str> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }

    // scp-style: `user@host:path`. Detect by the absence of `://`
    // and the presence of a `:` that is NOT inside a scheme.
    if !url.contains("://") {
        if let Some(colon) = url.find(':') {
            // Must have something before the colon and the colon
            // must not be the first char (filesystem path).
            let head = &url[..colon];
            if !head.is_empty() && !head.starts_with('/') {
                // Strip optional `user@` prefix.
                let host = head.rsplit_once('@').map(|(_u, h)| h).unwrap_or(head);
                if host.is_empty() {
                    return None;
                }
                return Some(strip_port(host));
            }
        }
        return None;
    }

    // URL-style: scheme://[user[:pass]@]host[:port]/...
    let after_scheme = url.split_once("://").map(|(_s, r)| r)?;
    let authority = after_scheme.split('/').next()?;
    let host_plus_port = authority
        .rsplit_once('@')
        .map(|(_c, h)| h)
        .unwrap_or(authority);
    if host_plus_port.is_empty() {
        return None;
    }
    Some(strip_port(host_plus_port))
}

fn strip_port(host: &str) -> &str {
    host.rsplit_once(':').map(|(h, _p)| h).unwrap_or(host)
}

/// Heuristic visibility from a git origin URL. Public only when the
/// host is on the known list (or `opts.extra_public_hosts`) AND not
/// flagged by `opts.private_tokens` as token-gated to a private org.
pub fn visibility_from_origin(url: &str, opts: &GitHeuristicOpts) -> ResolvedVisibility {
    let Some(host) = origin_host(url) else {
        return ResolvedVisibility {
            visibility: Visibility::Private,
            source: VisibilitySource::DefaultPrivate,
        };
    };

    let is_known_public = KNOWN_PUBLIC_HOSTS
        .iter()
        .any(|h| host.eq_ignore_ascii_case(h))
        || opts
            .extra_public_hosts
            .iter()
            .any(|h| host.eq_ignore_ascii_case(h));

    if !is_known_public {
        return ResolvedVisibility {
            visibility: Visibility::Private,
            source: VisibilitySource::DefaultPrivate,
        };
    }

    let is_token_private = opts
        .private_tokens
        .iter()
        .any(|(h, v)| *v && host.eq_ignore_ascii_case(h));
    if is_token_private {
        return ResolvedVisibility {
            visibility: Visibility::Private,
            source: VisibilitySource::DefaultPrivate,
        };
    }

    ResolvedVisibility {
        visibility: Visibility::Public,
        source: VisibilitySource::HeuristicHost(host.to_string()),
    }
}

/// Full reconciliation: config wins when it declares `visibility`
/// explicitly; otherwise fall back to the heuristic when an origin
/// URL is supplied; otherwise default private.
pub fn resolve_visibility(
    config: &ztlConfigLens,
    origin_url: Option<&str>,
    opts: &GitHeuristicOpts,
) -> ResolvedVisibility {
    if let Some(v) = config.vault.as_ref().and_then(|v| v.visibility) {
        return ResolvedVisibility {
            visibility: v.into(),
            source: VisibilitySource::ExplicitConfig,
        };
    }
    match origin_url {
        Some(url) => visibility_from_origin(url, opts),
        None => ResolvedVisibility {
            visibility: Visibility::Private,
            source: VisibilitySource::DefaultPrivate,
        },
    }
}

/// The decision the build driver acts on after reconciling
/// visibility with the `[access]` config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrantsSource {
    /// Private vault: read `grants.toml` from inside the repo tree
    /// at the supplied path.
    InRepo { path: PathBuf },
    /// Public vault with a configured external file: read grants
    /// from this path and treat the in-repo file, if present, as the
    /// committed stub (never as a source of recipient metadata).
    External { path: PathBuf },
}

/// Refusal surface. Every variant is an exit-1 condition for
/// `ztl build` / `ztl cap invite`; the CLI renders each with a
/// remediation hint.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PolicyError {
    /// Vault is public but `[access] grants_file_external` is unset.
    /// Operator needs to add the declaration and point it at a path
    /// outside the repo.
    #[error(
        "public-vault safety (REQ-3423): `[access] grants_file_external` is required but not set; \
         add it to .ztl/config.toml and point it at a path outside the repository"
    )]
    MissingExternalPath,
    /// `[access] grants_file_external` is set but the path resolves
    /// inside `repo_root`. The whole point of the external file is
    /// that it is NOT tracked by git; a path inside the repo defeats
    /// the safety and is refused.
    #[error(
        "public-vault safety (REQ-3423): `[access] grants_file_external` path {external} lies \
         inside the repository root {repo_root}; move it to a directory outside the repo"
    )]
    ExternalPathInsideRepo {
        external: PathBuf,
        repo_root: PathBuf,
    },
    /// `[access] grants_file_external` is set but the string did not
    /// parse as a usable path (empty after trimming). Pure core
    /// performs only the empty-string check; `~` expansion and
    /// canonicalisation live in the shell and may surface other
    /// errors.
    #[error("public-vault safety (REQ-3423): `[access] grants_file_external` value is empty")]
    EmptyExternalPath,
}

/// Pure-core policy decision. The caller supplies (a) the vault's
/// resolved visibility, (b) the parsed `[access]` config, (c) the
/// repository root, and (d) the in-repo grants path. The function
/// returns either a [`GrantsSource`] or a [`PolicyError`] that
/// explains how to fix the config.
///
/// This is deliberately agnostic to `~` expansion and symlink
/// resolution — both depend on the operator's environment and
/// belong in the shell layer. The pure-core check is the pair of
/// invariants that hold regardless of filesystem state:
///
///   * public + external-empty → refuse
///   * public + external-inside-repo → refuse
///
/// The shell layer canonicalises both paths before calling this
/// function, so the "inside the repo" check reduces to a path
/// prefix comparison.
pub fn decide_grants_source(
    resolved: &ResolvedVisibility,
    access: &AccessConfig,
    repo_root: &Path,
    in_repo_grants_path: &Path,
) -> Result<GrantsSource, PolicyError> {
    if resolved.visibility == Visibility::Private {
        return Ok(GrantsSource::InRepo {
            path: in_repo_grants_path.to_path_buf(),
        });
    }

    let raw = access
        .grants_file_external
        .as_deref()
        .map(str::trim)
        .unwrap_or("");
    if raw.is_empty() {
        return Err(PolicyError::MissingExternalPath);
    }

    let external = PathBuf::from(raw);
    if path_is_inside(&external, repo_root) {
        return Err(PolicyError::ExternalPathInsideRepo {
            external,
            repo_root: repo_root.to_path_buf(),
        });
    }
    Ok(GrantsSource::External { path: external })
}

/// Conservative prefix check: both paths must be absolute for the
/// comparison to engage. A relative external path (which the shell
/// should have resolved before calling us) is treated as "not inside"
/// so the caller gets the filesystem-level error from
/// `std::fs::canonicalize` rather than a misleading safety error.
fn path_is_inside(candidate: &Path, root: &Path) -> bool {
    if !candidate.is_absolute() || !root.is_absolute() {
        return false;
    }
    candidate.starts_with(root)
}

/// Stub form of the committed grants file. Carries cohort ids and
/// their stable path-cap salts so URLs stay reproducible across
/// operator machines without leaking any reader-identifying data.
///
/// The stub is serialised at repo-root as `grants-committed.toml`
/// (distinct from `grants.toml` so the two paths are never confused).
/// It is the ONLY grants artefact a public repo should carry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GrantsCommittedStub {
    /// Schema version; bumped when the stub's shape changes.
    pub version: u32,
    /// Commentary that survives the round trip. Helps readers of the
    /// committed file understand why the interesting data is missing.
    #[serde(default)]
    pub notice: Option<String>,
    /// Cohort entries. Ordered for byte-stable output; the shell
    /// layer is expected to feed cohorts in `recipients.toml` order.
    #[serde(default, rename = "cohort")]
    pub cohorts: Vec<CommittedCohortStub>,
}

/// One cohort's public-safe stub entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CommittedCohortStub {
    pub id: String,
    /// Stable path-cap salt (base64url). Present in the committed
    /// stub so every operator's build emits identical URLs. If a
    /// cohort has no stable salt yet (fresh vault, no invites), the
    /// field is omitted rather than sent as an empty string.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub salt_stable: Option<String>,
}

/// Current `grants-committed.toml` schema version.
pub const COMMITTED_STUB_VERSION: u32 = 1;

/// Default header committed into the stub so a human opening the
/// file understands why it exists.
pub const COMMITTED_STUB_NOTICE: &str =
    "Public-repo safety stub (SPEC-034 REQ-3423). Contains cohort ids and path-cap salts only. \
     Recipient pubkeys, grant metadata, and reader names live in the out-of-tree file referenced \
     by [access] grants_file_external.";

/// Build the stub from a parsed `recipients.toml`. Pure function:
/// depends only on the cohort list. Returned `cohorts` preserves the
/// input order so the TOML output is byte-stable given stable input.
pub fn build_committed_stub(recipients: &RecipientsFile) -> GrantsCommittedStub {
    let cohorts = recipients
        .cohorts
        .iter()
        .map(committed_stub_for_cohort)
        .collect();
    GrantsCommittedStub {
        version: COMMITTED_STUB_VERSION,
        notice: Some(COMMITTED_STUB_NOTICE.to_string()),
        cohorts,
    }
}

fn committed_stub_for_cohort(cohort: &Cohort) -> CommittedCohortStub {
    CommittedCohortStub {
        id: cohort.id.clone(),
        salt_stable: cohort.salt_stable.clone(),
    }
}

/// Serialise the stub to TOML. Uses `toml::to_string_pretty` so the
/// committed artefact is diff-friendly.
pub fn serialise_committed_stub(stub: &GrantsCommittedStub) -> Result<String, toml::ser::Error> {
    toml::to_string_pretty(stub)
}

/// Parse a stub back from TOML. Exposed so tests and the build
/// driver can round-trip the artefact during verification.
pub fn parse_committed_stub(body: &str) -> Result<GrantsCommittedStub, toml::de::Error> {
    toml::from_str(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cap::recipients::parsing::{Cohort, CohortMode, VaultSection};

    fn sample_recipients() -> RecipientsFile {
        RecipientsFile {
            version: 1,
            vault: VaultSection {
                signing_pubkey: "ed25519:Aabc_-".to_string(),
            },
            cohorts: vec![
                Cohort {
                    id: "engineering".to_string(),
                    name: Some("Engineering".to_string()),
                    mode: CohortMode::DelegatedUrl,
                    pubkeys: vec!["age-recipient-v1:xyz".to_string()],
                    pages: None,
                    salt_stable: Some("stable-eng-salt".to_string()),
                    salt_rotated: Some("rot-1".to_string()),
                    last_rotated: None,
                },
                Cohort {
                    id: "ops".to_string(),
                    name: None,
                    mode: CohortMode::DelegatedUrl,
                    pubkeys: vec![],
                    pages: None,
                    salt_stable: None,
                    salt_rotated: None,
                    last_rotated: None,
                },
            ],
        }
    }

    #[test]
    fn origin_host_parses_all_three_url_forms() {
        assert_eq!(
            origin_host("git@github.com:org/repo.git"),
            Some("github.com")
        );
        assert_eq!(
            origin_host("ssh://git@github.com:22/org/repo.git"),
            Some("github.com")
        );
        assert_eq!(
            origin_host("https://user:pass@github.com/org/repo"),
            Some("github.com")
        );
        assert_eq!(
            origin_host("https://github.com/org/repo"),
            Some("github.com")
        );
    }

    #[test]
    fn origin_host_rejects_filesystem_remotes() {
        assert_eq!(origin_host("/srv/git/foo.git"), None);
        assert_eq!(origin_host(""), None);
        assert_eq!(origin_host("   "), None);
    }

    #[test]
    fn heuristic_flags_known_public_hosts() {
        let opts = GitHeuristicOpts::default();
        for url in [
            "git@github.com:org/repo.git",
            "https://gitlab.com/org/repo",
            "ssh://git@codeberg.org/org/repo.git",
        ] {
            let r = visibility_from_origin(url, &opts);
            assert_eq!(
                r.visibility,
                Visibility::Public,
                "expected {url} to resolve as public"
            );
            assert!(matches!(r.source, VisibilitySource::HeuristicHost(_)));
        }
    }

    #[test]
    fn heuristic_leaves_self_hosted_as_private() {
        let opts = GitHeuristicOpts::default();
        for url in [
            "git@gitlab.acme.corp:team/repo.git",
            "https://git.internal.example/team/repo",
        ] {
            let r = visibility_from_origin(url, &opts);
            assert_eq!(r.visibility, Visibility::Private);
            assert_eq!(r.source, VisibilitySource::DefaultPrivate);
        }
    }

    #[test]
    fn extra_public_hosts_flip_self_hosted_to_public() {
        let opts = GitHeuristicOpts {
            extra_public_hosts: vec!["gitlab.acme.corp".to_string()],
            ..Default::default()
        };
        let r = visibility_from_origin("git@gitlab.acme.corp:team/repo.git", &opts);
        assert_eq!(r.visibility, Visibility::Public);
    }

    #[test]
    fn private_token_flag_overrides_known_public_host() {
        let opts = GitHeuristicOpts {
            private_tokens: vec![("github.com".to_string(), true)],
            ..Default::default()
        };
        let r = visibility_from_origin("git@github.com:org/private-repo.git", &opts);
        assert_eq!(r.visibility, Visibility::Private);
    }

    #[test]
    fn private_token_flag_is_ignored_when_false() {
        let opts = GitHeuristicOpts {
            private_tokens: vec![("github.com".to_string(), false)],
            ..Default::default()
        };
        let r = visibility_from_origin("git@github.com:org/repo.git", &opts);
        assert_eq!(r.visibility, Visibility::Public);
    }

    #[test]
    fn explicit_config_beats_heuristic() {
        let cfg = ztlConfigLens {
            vault: Some(VaultConfig {
                visibility: Some(VisibilityDecl::Private),
            }),
            access: None,
        };
        let opts = GitHeuristicOpts::default();
        let r = resolve_visibility(&cfg, Some("git@github.com:org/repo.git"), &opts);
        assert_eq!(r.visibility, Visibility::Private);
        assert_eq!(r.source, VisibilitySource::ExplicitConfig);
    }

    #[test]
    fn explicit_public_config_overrides_self_hosted_origin() {
        let cfg = ztlConfigLens {
            vault: Some(VaultConfig {
                visibility: Some(VisibilityDecl::Public),
            }),
            access: None,
        };
        let opts = GitHeuristicOpts::default();
        let r = resolve_visibility(&cfg, Some("git@git.internal.example:team/repo.git"), &opts);
        assert_eq!(r.visibility, Visibility::Public);
        assert_eq!(r.source, VisibilitySource::ExplicitConfig);
    }

    #[test]
    fn no_origin_no_config_defaults_private() {
        let r = resolve_visibility(
            &ztlConfigLens::default(),
            None,
            &GitHeuristicOpts::default(),
        );
        assert_eq!(r.visibility, Visibility::Private);
        assert_eq!(r.source, VisibilitySource::DefaultPrivate);
    }

    #[test]
    fn parse_config_lens_accepts_full_example() {
        let toml_body = r#"
            [vault]
            visibility = "public"

            [access]
            grants_file_external = "/home/op/.config/ztl/my-wiki/grants.toml"

            [parse]
            default = "markdown"
        "#;
        let lens = parse_config_lens(toml_body).unwrap();
        assert_eq!(
            lens.vault.and_then(|v| v.visibility),
            Some(VisibilityDecl::Public)
        );
        assert_eq!(
            lens.access.and_then(|a| a.grants_file_external),
            Some("/home/op/.config/ztl/my-wiki/grants.toml".to_string())
        );
    }

    #[test]
    fn parse_config_lens_tolerates_missing_sections() {
        let lens = parse_config_lens("").unwrap();
        assert!(lens.vault.is_none());
        assert!(lens.access.is_none());
    }

    #[test]
    fn parse_config_lens_accepts_split_key_block() {
        let toml_body = r#"
            [access.split_key]
            enabled = true
            second_factor = "qr"
        "#;
        let lens = parse_config_lens(toml_body).unwrap();
        let sk = lens
            .access
            .expect("access block")
            .split_key
            .expect("split_key block");
        assert!(sk.enabled);
        assert_eq!(sk.second_factor, Some(SplitKeySecondFactor::Qr));
        assert_eq!(sk.effective_second_factor().as_wire_str(), "qr");
    }

    #[test]
    fn parse_config_lens_split_key_defaults_to_spoken_phrase() {
        let toml_body = r#"
            [access.split_key]
            enabled = true
        "#;
        let lens = parse_config_lens(toml_body).unwrap();
        let sk = lens.access.unwrap().split_key.unwrap();
        assert!(sk.enabled);
        assert_eq!(sk.second_factor, None);
        assert_eq!(
            sk.effective_second_factor(),
            SplitKeySecondFactor::SpokenPhrase
        );
    }

    #[test]
    fn parse_config_lens_split_key_rejects_unknown_factor() {
        let toml_body = r#"
            [access.split_key]
            enabled = true
            second_factor = "morse-code"
        "#;
        parse_config_lens(toml_body).unwrap_err();
    }

    #[test]
    fn parse_config_lens_split_key_default_is_disabled() {
        // REQ-3430 acceptance: default must be `enabled = false`.
        let toml_body = r#"
            [access.split_key]
        "#;
        let lens = parse_config_lens(toml_body).unwrap();
        let sk = lens.access.unwrap().split_key.unwrap();
        assert_eq!(sk.enabled, false);
    }

    #[test]
    fn parse_config_lens_rejects_unknown_visibility_value() {
        let toml_body = r#"
            [vault]
            visibility = "world-writable"
        "#;
        parse_config_lens(toml_body).unwrap_err();
    }

    #[test]
    fn decide_grants_source_private_returns_in_repo() {
        let resolved = ResolvedVisibility {
            visibility: Visibility::Private,
            source: VisibilitySource::DefaultPrivate,
        };
        let access = AccessConfig::default();
        let decision = decide_grants_source(
            &resolved,
            &access,
            Path::new("/tmp/repo"),
            Path::new("/tmp/repo/.ztl/caps/grants.toml"),
        )
        .unwrap();
        assert!(matches!(decision, GrantsSource::InRepo { .. }));
    }

    #[test]
    fn decide_grants_source_public_missing_external_errors() {
        let resolved = ResolvedVisibility {
            visibility: Visibility::Public,
            source: VisibilitySource::ExplicitConfig,
        };
        let access = AccessConfig::default();
        let err = decide_grants_source(
            &resolved,
            &access,
            Path::new("/tmp/repo"),
            Path::new("/tmp/repo/.ztl/caps/grants.toml"),
        )
        .unwrap_err();
        assert_eq!(err, PolicyError::MissingExternalPath);
    }

    #[test]
    fn decide_grants_source_public_empty_external_errors() {
        let resolved = ResolvedVisibility {
            visibility: Visibility::Public,
            source: VisibilitySource::ExplicitConfig,
        };
        let access = AccessConfig {
            grants_file_external: Some("   ".to_string()),
            split_key: None,
        };
        let err = decide_grants_source(
            &resolved,
            &access,
            Path::new("/tmp/repo"),
            Path::new("/tmp/repo/.ztl/caps/grants.toml"),
        )
        .unwrap_err();
        assert_eq!(err, PolicyError::MissingExternalPath);
    }

    #[test]
    fn decide_grants_source_public_external_inside_repo_errors() {
        let resolved = ResolvedVisibility {
            visibility: Visibility::Public,
            source: VisibilitySource::ExplicitConfig,
        };
        let access = AccessConfig {
            grants_file_external: Some("/tmp/repo/hidden/grants.toml".to_string()),
            split_key: None,
        };
        let err = decide_grants_source(
            &resolved,
            &access,
            Path::new("/tmp/repo"),
            Path::new("/tmp/repo/.ztl/caps/grants.toml"),
        )
        .unwrap_err();
        assert!(matches!(err, PolicyError::ExternalPathInsideRepo { .. }));
    }

    #[test]
    fn decide_grants_source_public_external_outside_repo_ok() {
        let resolved = ResolvedVisibility {
            visibility: Visibility::Public,
            source: VisibilitySource::ExplicitConfig,
        };
        let access = AccessConfig {
            grants_file_external: Some("/home/op/.config/ztl/site/grants.toml".to_string()),
            split_key: None,
        };
        let decision = decide_grants_source(
            &resolved,
            &access,
            Path::new("/tmp/repo"),
            Path::new("/tmp/repo/.ztl/caps/grants.toml"),
        )
        .unwrap();
        match decision {
            GrantsSource::External { path } => {
                assert_eq!(path, Path::new("/home/op/.config/ztl/site/grants.toml"));
            }
            other => panic!("expected External, got {other:?}"),
        }
    }

    #[test]
    fn build_committed_stub_drops_recipients_and_names() {
        let r = sample_recipients();
        let stub = build_committed_stub(&r);
        assert_eq!(stub.version, COMMITTED_STUB_VERSION);
        assert_eq!(stub.cohorts.len(), 2);
        assert_eq!(stub.cohorts[0].id, "engineering");
        assert_eq!(
            stub.cohorts[0].salt_stable.as_deref(),
            Some("stable-eng-salt")
        );
        assert_eq!(stub.cohorts[1].id, "ops");
        assert!(stub.cohorts[1].salt_stable.is_none());

        let body = serialise_committed_stub(&stub).unwrap();
        // Hard negative assertions: pubkeys, names, grant metadata all absent.
        assert!(
            !body.contains("age-recipient-v1"),
            "stub leaked a recipient: {body}"
        );
        assert!(
            !body.contains("Engineering"),
            "stub leaked cohort display name: {body}"
        );
        assert!(!body.contains("rot-1"), "stub leaked rotated salt: {body}");
        assert!(
            !body.contains("last_rotated"),
            "stub leaked audit timestamp: {body}"
        );
        // Positive: salts + ids are present.
        assert!(body.contains("engineering"));
        assert!(body.contains("stable-eng-salt"));
    }

    #[test]
    fn committed_stub_round_trips() {
        let stub = build_committed_stub(&sample_recipients());
        let body = serialise_committed_stub(&stub).unwrap();
        let back = parse_committed_stub(&body).unwrap();
        assert_eq!(back, stub);
    }

    #[test]
    fn committed_stub_rejects_unknown_cohort_fields() {
        let bad = r#"
            version = 1

            [[cohort]]
            id = "eng"
            salt_stable = "abc"
            recipients = ["leaked"]
        "#;
        parse_committed_stub(bad).unwrap_err();
    }
}
