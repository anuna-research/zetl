//! Operator checklist for taking a capability-mode deployment offline
//! at the host level (SPEC-034 REQ-3431, §11.3; BUG-024 resolution).
//!
//! `ztl cap emergency-shutdown` is a **documentation-generation**
//! command — the spec deliberately has no cryptographic kill-switch, so
//! this module merely assembles a printable operator runbook from
//! whatever vault-level context is discoverable on disk. The effectful
//! shell (`src/main.rs::cmd_cap_emergency_shutdown`) reads
//! `recipients.toml` (if present) and hands a `ShutdownContext` to
//! `render_checklist` / `checklist_json` here; everything in this file
//! is pure.
//!
//! The four canonical steps mirror REQ-3431:
//!   1. Remove or point DNS away from the deployment.
//!   2. Instruct the CDN to purge/delete all `/c/*` objects.
//!   3. Rotate `ztl_CAP_SECRET` + `ztl_CAP_SIGNING_KEY`
//!      (fresh `ztl cap genkey`).
//!   4. Announce the incident to readers; expect re-enrolment when
//!      service is restored.

use serde::Serialize;

/// Context collected by the effectful shell and rendered into the
/// operator checklist. All fields are optional so the command degrades
/// gracefully when run in a bare directory: the printed text still
/// covers every REQ-3431 step, it just cannot name vault-specific
/// cohorts or the signing key to rotate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ShutdownContext {
    /// Human-readable vault label (typically the basename of the vault
    /// root). Printed in the header so an operator running this across
    /// multiple vaults knows which one they are shutting down.
    pub vault_name: String,
    /// Cohort ids discovered in `recipients.toml`, rendered verbatim so
    /// the operator can cross-reference them in the announcement step.
    /// Empty when `recipients.toml` is missing or unreadable.
    pub cohorts: Vec<CohortRef>,
    /// Deploy target hostname if the operator has configured one
    /// (`--site-url` on `ztl build --capability`, or equivalent).
    /// Printed as a breadcrumb in step 1; absence is non-fatal.
    pub deploy_target: Option<String>,
    /// Ed25519 vault-signing pubkey from `recipients.toml` `[vault]`
    /// section. Printed alongside step 3 so the operator can match the
    /// on-disk key against the one embedded in the deployed shim.
    pub signing_pubkey: Option<String>,
}

/// One cohort row in the checklist. `name` mirrors
/// `recipients.toml::[[cohort]].name` when set, falling back to `id`
/// for the public label.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CohortRef {
    pub id: String,
    pub name: Option<String>,
}

/// Render the printable operator checklist. Output is deterministic
/// plain text with no ANSI escapes, terminates with a single trailing
/// newline, and is safe to pipe into `less`, a mail body, or a runbook
/// document.
pub fn render_checklist(ctx: &ShutdownContext) -> String {
    let mut out = String::new();

    out.push_str("ztl cap emergency-shutdown\n");
    out.push_str("===========================\n");
    out.push_str(&format!("vault:  {}\n", ctx.vault_name));
    if let Some(target) = ctx.deploy_target.as_deref() {
        out.push_str(&format!("deploy: {target}\n"));
    } else {
        out.push_str("deploy: <not configured — substitute your hostname>\n");
    }
    out.push('\n');

    out.push_str(
        "This command does NOT modify any files, purge any CDN, or rotate any\n\
         key material. It prints the operator actions required to take the\n\
         wiki offline at the host level (SPEC-034 REQ-3431, §11.3). Work\n\
         through the steps in order; none of them are reversible.\n",
    );
    out.push('\n');

    out.push_str("Step 1 — Remove or redirect DNS\n");
    out.push_str("-------------------------------\n");
    if let Some(target) = ctx.deploy_target.as_deref() {
        out.push_str(&format!(
            "  * Point the DNS record for {target} at a holding page, or delete\n\
             \x20   the record entirely. Wait for TTL propagation before declaring\n\
             \x20   the site offline.\n",
        ));
    } else {
        out.push_str(
            "  * Point the deployment's DNS record at a holding page, or delete\n\
             \x20   it entirely. Wait for TTL propagation before declaring the\n\
             \x20   site offline.\n",
        );
    }
    out.push('\n');

    out.push_str("Step 2 — CDN: purge /c/* objects\n");
    out.push_str("--------------------------------\n");
    out.push_str(
        "  * Instruct the CDN to purge or delete every object under `/c/*` so\n\
         \x20   cached encrypted pages can no longer be served. This also\n\
         \x20   invalidates any `/assets/shim.js` cache entries.\n\
         \x20 * Keep purge receipts; readers surfacing cached pages from a\n\
         \x20   mirror will need evidence that the origin was flushed.\n",
    );
    out.push('\n');

    out.push_str("Step 3 — Rotate ztl_CAP_SECRET + ztl_CAP_SIGNING_KEY\n");
    out.push_str("------------------------------------------------------\n");
    out.push_str(
        "  * Run `ztl cap genkey` to produce fresh encryption and signing\n\
         \x20   secrets. Store them in the password manager; the old values are\n\
         \x20   now considered compromised.\n\
         \x20 * Any subsequent rebuild SHALL use the new secrets. Readers will\n\
         \x20   need to re-enrol per device (TOFU) once service resumes.\n",
    );
    if let Some(pk) = ctx.signing_pubkey.as_deref() {
        out.push_str(&format!(
            "  * Current `[vault].signing_pubkey` on disk: {pk}\n\
             \x20   (discard after rotation.)\n",
        ));
    }
    out.push('\n');

    out.push_str("Step 4 — Announce to readers\n");
    out.push_str("----------------------------\n");
    if ctx.cohorts.is_empty() {
        out.push_str(
            "  * `recipients.toml` was not found or parseable — announce\n\
             \x20   through every reader channel you have on record.\n",
        );
    } else {
        out.push_str(
            "  * Announce the shutdown to every affected cohort. Readers\n\
             \x20   whose devices retain decrypted content should be told the\n\
             \x20   vault-signing key has been rotated; they will need to\n\
             \x20   re-enrol per device when (and if) service is restored.\n\
             \x20 * Cohorts on record:\n",
        );
        for c in &ctx.cohorts {
            match c.name.as_deref() {
                Some(name) if !name.is_empty() => {
                    out.push_str(&format!("      - {} ({})\n", c.id, name));
                }
                _ => {
                    out.push_str(&format!("      - {}\n", c.id));
                }
            }
        }
    }
    out.push('\n');

    out.push_str("Step 5 — Re-enrolment (when service resumes)\n");
    out.push_str("--------------------------------------------\n");
    out.push_str(
        "  * Issue fresh `ztl cap invite` grants for returning readers.\n\
         \x20 * Do NOT reuse any old invite URL, grant id, or cohort salt.\n\
         \x20 * The new deployment starts from a clean TOFU state.\n",
    );

    out
}

/// JSON-shaped mirror of the checklist for `--json` / `-f json`
/// callers. Fields are stable across runs so scripts and agents can
/// parse the structured form without scraping the text layout.
pub fn checklist_json(ctx: &ShutdownContext) -> serde_json::Value {
    serde_json::json!({
        "command": "ztl cap emergency-shutdown",
        "spec": "SPEC-034 REQ-3431",
        "automated": false,
        "vault_name": ctx.vault_name,
        "deploy_target": ctx.deploy_target,
        "signing_pubkey": ctx.signing_pubkey,
        "cohorts": ctx.cohorts,
        "steps": [
            {
                "n": 1,
                "title": "Remove or redirect DNS",
                "summary": "Point the deployment's DNS record at a holding page or delete it; wait for TTL propagation."
            },
            {
                "n": 2,
                "title": "CDN: purge /c/* objects",
                "summary": "Purge or delete every object under /c/* and invalidate /assets/shim.js."
            },
            {
                "n": 3,
                "title": "Rotate ztl_CAP_SECRET + ztl_CAP_SIGNING_KEY",
                "summary": "Run `ztl cap genkey` for fresh secrets; discard the compromised pair."
            },
            {
                "n": 4,
                "title": "Announce to readers",
                "summary": "Notify every affected cohort; expect re-enrolment per device on resumption."
            },
            {
                "n": 5,
                "title": "Re-enrolment",
                "summary": "Issue fresh grants with `ztl cap invite`; do not reuse old URLs or salts."
            }
        ]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_ctx() -> ShutdownContext {
        ShutdownContext {
            vault_name: "my-vault".to_string(),
            cohorts: vec![
                CohortRef {
                    id: "engineering".to_string(),
                    name: Some("Engineering Team".to_string()),
                },
                CohortRef {
                    id: "ops".to_string(),
                    name: None,
                },
            ],
            deploy_target: Some("vault.example".to_string()),
            signing_pubkey: Some("ed25519:abc123".to_string()),
        }
    }

    #[test]
    fn checklist_contains_every_req_3431_step() {
        let out = render_checklist(&base_ctx());
        assert!(out.contains("Step 1 — Remove or redirect DNS"));
        assert!(out.contains("Step 2 — CDN: purge /c/* objects"));
        assert!(out.contains("Step 3 — Rotate ztl_CAP_SECRET + ztl_CAP_SIGNING_KEY"));
        assert!(out.contains("Step 4 — Announce to readers"));
        assert!(out.contains("Step 5 — Re-enrolment"));
    }

    #[test]
    fn checklist_surfaces_vault_identifiers() {
        let out = render_checklist(&base_ctx());
        assert!(out.contains("my-vault"), "vault name missing:\n{out}");
        assert!(
            out.contains("vault.example"),
            "deploy target missing:\n{out}"
        );
        assert!(out.contains("engineering"), "cohort id missing:\n{out}");
        assert!(
            out.contains("Engineering Team"),
            "cohort label missing:\n{out}"
        );
        assert!(out.contains("ops"), "unlabelled cohort missing:\n{out}");
        assert!(
            out.contains("ed25519:abc123"),
            "signing pubkey missing:\n{out}"
        );
    }

    #[test]
    fn checklist_is_not_automated() {
        let out = render_checklist(&base_ctx());
        assert!(out.contains("does NOT modify"));
        let json = checklist_json(&base_ctx());
        assert_eq!(json["automated"], false);
    }

    #[test]
    fn checklist_renders_without_optional_context() {
        let ctx = ShutdownContext {
            vault_name: "bare".to_string(),
            cohorts: Vec::new(),
            deploy_target: None,
            signing_pubkey: None,
        };
        let out = render_checklist(&ctx);
        // Every REQ-3431 step still prints even with no discoverable context.
        assert!(out.contains("Step 1"));
        assert!(out.contains("Step 2"));
        assert!(out.contains("Step 3"));
        assert!(out.contains("Step 4"));
        assert!(out.contains("Step 5"));
        assert!(out.contains("bare"));
        assert!(out.contains("not configured"));
        assert!(out.contains("recipients.toml` was not found"));
    }

    #[test]
    fn render_is_deterministic() {
        let a = render_checklist(&base_ctx());
        let b = render_checklist(&base_ctx());
        assert_eq!(a, b);
    }

    #[test]
    fn json_mirrors_text_fields() {
        let ctx = base_ctx();
        let j = checklist_json(&ctx);
        assert_eq!(j["vault_name"], "my-vault");
        assert_eq!(j["deploy_target"], "vault.example");
        assert_eq!(j["signing_pubkey"], "ed25519:abc123");
        assert_eq!(j["cohorts"].as_array().unwrap().len(), 2);
        assert_eq!(j["steps"].as_array().unwrap().len(), 5);
    }
}
