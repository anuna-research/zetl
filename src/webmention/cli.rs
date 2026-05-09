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
    /// Source URL (the page that links to you).
    pub source: String,
    /// Target URL (your page that's being linked).
    pub target: String,
}

#[derive(Args, Debug, Clone)]
pub struct WebmentionSendArgs {
    /// Page slug or URL whose outbound mentions to re-send.
    pub page: String,
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
    fn accept_requires_source_and_target() {
        let cli = Wrapper::try_parse_from([
            "zetl",
            "accept",
            "https://a.example/",
            "https://me.example/p",
        ])
        .unwrap();
        match cli.command {
            WebmentionCommand::Accept(args) => {
                assert_eq!(args.source, "https://a.example/");
                assert_eq!(args.target, "https://me.example/p");
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
