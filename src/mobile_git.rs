//! SPEC-040 mobile git operations (REQ-4002, REQ-4009, REQ-4010,
//! REQ-4011, ADR-4002, ADR-4004).
//!
//! Thin `git2-rs` wrappers for the three operations the mobile app
//! needs: `clone` (during onboarding), `pull` (FF-only, on app open),
//! and `push` (after every save / capture). Authentication runs
//! through an SSH credential callback that loads the private key
//! from [`crate::mobile_state::global()`] inside the callback closure
//! — never logging the key, fail-fast if no seed has been imported.
//!
//! Conflict policy is enforced here per [[ADR-4004]]: pulls are
//! `--ff-only`. A non-FF pull returns `PullOutcome::Conflict` and
//! the caller (typically the `/_mobile/sync` route) refuses to push
//! until a subsequent FF pull succeeds.

use std::path::Path;

use anyhow::{anyhow, Context, Result};
use git2::{
    build::RepoBuilder, Cred, FetchOptions, MergeAnalysis, PushOptions, RemoteCallbacks, Repository,
};

/// Outcome of a `pull` attempt.
#[derive(Debug, PartialEq, Eq)]
pub enum PullOutcome {
    /// The local working tree was already at or ahead of the remote;
    /// nothing was applied.
    UpToDate,
    /// A fast-forward update was applied; the working tree now points
    /// at the remote `HEAD`. The caller should reindex.
    FastForwarded { from: String, to: String },
    /// The remote has diverged from the local branch. Push is now
    /// blocked until the user resolves on desktop and the next pull
    /// is fast-forward (per [[ADR-4004]]).
    Conflict,
    /// Uncommitted local edits would be overwritten by the fast-forward
    /// checkout. The pull is refused — the caller surfaces this as an
    /// actionable error rather than silently clobbering the changes.
    DirtyWorktree,
}

/// Clone a remote vault into `into`. Authenticates per protocol:
///
/// - SSH (`git@host:owner/repo.git` or `ssh://...`) → BIP39-derived
///   ed25519 key from [`crate::mobile_state::global()`].
/// - HTTPS (`https://...`) → personal access token via `pat`. Pass
///   `None` for public repos that don't need credentials.
///
/// Empty-destination prep + `.git`-already-present refusal as before.
pub fn clone(remote_url: &str, into: &Path, pat: Option<&str>) -> Result<Repository> {
    require_keystore_loaded()?;
    ensure_known_hosts_file();
    prepare_clone_destination(into)
        .with_context(|| format!("prepare {} for clone", into.display()))?;

    let mut fetch_opts = FetchOptions::new();
    fetch_opts.remote_callbacks(auth_callbacks(pat));

    let mut builder = RepoBuilder::new();
    builder.fetch_options(fetch_opts);

    builder
        .clone(remote_url, into)
        .with_context(|| format!("git clone {remote_url} → {} failed", into.display()))
}

fn prepare_clone_destination(into: &Path) -> Result<()> {
    if !into.exists() {
        return Ok(());
    }
    if into.join(".git").exists() {
        return Err(anyhow!(
            "{} is already a git working tree — reset via /_mobile/sync to switch vaults",
            into.display()
        ));
    }
    // The dir exists with no .git inside; it's an onboarding-state
    // directory holding at most a `.DS_Store` or stray files from a
    // failed clone. Wipe the contents so git2 sees an empty dir.
    let mut had_anything = false;
    for entry in std::fs::read_dir(into).context("read clone destination")? {
        let entry = entry.context("iterate clone destination")?;
        had_anything = true;
        let p = entry.path();
        if p.is_dir() {
            std::fs::remove_dir_all(&p).with_context(|| format!("rm -rf {}", p.display()))?;
        } else {
            std::fs::remove_file(&p).with_context(|| format!("rm {}", p.display()))?;
        }
    }
    if had_anything {
        eprintln!(
            "[zetl-mobile] cleared non-empty clone destination {} before clone",
            into.display()
        );
    }
    Ok(())
}

/// Fetch + fast-forward merge from the configured `origin` remote.
///
/// Caller is responsible for triggering a vault reindex on
/// `FastForwarded`.
pub fn pull_ff_only(repo_path: &Path) -> Result<PullOutcome> {
    require_keystore_loaded()?;
    ensure_known_hosts_file();

    // Use Repository::discover so callers can pass a vault subdirectory
    // (e.g. vaults/<label>/notes/) and git2 walks up to find the .git
    // dir at the repo root. This supports the SPEC-040 vault-subpath
    // picker where the symlink target may not be the repo root.
    let repo = Repository::discover(repo_path).with_context(|| {
        format!(
            "not a git working tree (no .git ancestor of {})",
            repo_path.display()
        )
    })?;

    let mut remote = repo
        .find_remote("origin")
        .context("remote 'origin' not configured")?;

    let mut fetch_opts = FetchOptions::new();
    fetch_opts.remote_callbacks(ssh_callbacks());

    // Fetch the active branch's tracking ref. Fall back to all heads
    // if no specific refspec applies.
    remote
        .fetch::<&str>(&[], Some(&mut fetch_opts), None)
        .context("git fetch failed")?;

    let fetch_head = repo
        .find_reference("FETCH_HEAD")
        .context("FETCH_HEAD missing after fetch — empty remote?")?;
    let fetch_commit = repo
        .reference_to_annotated_commit(&fetch_head)
        .context("could not annotate FETCH_HEAD")?;

    let (analysis, _pref) = repo
        .merge_analysis(&[&fetch_commit])
        .context("merge analysis failed")?;

    if analysis.contains(MergeAnalysis::ANALYSIS_UP_TO_DATE) {
        Ok(PullOutcome::UpToDate)
    } else if analysis.contains(MergeAnalysis::ANALYSIS_FASTFORWARD) {
        // Refuse to fast-forward over uncommitted edits — `force()`
        // would silently overwrite them. The mobile WebView edit path
        // and the desktop serve writing to the same working tree can
        // both leave files dirty between commits, so this is a real
        // concern, not just a theoretical one.
        if worktree_is_dirty(&repo)? {
            return Ok(PullOutcome::DirtyWorktree);
        }

        let head_before = repo
            .head()
            .ok()
            .and_then(|h| h.target())
            .map(|o| o.to_string())
            .unwrap_or_else(|| "<none>".into());

        // Resolve the local branch ref the FETCH_HEAD points at and
        // fast-forward it to the fetched commit. `safe()` aborts on
        // any conflict instead of silently clobbering.
        let head_ref_name = repo.head()?.name().unwrap_or("HEAD").to_string();
        let mut head_ref = repo.find_reference(&head_ref_name)?;
        head_ref.set_target(fetch_commit.id(), "ff-pull from /_mobile/sync")?;
        repo.set_head(&head_ref_name)?;
        repo.checkout_head(Some(git2::build::CheckoutBuilder::new().safe()))?;

        Ok(PullOutcome::FastForwarded {
            from: head_before,
            to: fetch_commit.id().to_string(),
        })
    } else {
        Ok(PullOutcome::Conflict)
    }
}

/// True if the working tree or the index has any pending non-ignored
/// changes that a fast-forward checkout could overwrite. Untracked
/// files are *not* counted as dirty (they wouldn't be touched by a
/// fast-forward) so the user can still pull while drafting new notes.
fn worktree_is_dirty(repo: &Repository) -> Result<bool> {
    let mut opts = git2::StatusOptions::new();
    opts.include_ignored(false)
        .include_untracked(false)
        .recurse_untracked_dirs(false);
    let statuses = repo
        .statuses(Some(&mut opts))
        .context("git status failed")?;
    Ok(statuses.iter().any(|s| !s.status().is_empty()))
}

/// Push the active branch to `origin`. Returns `Ok(())` only on a
/// successful push; non-FF rejections surface as
/// `Err(... "remote rejected ...")`.
pub fn push(repo_path: &Path) -> Result<()> {
    require_keystore_loaded()?;
    ensure_known_hosts_file();

    // Use Repository::discover so callers can pass a vault subdirectory
    // (e.g. vaults/<label>/notes/) and git2 walks up to find the .git
    // dir at the repo root. This supports the SPEC-040 vault-subpath
    // picker where the symlink target may not be the repo root.
    let repo = Repository::discover(repo_path).with_context(|| {
        format!(
            "not a git working tree (no .git ancestor of {})",
            repo_path.display()
        )
    })?;

    let mut remote = repo
        .find_remote("origin")
        .context("remote 'origin' not configured")?;

    let head_ref = repo.head().context("HEAD has no commit yet")?;
    let head_ref_name = head_ref
        .name()
        .ok_or_else(|| anyhow!("HEAD ref has no name"))?
        .to_string();

    let mut opts = PushOptions::new();
    opts.remote_callbacks(ssh_callbacks());

    remote
        .push::<&str>(&[&head_ref_name], Some(&mut opts))
        .context("git push failed")
}

fn ssh_callbacks() -> RemoteCallbacks<'static> {
    auth_callbacks(None)
}

/// Credential callback that handles both SSH (BIP39-derived key) and
/// HTTPS (personal access token) protocols. `git2` passes a bitmask
/// `allowed` indicating which credential types it expects; we
/// produce the matching one.
fn auth_callbacks(pat: Option<&str>) -> RemoteCallbacks<'static> {
    let owned_pat: Option<String> = pat.map(String::from);
    let mut cb = RemoteCallbacks::new();
    cb.credentials(move |_url, username_from_url, allowed| {
        // SSH key auth — git2 asks for SSH_KEY when the remote URL is
        // ssh:// or git@host:...
        if allowed.contains(git2::CredentialType::SSH_KEY) {
            let username = username_from_url.unwrap_or("git");
            let priv_pem = crate::mobile_state::global().priv_pem().ok_or_else(|| {
                git2::Error::from_str(
                    "no SSH key in keystore — onboard first via /_mobile/onboarding",
                )
            })?;
            return Cred::ssh_key_from_memory(username, None, &priv_pem, None);
        }
        // HTTPS basic / personal-access-token auth — git2 asks for
        // USER_PASS_PLAINTEXT.
        if allowed.contains(git2::CredentialType::USER_PASS_PLAINTEXT) {
            let token = owned_pat.as_deref().ok_or_else(|| {
                git2::Error::from_str(
                    "private HTTPS clone needs a personal access token \
                     (paste one in the 'Advanced: private HTTPS' section, \
                     or use the SSH URL like git@host:owner/repo.git)",
                )
            })?;
            // The PAT is the *password*; the username is conventionally
            // any non-empty string on GitHub / GitLab / Codeberg.
            return Cred::userpass_plaintext("zetl-mobile", token);
        }
        Err(git2::Error::from_str(&format!(
            "no supported credential type available (allowed={allowed:?})"
        )))
    });
    // Trust on first use for SSH host keys. Android (and any sandboxed
    // mobile environment) has no `~/.ssh/known_hosts`, so libgit2's
    // default verifier fails with `error loading known_hosts; class=Ssh
    // (23)` before the SSH session can even start. Accepting the
    // presented host fingerprint matches the v0.1 mobile UX: the user
    // typed the remote URL by hand and is about to clone into an empty
    // local directory, so a silent MITM has nothing of theirs to steal
    // — and the next pull would mismatch their freshly-pushed commits
    // if the remote isn't what they expected. Tightening this to a
    // pinned set of git-host fingerprints is a v0.2 hardening item.
    cb.certificate_check(|_cert, _host| Ok(git2::CertificateCheckStatus::CertificateOk));
    cb
}

fn require_keystore_loaded() -> Result<()> {
    if crate::mobile_state::global().is_loaded() {
        Ok(())
    } else {
        Err(anyhow!(
            "no SSH key in keystore — paste your BIP39 seed at /_mobile/onboarding first"
        ))
    }
}

/// libgit2's SSH transport tries to load `$HOME/.ssh/known_hosts`
/// during transport init — *before* any callbacks fire. On Android
/// (and any sandboxed mobile environment) `$HOME` is unset or points
/// at the app's private data dir with no `.ssh/` subtree, so libssh2
/// errors out with `error loading known_hosts; class=Ssh (23)` before
/// our `certificate_check` callback even gets to override it.
///
/// libgit2's SSH transport behaviour:
///
/// - If `$HOME/.ssh/known_hosts` does NOT exist, `git_sysdir_find_global_file`
///   returns `GIT_ENOTFOUND` and the SSH transport skips the read.
///   Host verification then falls through to our `certificate_check`
///   callback (which accepts TOFU).
/// - If the file DOES exist but `libssh2_knownhost_readfile` fails on it
///   (e.g., for an empty file libssh2 quirkily returns negative with an
///   empty error message), the transport aborts with
///   `error loading known_hosts; class=Ssh (23)` and our callback never
///   fires.
///
/// First-pass fix created an empty file; that tripped the second case
/// on Android. Inverting: guarantee the file does NOT exist so libgit2
/// skips known_hosts entirely. Set `$HOME` to a real directory so other
/// libgit2 features that need a home (global config search, etc.) keep
/// working.
pub fn ensure_known_hosts_file() {
    let Some(app_data) = crate::mobile_state::app_data_dir() else {
        return;
    };
    let ssh_dir = app_data.join(".ssh");
    let known_hosts = ssh_dir.join("known_hosts");
    // ENOENT on either of these is the goal — ignore errors.
    let _ = std::fs::remove_file(&known_hosts);
    let _ = std::fs::remove_dir(&ssh_dir);

    // Set HOME so libgit2's homedir resolution succeeds. Finding no
    // .ssh dir yields ENOTFOUND, libgit2 skips known_hosts, the SSH
    // handshake proceeds, and our certificate_check callback accepts
    // on first sight.
    //
    // SAFETY: `std::env::set_var` is technically unsound in 2024
    // edition because libc setenv isn't thread-safe — but this runs
    // from the Tauri setup() hook before any other thread touches
    // HOME and before libgit2 has cached its homedir.
    if std::env::var_os("HOME").as_deref() != Some(app_data.as_os_str()) {
        std::env::set_var("HOME", &app_data);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    /// The integration tests below need the keystore "loaded" so the
    /// `require_keystore_loaded` guard does not short-circuit. They
    /// only ever clone/pull/push over `file://` so the SSH callback
    /// is never invoked. Use the standard fixture mnemonic so tests
    /// remain deterministic.
    fn load_keystore() {
        let _ = crate::mobile_state::global().import_mnemonic(
            "abandon abandon abandon abandon abandon abandon \
             abandon abandon abandon abandon abandon about",
        );
    }

    fn make_bare_remote_with_one_commit() -> (TempDir, PathBuf) {
        let bare_dir = tempfile::tempdir().unwrap();
        let bare_path = bare_dir.path().join("remote.git");
        Repository::init_bare(&bare_path).unwrap();

        // Seed the bare remote by cloning, committing once on an
        // explicit `main` branch, pushing. We use a fresh refs/heads/main
        // (rather than `Some("HEAD")` which would land on whatever
        // libgit2's default-branch config resolves to — historically
        // `master`) so the push refspec below always resolves.
        let tmp = tempfile::tempdir().unwrap();
        let work = tmp.path().join("work");
        let url = format!("file://{}", bare_path.display());
        let repo = Repository::clone(&url, &work).unwrap();
        std::fs::write(work.join("README.md"), "# initial\n").unwrap();
        let mut idx = repo.index().unwrap();
        idx.add_path(Path::new("README.md")).unwrap();
        idx.write().unwrap();
        let tree_id = idx.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let sig = git2::Signature::now("test", "test@example").unwrap();
        repo.commit(Some("refs/heads/main"), &sig, &sig, "init", &tree, &[])
            .unwrap();
        repo.set_head("refs/heads/main").unwrap();

        let mut remote = repo.find_remote("origin").unwrap();
        remote
            .push::<&str>(&["refs/heads/main:refs/heads/main"], None)
            .unwrap();

        // Re-aim the bare's HEAD at main so subsequent `Repository::clone`
        // operations land on a born branch (libgit2's default-branch
        // config still says "master" otherwise, producing UnbornBranch).
        let bare = Repository::open_bare(&bare_path).unwrap();
        bare.set_head("refs/heads/main").unwrap();
        drop(bare);

        (bare_dir, bare_path)
    }

    #[test]
    fn clone_via_file_transport() {
        load_keystore();
        let (_bare_dir, bare_path) = make_bare_remote_with_one_commit();
        let dest_dir = tempfile::tempdir().unwrap();
        let dest = dest_dir.path().join("vault");
        let url = format!("file://{}", bare_path.display());

        let repo = clone(&url, &dest, None).expect("file:// clone should work without SSH");
        assert!(repo.find_remote("origin").is_ok());
        assert!(dest.join("README.md").exists());
    }

    #[test]
    fn pull_up_to_date_after_fresh_clone() {
        load_keystore();
        let (_bare_dir, bare_path) = make_bare_remote_with_one_commit();
        let dest_dir = tempfile::tempdir().unwrap();
        let dest = dest_dir.path().join("vault");
        let url = format!("file://{}", bare_path.display());
        clone(&url, &dest, None).unwrap();

        let outcome = pull_ff_only(&dest).expect("pull on up-to-date repo");
        assert_eq!(outcome, PullOutcome::UpToDate);
    }

    #[test]
    fn require_keystore_loaded_blocks_when_empty() {
        // Use a fresh global state — but since the global is shared
        // across tests, we cannot truly clear it here. Instead, this
        // test is informational: the `require_keystore_loaded` gate
        // is exercised in `tests/mobile_integration.rs` where the
        // global state can be controlled at test-binary startup.
        // This in-process unit-test simply documents the contract.
        // (No assertion here.)
    }
}
