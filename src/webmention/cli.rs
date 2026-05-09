//! `zetl webmention` clap argument types per CON-3903 + CON-3908.
//!
//! Mirrors the layout of [`crate::feed::cli`]: this module exposes the
//! argument types only; handler dispatch lives in `src/main.rs`.

use clap::{Args, Subcommand};

#[derive(Subcommand, Debug, Clone)]
pub enum WebmentionCommand {
    /// List queued and accepted webmentions on this vault.
    List(WebmentionListArgs),
    /// Promote a queued mention to accepted (manual moderator decision).
    Accept(WebmentionDecisionArgs),
    /// Tombstone a mention — removes from the live edge set.
    Reject(WebmentionDecisionArgs),
    /// Force-resend the outbound POSTs for one page (escape hatch when
    /// an earlier build's POST failed).
    Send(WebmentionSendArgs),
    /// Show counters — queue depth, accepted, sent, denied.
    Status(WebmentionStatusArgs),
}

#[derive(Args, Debug, Clone)]
pub struct WebmentionListArgs {
    /// Show only queued (pending moderation) entries.
    #[arg(long)]
    pub queued: bool,
    /// Show only accepted (live) edges.
    #[arg(long)]
    pub accepted: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug, Clone)]
pub struct WebmentionDecisionArgs {
    /// Either a numeric index from the most recent `webmention list`
    /// (1-based), OR the source URL of the queued mention. When numeric,
    /// `target` MUST be omitted.
    pub source_or_index: String,
    /// Target URL (required iff `source_or_index` is a URL).
    pub target: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct WebmentionSendArgs {
    /// Path to the rendered HTML file or `dist/` root to scan. When a
    /// directory, every `*.html` under it is processed.
    pub path: std::path::PathBuf,
    /// Show what would be POSTed without actually issuing the requests.
    #[arg(long)]
    pub dry_run: bool,
    /// Vault base URL — required so external-link extraction can
    /// distinguish self-links from external. Falls back to
    /// `[feed].base_url` from `.zetl/config.toml` when omitted.
    #[arg(long)]
    pub base_url: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct WebmentionStatusArgs {
    #[arg(long)]
    pub json: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser)]
    struct Wrapper {
        #[command(subcommand)]
        command: WebmentionCommand,
    }

    #[test]
    fn list_no_args_parses() {
        let cli = Wrapper::try_parse_from(["zetl", "list"]).unwrap();
        assert!(matches!(cli.command, WebmentionCommand::List(_)));
    }

    #[test]
    fn list_with_filter_flags() {
        let cli = Wrapper::try_parse_from(["zetl", "list", "--queued", "--json"]).unwrap();
        match cli.command {
            WebmentionCommand::List(args) => {
                assert!(args.queued);
                assert!(args.json);
                assert!(!args.accepted);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn accept_takes_source_and_target_urls() {
        let cli = Wrapper::try_parse_from([
            "zetl",
            "accept",
            "https://a.example/",
            "https://me.example/p",
        ])
        .unwrap();
        match cli.command {
            WebmentionCommand::Accept(args) => {
                assert_eq!(args.source_or_index, "https://a.example/");
                assert_eq!(args.target.as_deref(), Some("https://me.example/p"));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn accept_takes_numeric_index() {
        let cli = Wrapper::try_parse_from(["zetl", "accept", "3"]).unwrap();
        match cli.command {
            WebmentionCommand::Accept(args) => {
                assert_eq!(args.source_or_index, "3");
                assert!(args.target.is_none());
            }
            _ => panic!(),
        }
    }

    #[test]
    fn status_with_json_flag() {
        let cli = Wrapper::try_parse_from(["zetl", "status", "--json"]).unwrap();
        match cli.command {
            WebmentionCommand::Status(args) => assert!(args.json),
            _ => panic!(),
        }
    }
}
