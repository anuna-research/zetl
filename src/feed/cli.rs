//! `zetl feed` subcommand surface per CON-3801 + CON-3806.
//!
//! Naming clarification: SPEC-038 uses the word "feed" on three axes:
//!
//!   * **outbound publishing** — `[feed]` config + `dist/feed.xml`
//!     emitted by `zetl build`. There is no `zetl feed publish` CLI;
//!     the build pipeline handles it automatically.
//!   * **inbound subscriptions** — `[[subscriptions]]` config + items
//!     ingested into `.zetl/feeds/<sub-id>/inbox/`. THIS is what every
//!     subcommand below operates on. (The dir name `.zetl/feeds/`
//!     also reflects this — it stores INBOUND state.)
//!   * **per-page opt-in** — `frontmatter.feed: true` flags a page
//!     for inclusion in the OUTBOUND root feed.
//!
//! `zetl feed pull|list|status|validate|forget` are all inbound-side
//! commands; per-subcommand help text below makes that explicit.
//!
//! This module exposes the clap argument types only. The actual
//! handlers live in the shell (`src/main.rs`); each subcommand
//! reduces to a one-liner that calls into one of the leaf modules:
//!
//!   * `pull` -> [`crate::feed::fetch`] + [`crate::feed::inbound`]
//!   * `list` / `status` -> reads from `.zetl/feeds/*/state.json`
//!   * `validate` -> per-format strict parsers (Phase 10 wires this)
//!   * `forget` -> [`crate::feed::forget::plan_forget`]

use clap::{Args, Subcommand, ValueEnum};

/// Feed wire format for `zetl feed validate`. Closed set; clap rejects
/// unknown values at parse time. Auto-detection happens when the flag
/// is omitted.
#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedValidateFormat {
    Rss,
    Atom,
    Jsonfeed,
}

impl FeedValidateFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            FeedValidateFormat::Rss => "rss",
            FeedValidateFormat::Atom => "atom",
            FeedValidateFormat::Jsonfeed => "jsonfeed",
        }
    }
}

#[derive(Subcommand, Debug, Clone)]
pub enum FeedCommand {
    /// (inbound) Fetch one or more subscribed upstream feeds and ingest
    /// their items into `.zetl/feeds/<sub-id>/inbox/`. Outbound feed
    /// publishing happens automatically inside `zetl build` and has no
    /// dedicated subcommand.
    Pull(FeedPullArgs),
    /// (inbound) Show all configured `[[subscriptions]]` in tabular
    /// form with last-success / last-error / item-count columns.
    List(FeedListArgs),
    /// (inbound) Show detailed status for a single subscription:
    /// dedup-state size, retention policy, recent error log lines.
    Status(FeedStatusArgs),
    /// (inbound + outbound) Validate a feed file or stdin against the
    /// per-format strict parsers. Used to QA both upstream feeds
    /// before subscribing AND your own published feeds before deploy.
    Validate(FeedValidateArgs),
    /// (inbound) Erase items from a subscription's inbox / archive,
    /// minting tombstone records that block re-import on subsequent
    /// pulls. Does NOT remove anything from your own published feed —
    /// for that, edit / delete the source page and rebuild.
    Forget(FeedForgetArgs),
}

#[derive(Args, Debug, Clone)]
pub struct FeedPullArgs {
    /// Subscription ids to pull. Omit to pull every configured
    /// subscription.
    #[arg(value_name = "SUB_ID")]
    pub subscription_ids: Vec<String>,
    /// Disable interactive prompts (e.g. for failing health checks).
    #[arg(long)]
    pub no_input: bool,
    /// Emit JSON output regardless of stdout TTY status.
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug, Clone)]
pub struct FeedListArgs {
    /// Emit JSON output regardless of stdout TTY status.
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug, Clone)]
pub struct FeedStatusArgs {
    pub subscription_id: String,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug, Clone)]
pub struct FeedValidateArgs {
    /// Path to a feed file. Reads stdin when omitted.
    #[arg(value_name = "PATH")]
    pub path: Option<std::path::PathBuf>,
    /// Force a particular feed format. Auto-detected by default from
    /// the body shape (`{` → jsonfeed, `<feed` → atom, otherwise rss).
    /// Named `--feed-format` rather than `--format` to avoid colliding
    /// with the global `--format` flag that selects table vs JSON
    /// output for the rest of zetl.
    #[arg(long = "feed-format", value_enum)]
    pub feed_format: Option<FeedValidateFormat>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug, Clone)]
pub struct FeedForgetArgs {
    /// Subscription id to operate on.
    pub subscription_id: String,
    /// Pattern: slug glob, GUID prefix, or content-hash prefix.
    pub pattern: String,
    /// Also remove from archived/.
    #[arg(long)]
    pub include_archive: bool,
    /// Free-form reason captured in tombstone records.
    #[arg(long)]
    pub reason: Option<String>,
    /// Print what would be removed without writing.
    #[arg(long)]
    pub dry_run: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser)]
    struct Wrapper {
        #[command(subcommand)]
        command: FeedCommand,
    }

    #[test]
    fn pull_no_args_parses() {
        let cli = Wrapper::try_parse_from(["zetl", "pull"]).unwrap();
        match cli.command {
            FeedCommand::Pull(args) => {
                assert!(args.subscription_ids.is_empty());
            }
            _ => panic!(),
        }
    }

    #[test]
    fn pull_with_subscription_ids() {
        let cli = Wrapper::try_parse_from(["zetl", "pull", "a", "b"]).unwrap();
        match cli.command {
            FeedCommand::Pull(args) => assert_eq!(args.subscription_ids, vec!["a", "b"]),
            _ => panic!(),
        }
    }

    #[test]
    fn forget_with_dry_run_and_reason() {
        let cli = Wrapper::try_parse_from([
            "zetl",
            "forget",
            "x",
            "**/*.md",
            "--dry-run",
            "--reason",
            "bug",
        ])
        .unwrap();
        match cli.command {
            FeedCommand::Forget(args) => {
                assert_eq!(args.subscription_id, "x");
                assert_eq!(args.pattern, "**/*.md");
                assert!(args.dry_run);
                assert_eq!(args.reason.as_deref(), Some("bug"));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn validate_with_path() {
        let cli = Wrapper::try_parse_from(["zetl", "validate", "feed.xml"]).unwrap();
        match cli.command {
            FeedCommand::Validate(args) => assert!(args.path.is_some()),
            _ => panic!(),
        }
    }
}
