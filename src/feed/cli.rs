//! `zetl feed` subcommand surface per CON-3801 + CON-3806.
//!
//! This module exposes the clap argument types only. The actual
//! handlers live in the shell (`src/main.rs`); each subcommand
//! reduces to a one-liner that calls into one of the leaf modules:
//!
//!   * `pull` -> [`crate::feed::fetch`] + [`crate::feed::inbound`]
//!   * `list` / `status` -> reads from `.zetl/feeds/*/state.json`
//!   * `validate` -> per-format strict parsers (Phase 10 wires this)
//!   * `forget` -> [`crate::feed::forget::plan_forget`]

use clap::{Args, Subcommand};

#[derive(Subcommand, Debug, Clone)]
pub enum FeedCommand {
    /// Fetch one or more subscribed feeds (or all if no ids given).
    Pull(FeedPullArgs),
    /// Show all subscriptions in tabular form.
    List(FeedListArgs),
    /// Show detailed status for a single subscription.
    Status(FeedStatusArgs),
    /// Validate a feed file or stdin against the per-format strict
    /// parsers.
    Validate(FeedValidateArgs),
    /// Erase items from an inbox / archive plus mint tombstone records.
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
    /// Force a particular format ('rss', 'atom', 'jsonfeed').
    /// Auto-detected by default from Content-Type or file extension.
    #[arg(long)]
    pub format: Option<String>,
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
