# Hook Security Model

This document describes the trust posture, isolation guarantees, and
operational limits zetl applies to render-pipeline hooks (SPEC-032 §10).
It is the reference for theme authors writing hooks, vault operators
auditing third-party themes, and anyone running zetl in a CI or
multi-tenant context.

## Threat Model

Zetl treats every hook — vault, theme, or ecosystem-bundled — as
**untrusted code**. The user authoring or installing the hook has
already consented to running it as their own user; zetl's job is to
narrow the blast radius of bugs and misbehaviour, not to defend against
a vault author who deliberately pwns their own machine.

In scope:

- Confining a buggy or hostile hook so it can't OOM, exfiltrate
  unrelated environment variables, or hang the build.
- Surfacing a typed error when a hook violates the protocol so
  REQ-3207 failure-scoping can quarantine the offending hook.
- Making the trust boundary explicit and testable.

Out of scope (v1):

- Targeted exfiltration by a malicious theme author with full FS / network
  reach. Same posture as Obsidian plugins and VS Code extensions:
  **installing the theme is consent to its hooks.**
- Per-hook capability gating (filesystem allowlists, syscall filters).
  Operators wanting harder isolation should run zetl under
  `bwrap`, `firejail`, Docker, or equivalent.
- Cryptographic verification of hook provenance.

## Process Isolation

Hooks run **out-of-process**. The host (zetl) and the hook talk over
line-delimited JSON on the child's stdin/stdout. There is no shared
memory, no shared file descriptor table beyond the three stdio pipes,
and no in-process plugin loader.

- Each persistent-mode hook owns its own subprocess, its own pump
  threads, and its own captured stderr buffer.
- The child inherits the parent user's UID/GID and filesystem
  permissions. Zetl does **not** sandbox or chroot the child in v1.
- A hook crash, panic, or segfault is contained: the host's pump
  threads observe `EOF`, the typed [`ProtocolError::UnexpectedEof`] is
  surfaced, and REQ-3207 owns the recovery decision (continue with the
  rest of the pipeline, or fail the build under
  `--hook-fail-on error`).

## Timeout Enforcement

Every hook invocation is bounded by a wall-clock deadline:

| Phase            | Default | Override                                         |
| ---------------- | ------- | ------------------------------------------------ |
| Handshake        | 5 s     | `PersistentHook::spawn_with_config`              |
| Per-page `run`   | 100 ms  | manifest `timeout_ms`, also passed in JSON `deadline_ms` |
| Shutdown grace   | 1 s     | `PersistentHook::spawn_with_config`              |

When a deadline lapses zetl **hard-kills the child** (SIGKILL on Unix)
and marks the hook instance dead so subsequent calls short-circuit. The
child reaper runs in `Drop`, so a panicking host can't leak orphan
processes — verified by `drop_does_not_leak_children` in
`src/hooks/persistent.rs`.

Tested cases:

- `handshake_timeout_kills_child` — silent hook is killed within the
  configured deadline.
- `run_timeout_kills_child_and_marks_dead` — hung `run` returns the
  typed [`ProtocolError::Timeout`] and the instance is unusable.

## Stdout / Stderr Streaming

Both pipes are pumped on dedicated threads to avoid pipe-buffer
deadlock. Both pumps enforce hard size caps so a misbehaving hook
cannot OOM the host:

- **Stdout** (`max_message_bytes`, default `10 MiB`). A single line
  longer than the cap aborts the read with
  [`ProtocolError::MessageTooLarge { direction: "recv", … }`] and
  kills the child. The pump uses `read_line_capped` to ensure no more
  than `cap + buffer-fill` bytes are allocated even while reading,
  defeating an unbounded-line attack.
- **Stderr** (`max_stderr_bytes`, default `1 MiB`). Captured into a
  bounded buffer; once the cap is hit, further bytes are dropped on
  the floor and a single `[stderr truncated]` marker is appended so
  operators know the captured tail isn't the whole story. Tested by
  `stderr_capped_with_truncation_marker`.

Symmetrically, the host enforces the same `max_message_bytes` on the
**send** side: a host-side bug that tries to push a payload above the
cap fails fast with
[`ProtocolError::MessageTooLarge { direction: "send", … }`] *before*
the bytes reach the wire — the hook never has to see (or get killed
by) an oversized payload it didn't ask for.

## Redact-Env-By-Default

The single largest accidental-leak surface in any subprocess plugin
system is environment-variable inheritance. By default the parent's
secrets (`AWS_SECRET_ACCESS_KEY`, `OPENAI_API_KEY`,
`ANTHROPIC_API_KEY`, `GITHUB_TOKEN`, ad infinitum) are passed straight
through to every child. **Zetl reverses that default.**

When a [`PersistentHook`] is spawned, the child's environment is
**cleared** and only the variables in the
[`SecurityPolicy::env_allowlist`] are copied through from the parent.
The default allowlist (`DEFAULT_ENV_ALLOWLIST`) is the minimum
shebang-and-locale baseline:

| Variable          | Why                                              |
| ----------------- | ------------------------------------------------ |
| `PATH`            | `#!/usr/bin/env python3` shebang resolution      |
| `HOME`            | Python user site-packages, `~/.cache` lookups    |
| `USER`, `LOGNAME` | Some toolchains assert presence                  |
| `LANG`, `LC_*`    | Predictable text encoding                        |
| `TZ`              | Timezone-aware date formatting                   |
| `TMPDIR`/`TEMP`/`TMP` | Where the hook writes scratch files          |
| `TERM`, `SHELL`   | Some stdlibs panic without these                 |

Anything not in this list — including all of the API-key /
cloud-credential / VCS-token namespaces — is **invisible** to the
hook. A hook that runs `echo $AWS_SECRET_ACCESS_KEY` sees the empty
string. A hook that runs `os.environ.get("OPENAI_API_KEY", "<unset>")`
gets `"<unset>"`. Verified by `env_redacted_by_default` in
`src/hooks/persistent.rs`.

Two escape hatches exist for hooks that genuinely need a non-default
variable:

```rust
// Add a single var to the default allowlist:
let policy = SecurityPolicy::default()
    .with_extra_env(["VIRTUAL_ENV"]);

// Or replace the allowlist wholesale (CI runner with bespoke vars):
let policy = SecurityPolicy::default()
    .with_env_allowlist(["PATH", "HOME", "BUILDKITE_BUILD_ID"]);

PersistentHook::spawn_with_policy(cmd, "id", stage,
    DEFAULT_HANDSHAKE_TIMEOUT, DEFAULT_SHUTDOWN_GRACE, policy)?;
```

Explicit `cmd.env(...)` calls made on the [`Command`] **before** spawn
always survive redaction — they are the documented mechanism for
zetl to surface its own `ZETL_*` context vars (REQ-3220). Caller
intent always wins over the parent-env passthrough. Verified by
`explicit_command_env_survives_redaction`.

## Untrusted JSON Deserialisation

Hooks send JSON back; zetl deserialises it. SPEC-032 §10's mitigations
apply:

- **`max_message_bytes` cap** (10 MiB default) — bounds memory
  per-message. See above.
- **Recursion depth** — `serde_json` parses iteratively for arrays and
  objects (no stack-frame-per-level recursion); CommonMark's nesting
  cap of 256 is enforced at the AST-validation layer (REQ-3202),
  so deeply-nested user content can't reach the deserialiser unbounded.
- **Schema validation** — typed AST documents are validated against
  `tools/zetl-ast-schema-v1.json` at the transform-stage boundary
  (REQ-3221).

## Operational Hardening

For environments where the v1 isolation defaults aren't enough:

- **Run zetl under `bwrap` / `firejail` / Docker.** All three honour
  the read-only-FS, no-network, and namespace-isolation knobs zetl
  intentionally doesn't replicate.
- **Disable theme hooks via `--safe-mode`** (REQ-3223). Suppresses
  every theme-supplied hook for the build; only vault hooks run.
- **Audit the theme's `[[theme.hooks]]` declaration** before
  installing — it lists every hook the theme will register, so a
  surprise `pre-build` script in a fresh release is visible at
  manifest-review time.
- **Pin hook versions** in vault config. The handshake's `version`
  field is logged into OBS-3201 for every build; an unexpected bump
  surfaces in the activity log.

## Tested Acceptance

Every claim above is exercised by a test in
`src/hooks/persistent.rs::tests`:

| Claim                                       | Test                                              |
| ------------------------------------------- | ------------------------------------------------- |
| Parent secrets are invisible to hooks       | `env_redacted_by_default`                         |
| `PATH` / `HOME` reach the hook              | `env_allowlist_passes_path`                       |
| Caller `cmd.env(...)` survives redaction    | `explicit_command_env_survives_redaction`         |
| Per-var opt-in restores a single var        | `extra_env_allowlist_opts_specific_vars_back_in`  |
| Stderr is bounded with truncation marker    | `stderr_capped_with_truncation_marker`            |
| Oversized hook response fails typed         | `oversized_response_line_returns_typed_error`     |
| Oversized host send fails typed             | `oversized_send_payload_returns_typed_error`      |
| Default caps match SPEC-032 §10             | `default_policy_carries_spec_32_caps`             |
| Hook past per-call deadline is killed       | `run_timeout_kills_child_and_marks_dead`          |
| Silent hook past handshake deadline killed  | `handshake_timeout_kills_child`                   |
| Drop reaps even an EOF-ignoring child       | `drop_does_not_leak_children`                     |

## Cross-References

- SPEC-032 §10 — normative security requirements.
- SPEC-032 REQ-3207 — failure-scoping (how a security trip-wire turns
  into a quarantined hook rather than a broken build).
- SPEC-032 REQ-3220 — `ZETL_*` env vars passed to every hook.
- SPEC-032 REQ-3223 — `--safe-mode` and `[[theme.hooks]]` declaration.
- `src/hooks/persistent.rs` — the [`SecurityPolicy`] implementation
  and the test matrix above.
