use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(name = "zetl", version, about = "Bi-directional wikilink graph CLI")]
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
    },

    /// Validate: report dead links, orphans, and syntax errors
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
}

#[derive(Clone, ValueEnum)]
pub enum FailLevel {
    Error,
    Warning,
}
