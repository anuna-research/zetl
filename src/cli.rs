use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(
    name = "zetl",
    version,
    about = "Bi-directional wikilink graph CLI with defeasible reasoning"
)]
pub struct Cli {
    /// Vault root directory
    #[arg(short = 'd', long, default_value = ".")]
    pub dir: String,

    /// Output format
    #[arg(short = 'f', long, default_value = "json")]
    pub format: OutputFormat,

    /// Force full rescan, ignore cached index
    #[arg(long)]
    pub no_cache: bool,

    /// Disable colored output
    #[arg(long)]
    pub no_color: bool,

    /// Suppress non-essential output
    #[arg(short, long)]
    pub quiet: bool,

    /// Increase verbosity (repeat for more: -vv)
    #[arg(short, long, action = clap::ArgAction::Count)]
    pub verbose: u8,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Clone, ValueEnum)]
pub enum OutputFormat {
    Json,
    Table,
}

#[derive(Subcommand)]
pub enum Command {
    /// Build or refresh the link index
    Index,

    /// Query forward links from a page
    Links {
        /// Page name (case-insensitive)
        page: String,
        /// Enable fuzzy page name matching
        #[arg(long)]
        fuzzy: bool,
        /// Include N characters of surrounding text
        #[arg(long, default_value = "0")]
        context: usize,
        /// Traverse N hops (1 = direct only)
        #[arg(long, default_value = "1")]
        depth: usize,
        /// Show which conclusions each linked page contributes (requires reason feature)
        #[arg(long)]
        with_conclusions: bool,
    },

    /// Query backlinks to a page
    Backlinks {
        /// Page name (case-insensitive)
        page: String,
        /// Enable fuzzy page name matching
        #[arg(long)]
        fuzzy: bool,
        /// Include N characters of surrounding text
        #[arg(long, default_value = "0")]
        context: usize,
        /// Traverse N hops (1 = direct only)
        #[arg(long, default_value = "1")]
        depth: usize,
        /// Show which conclusions each linked page contributes (requires reason feature)
        #[arg(long)]
        with_conclusions: bool,
    },

    /// Validate: report dead links, orphans, syntax errors, and SPL diagnostics
    Check {
        /// Show only dead links
        #[arg(long)]
        dead_links: bool,
        /// Show only orphan pages
        #[arg(long)]
        orphans: bool,
        /// Show only syntax errors
        #[arg(long)]
        syntax: bool,
        /// Show only SPL diagnostics (parse errors, duplicate labels, undefined references, unreachable literals)
        #[arg(long)]
        spl: bool,
        /// Show only drift diagnostics (SPL blocks with changed grounding since last theory build)
        #[arg(long)]
        drift: bool,
        /// Exit non-zero if issues at level
        #[arg(long, default_value = "error")]
        fail_on: FailLevel,
    },

    /// Find pages with similar names (SimHash)
    Similar {
        /// Search string
        query: String,
        /// Max Hamming distance
        #[arg(long, default_value = "12")]
        threshold: u32,
        /// Max results
        #[arg(long, default_value = "10")]
        limit: usize,
    },

    /// Search vault file contents for text
    Search {
        /// Search string (literal text, or regex with --regex)
        query: String,
        /// Include N characters of surrounding text
        #[arg(long, default_value = "0")]
        context: usize,
        /// Max results to return
        #[arg(long, default_value = "50")]
        limit: usize,
        /// Interpret query as a regular expression
        #[arg(long)]
        regex: bool,
        /// Require exact case match
        #[arg(long)]
        case_sensitive: bool,
        /// Search raw file content (include frontmatter, code blocks, comments)
        #[arg(long)]
        all: bool,
        /// Restrict results to files matching glob (relative to vault root)
        #[arg(long)]
        path: Option<String>,
    },

    /// List all pages in the vault
    List,

    /// Print summary statistics
    Stats {
        /// Number of most-linked pages to show
        #[arg(long, default_value = "10")]
        top: usize,
    },

    /// Find shortest link path between two pages
    Path {
        /// Source page name
        from: String,
        /// Target page name
        to: String,
        /// Maximum path length to search
        #[arg(long, default_value = "10")]
        max_depth: usize,
    },

    /// Export the complete link graph
    Export,

    /// Launch interactive terminal UI
    Tui,

    /// Defeasible reasoning over vault-wide SPL
    #[cfg(feature = "reason")]
    Reason {
        #[command(subcommand)]
        command: ReasonCommand,
    },

    /// Defeasible reasoning over vault-wide SPL (requires --features reason)
    #[cfg(not(feature = "reason"))]
    Reason {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, num_args = 0..)]
        _args: Vec<String>,
    },
}

#[cfg(feature = "reason")]
#[derive(Subcommand)]
pub enum ReasonCommand {
    /// Show current reasoning status: conclusions from vault-wide SPL
    Status {
        /// Show only positive conclusions (+D, +d)
        #[arg(long)]
        positive: bool,
        /// Show only negative conclusions (-D, -d)
        #[arg(long)]
        negative: bool,
        /// Show only definite conclusions (+D, -D)
        #[arg(long)]
        definite: bool,
        /// Show only defeasible conclusions (+d, -d)
        #[arg(long)]
        defeasible: bool,
        /// Filter conclusions by literal name pattern (supports * and ? wildcards)
        #[arg(long)]
        literal: Option<String>,
    },
    /// Explain why a conclusion holds (proof tree with provenance)
    Explain {
        /// Literal to explain (e.g. "flies", "~guilty")
        literal: String,
        /// Maximum proof tree depth
        #[arg(long, default_value = "10")]
        depth: usize,
        /// Output format for the explanation
        #[arg(long, default_value = "json")]
        format: ExplainFormat,
    },
    /// Hypothetical reasoning: what if facts/rules were added?
    WhatIf {
        /// Inline SPL to add hypothetically (e.g. "(given bird)")
        spl: Option<String>,
        /// Read hypothetical SPL from a file instead of inline
        #[arg(long)]
        file: Option<String>,
        /// Focus diff on a specific literal (e.g. "flies", "~guilty")
        #[arg(long)]
        goal: Option<String>,
    },
    /// Why is a literal not provable?
    WhyNot {
        /// Literal to analyze (e.g. "flies", "~guilty")
        literal: String,
    },
    /// What facts are needed to prove a literal? (abductive reasoning)
    Require {
        /// Goal literal to make provable (e.g. "ready-for-production", "~guilty")
        literal: String,
        /// Maximum number of solution sets to return
        #[arg(long, default_value = "5")]
        max_solutions: usize,
        /// Assume these facts already true (inline SPL, e.g. "(given bird)")
        #[arg(long)]
        assume: Option<String>,
    },
    /// Analyze unresolved logical conflicts in the theory
    Conflicts {
        /// Suggest resolutions for each conflict
        #[arg(long)]
        suggest: bool,
        /// Exit non-zero (1) if conflicts are found
        #[arg(long)]
        fail_on_conflicts: bool,
    },
    /// Export the combined theory (SPL with provenance or structured JSON)
    Export {
        /// Output format: spl (reconstructed SPL with provenance comments), json (structured theory)
        #[arg(long, default_value = "json")]
        format: ExportFormat,
        /// Include reasoning results (conclusions) in the export
        #[arg(long)]
        with_conclusions: bool,
    },
    /// Trace full provenance for a conclusion, cross-referenced with the link graph
    Provenance {
        /// Literal to trace (e.g. "flies", "~guilty", "decided-use-redis")
        literal: String,
    },
}

#[cfg(feature = "reason")]
#[derive(Clone, ValueEnum)]
pub enum ExportFormat {
    Json,
    Spl,
}

#[cfg(feature = "reason")]
#[derive(Clone, ValueEnum)]
pub enum ExplainFormat {
    Json,
    Table,
    Natural,
    Dot,
}

#[derive(Clone, ValueEnum)]
pub enum FailLevel {
    Error,
    Warning,
}
