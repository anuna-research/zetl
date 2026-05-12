---
id: BUG-002
title: `zetl build` does not produce capability-URL encrypted output (SPEC-034 partial implementation)
status: open
severity: S2
priority: P1
detection-method: end-to-end deploy attempt — followed `zetl cap` workflow, inspected `dist/` for capability-namespaced output
date: 2026-05-10
binary: zetl 0.7.0 (macOS, aarch64-apple-darwin)
vault: 22-page travel itinerary with sensitive content (insurance policy numbers, addresses, contacts)
affects:
  - `[[zetl build]]` capability-URL static-site mode
  - any operator following the documented `zetl cap genkey → cap invite → zetl build` flow expecting encrypted output
  - public-safety guarantee implied by `zetl cap sweep`'s message "run `zetl build` to rebuild without the swept recipients, then invalidate the `/c/*` cache on your CDN"
not-affected:
  - `[[zetl cap genkey]]` — emits secret + signing keys correctly
  - `[[zetl cap invite]]` — issues delegated-URL grants and writes `grants.toml` correctly
  - normal (non-cap) `[[zetl build]]` — produces the expected plaintext static site
---

## Summary

After completing the documented capability-URL setup (`zetl cap genkey` → populate `recipients.toml` → `zetl cap invite`), `zetl build` produces a normal plaintext static site at `dist/`. The capability-namespaced output paths the workflow assumes (`dist/c/<cohort>/<slug>.html`) are never created, and sensitive page content is visible in plaintext under `dist/<slug>/index.html` and in `dist/search-index.json`.

The `zetl cap` CLI surfaces (`genkey`, `invite`, `rotate-grant`) work as documented; the build pipeline does not appear to consume the capability state. There is no documented activation switch — `--features cap`, `--mode cap`, `[access]` config flags, and `[vault] visibility = "capability"` were all tried (see *Things tried* below). Combined with `zetl cap list` returning *"not-yet-implemented (SPEC-034 REQ-3416 CLI surface stub)"*, this looks like the build-time half of SPEC-034 is partially landed.

This is a confidentiality regression for any user who follows the man page believing the output is encrypted.

## Specification Reference

- **Violates:** [[SPEC-034]] capability-URL static-site mode — specifically the implicit promise of `zetl cap sweep`'s help text:
  > *"Next: run `zetl build` to rebuild without the swept recipients, then invalidate the `/c/*` cache on your CDN."*

  The reference to `/c/*` paths and the cap-rotate flow imply that `zetl build` emits encrypted, capability-namespaced HTML; in practice it doesn't.

- **Related:**
  - `REQ-3416` (operator-confirmed-onboarding) — `zetl cap list` is explicitly stubbed against this. Suggests the broader feature is mid-implementation.
  - `REQ-3419` — `cap genkey` works.
  - `REQ-3409` — recipients.toml validation works.
  - `REQ-3404` — hardened/enrol-page mode mentioned in help; `--via enrol-page` requires recipients.toml as expected.

- **Documentation gap:** the canonical `recipients.toml` schema is not documented anywhere reachable from `zetl --help`, the man page, or the help text on `zetl cap`. It can only be reverse-engineered by trial-and-error against the validator's error messages (which are good — the iteration converges in five attempts).

## Environment

- **OS:** macOS 25.3.0 (Darwin)
- **Binary:** `zetl 0.7.0` from `~/.local/bin/zetl`
- **Vault layout:** 22-page personal travel-itinerary wiki with custom theme, no edits to `.zetl/config.toml` required for normal builds.

## Steps to Reproduce

```bash
# 1. Start with a vault that has at least one page containing a known-sensitive string
echo "Policy number: 123-SECRET-456" >> Insurance.md

# 2. Generate cap keys
zetl cap genkey --json
# → { secret.value, signing_key.value, signing_key.public_key }

# 3. Construct recipients.toml (schema reverse-engineered from validator):
PUBKEY_URL=$(echo -n "<public_key from genkey>" | tr '+/' '-_' | tr -d '=')
cat > recipients.toml <<EOF
version = 1

[vault]
signing_pubkey = "ed25519:${PUBKEY_URL}"

[[cohort]]
id = "family"
name = "Family readers"
mode = "delegated-url"
pubkeys = []
EOF

# 4. Issue a delegated-URL invite
export ZETL_CAP_SECRET="<secret.value>"
export ZETL_CAP_SIGNING_KEY="<signing_key.value>"
zetl cap invite "Self" --cohort family --site-url https://example.test
# → emits invite URL with #k=…, writes grants.toml

# 5. Build
rm -rf dist
zetl build --site-url https://example.test
```

## Expected Behaviour

Per the workflow described by the cap CLI verbs:

1. `dist/c/<cohort>/<slug>.html` exists and contains encrypted page bodies, with an in-page loader that decrypts using the URL-fragment key.
2. The plaintext path `dist/<slug>/index.html` either does not exist *or* contains only public-safe content (frontmatter-stripped chrome).
3. `dist/search-index.json` does not contain known-sensitive substrings from any encrypted page.

Reference: the man-page text on `zetl cap sweep` mentions `/c/*` cache invalidation, implying `/c/<cohort>/<slug>` paths are part of the build output contract.

## Actual Behaviour

```bash
$ ls dist/c/
ls: dist/c/: No such file or directory

$ grep -rl "123-SECRET-456" dist/
dist/insurance/index.html
dist/search-index.json
```

The build succeeds with normal output (`21 pages + 3 folder indexes written`), but produces a plaintext static site indistinguishable from a build with no cap state. Sensitive strings are present in:

- `dist/<slug>/index.html` (raw page HTML)
- `dist/search-index.json` (search index)
- `dist/<slug>/index.md` (raw markdown copy)
- `dist/<slug>/_history.html` (history view)

`grants.toml` and `recipients.toml` are correctly populated. Build emits no warning or hint that cap mode is set up but inactive.

## Things Tried

| Variable | Tried | Result |
|---|---|---|
| `ZETL_CAP_SECRET` + `ZETL_CAP_SIGNING_KEY` env vars | yes | no effect |
| recipients.toml `pubkeys = []` (empty) | yes | no effect |
| recipients.toml `pubkeys = ["age-recipient-v1:<from grants.toml>"]` | yes | no effect |
| `.zetl/config.toml` with `[vault] visibility = "capability"` and `[access]` | yes | no effect |
| `--features cap` flag | yes | rejected: "for more information, try '--help'" |
| `--mode cap` build flag | n/a | not present |

`zetl cap list` returns *"not-yet-implemented (SPEC-034 REQ-3416 CLI surface stub)"* — independent confirmation that this corner of the feature is in mid-implementation.

## Evidence

```text
$ zetl --version
zetl 0.7.0

$ zetl build --site-url https://example.test 2>&1 | tail -5
zetl build  →  21 pages + 3 folder indexes written to dist/ (static assets copied)
  brotli: 71 files precompressed
{
  "pages": 21,
  "folder_indexes": 3,
  "out_dir": "dist"
}

$ ls dist/c/ 2>&1
ls: dist/c/: No such file or directory

$ grep -c "Cover-More International" dist/insurance/index.html
4
```

## Confidentiality / Public-Safety Implication

Operator-believed encryption did not happen. If the operator deployed `dist/` to a public host (Cloudflare Pages, S3, GitHub Pages) believing pages are encrypted, all sensitive content is exposed. There is no failure-mode warning at build time.

Recommended hardening once the build is implemented: emit a `[zetl cap] WARNING: cap state is configured but build-time encryption is disabled (see <doc-anchor>)` line whenever `recipients.toml` exists and `ZETL_CAP_SECRET` is set but the build did not engage cap mode.

## Asks (suggested fixes, in priority order)

1. **Document the activation flow** — what config / env / CLI-flag triggers the cap build pipeline. Currently undiscoverable from the binary.
2. **Document `recipients.toml` schema** — currently reverse-engineerable only via the validator's progressive error messages.
3. **Either land the build-time encryption** or **emit a build-time warning** when cap state is configured but cap mode is inactive, so operators don't ship plaintext under the false belief it's encrypted.
4. **Document the partial-implementation status of SPEC-034** in the README / man page so operators can choose alternative approaches (e.g. host-side ACL, server-side encryption gateway) until the build-time half lands.

## Workaround for Operators

Pivot to host-side access control:

- **Cloudflare Access** (zero-trust auth, free for ≤50 users) gates an unencrypted `zetl build` deploy with email-allowlist auth. ~10 minutes setup. Equivalent privacy for personal use; weaker threat model than the cap-mode goal (host can technically read content) but adequate for sharing trip wikis with family etc.
- **`zetl serve` behind Tailscale** — keep the live server on a tailnet, share with intended readers only. Loses static-deploy benefits but preserves cap-mode-equivalent privacy.

## Notes for the fix

- Test fixture: any small vault with `ZETL_CAP_SECRET` + `ZETL_CAP_SIGNING_KEY` set, valid `recipients.toml` and `grants.toml`. Assert presence of `dist/c/<cohort>/` and absence of sensitive strings in `dist/<slug>/index.html` and `dist/search-index.json`.
- A failure-mode test (cap state present but build runs without secrets) should produce a stderr warning and exit non-zero or `--strict-cap` flag could enforce it.
