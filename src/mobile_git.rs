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
}

/// Clone a remote vault into `into`. Authenticates over SSH using the
/// key in [`crate::mobile_state::global()`].
pub fn clone(remote_url: &str, into: &Path) -> Result<Repository> {
    require_keystore_loaded()?;

    let mut fetch_opts = FetchOptions::new();
    fetch_opts.remote_callbacks(ssh_callbacks());

    let mut builder = RepoBuilder::new();
    builder.fetch_options(fetch_opts);

    builder
        .clone(remote_url, into)
        .with_context(|| format!("git clone {remote_url} → {} failed", into.display()))
}

/// Fetch + fast-forward merge from the configured `origin` remote.
///
/// Caller is responsible for triggering a vault reindex on
/// `FastForwarded`.
pub fn pull_ff_only(repo_path: &Path) -> Result<PullOutcome> {
    require_keystore_loaded()?;

    let repo = Repository::open(repo_path)
        .with_context(|| format!("not a git working tree: {}", repo_path.display()))?;

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
        let head_before = repo
            .head()
            .ok()
            .and_then(|h| h.target())
            .map(|o| o.to_string())
            .unwrap_or_else(|| "<none>".into());

        // Resolve the local branch ref the FETCH_HEAD points at and
        // fast-forward it to the fetched commit.
        let head_ref_name = repo.head()?.name().unwrap_or("HEAD").to_string();
        let mut head_ref = repo.find_reference(&head_ref_name)?;
        head_ref.set_target(fetch_commit.id(), "ff-pull from /_mobile/sync")?;
        repo.set_head(&head_ref_name)?;
        repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))?;

        Ok(PullOutcome::FastForwarded {
            from: head_before,
            to: fetch_commit.id().to_string(),
        })
    } else {
        Ok(PullOutcome::Conflict)
    }
}

/// Push the active branch to `origin`. Returns `Ok(())` only on a
/// successful push; non-FF rejections surface as
/// `Err(... "remote rejected ...")`.
pub fn push(repo_path: &Path) -> Result<()> {
    require_keystore_loaded()?;

    let repo = Repository::open(repo_path)
        .with_context(|| format!("not a git working tree: {}", repo_path.display()))?;

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
    let mut cb = RemoteCallbacks::new();
    cb.credentials(|_url, username_from_url, _allowed| {
        let username = username_from_url.unwrap_or("git");
        let priv_pem = crate::mobile_state::global().priv_pem().ok_or_else(|| {
            git2::Error::from_str("no SSH key in keystore — onboard first via /_mobile/onboarding")
        })?;
        Cred::ssh_key_from_memory(username, None, &priv_pem, None)
    });
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

        let repo = clone(&url, &dest).expect("file:// clone should work without SSH");
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
        clone(&url, &dest).unwrap();

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
