use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(
    name = "zetl",
    version,
    about = "Bi-directional wikilink graph CLI for personal knowledge management",
    after_help = "Examples:\n  zetl list                    List all pages\n  zetl links \"My Page\"         Show forward links\n  zetl search \"query\"          Search vault contents\n  zetl check                   Validate vault health\n  zetl serve                   Start local web server\n\nLearn more: https://github.com/anuna/zetl"
)]
pub struct Cli {
    /// Vault root directory
    #[arg(short = 'd', long, default_value = ".", env = "ZETL_DIR")]
    pub dir: String,

    /// Output format (auto-detects: table for TTY, JSON for pipes)
    #[arg(short = 'f', long, default_value = "auto", env = "ZETL_FORMAT")]
    pub format: OutputFormat,

    /// Force JSON output (shorthand for -f json)
    #[arg(long)]
    pub json: bool,

    /// Force full rescan, ignore cached index
    #[arg(long, env = "ZETL_NO_CACHE")]
    pub no_cache: bool,

    /// Disable colored output
    #[arg(long, env = "NO_COLOR")]
    pub no_color: bool,

    /// Suppress non-essential output
    #[arg(short, long)]
    pub quiet: bool,

    /// Increase verbosity (repeat for more: -vv)
    #[arg(short, long, action = clap::ArgAction::Count)]
    pub verbose: u8,

    /// Query vault state at a historical point in time (requires --features history).
    /// Accepts ISO 8601 dates ("2024-01-15"), relative expressions ("3 days ago",
    /// "last monday"), or VCS refs ("HEAD~1", change-ID prefix).
    #[cfg(feature = "history")]
    #[arg(long, value_name = "TIME-EXPR")]
    pub at: Option<String>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Clone, ValueEnum, PartialEq)]
pub enum OutputFormat {
    Json,
    Table,
    Auto,
}

#[derive(Subcommand)]
pub enum Command {
    /// Build or refresh the link index
    Index,

    /// Query forward links from a page
    #[command(after_help = "Examples:\n  zetl links \"My Page\"              Direct links\n  zetl links \"My Page\" --depth 2    Two hops deep")]
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
    #[command(after_help = "Examples:\n  zetl check                   Full vault health check\n  zetl check --dead-links      Show only broken links\n  zetl check --orphans          Show only unlinked pages")]
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
        /// Theme name for hook discovery (looks in .zetl/themes/<name>/hooks/)
        #[arg(long, default_value = "default")]
        theme: String,
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
    #[command(after_help = "Examples:\n  zetl search \"wikilink\"                 Basic search\n  zetl search \"API\" --near \"Backend\"     Search near a page\n  zetl search \"TODO\" --case-sensitive    Exact case match")]
    Search {
        /// Search string
        query: String,
        /// Include N characters of surrounding text
        #[arg(long, default_value = "40")]
        context: usize,
        /// Max results to return
        #[arg(long, default_value = "50")]
        limit: usize,
        /// Require exact case match
        #[arg(long)]
        case_sensitive: bool,
        /// Restrict results to files matching glob (relative to vault root)
        #[arg(long)]
        path: Option<String>,
        /// Restrict results to pages within --depth hops of PAGE
        #[arg(long, value_name = "PAGE")]
        near: Option<String>,
        /// Neighbourhood radius (default: 1, must be >= 1; only valid with --near)
        #[arg(long, value_name = "N")]
        depth: Option<usize>,
        /// Pure vector (semantic) search ranked by cosine similarity (requires --features semantic)
        #[arg(long)]
        semantic: bool,
        /// Hybrid BM25 + vector search via reciprocal rank fusion (requires --features semantic)
        #[arg(long)]
        hybrid: bool,
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
    #[command(after_help = "Examples:\n  zetl path \"Page A\" \"Page B\"           Find shortest path\n  zetl path \"Page A\" \"Page B\" --max-depth 5")]
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

    /// List Merkle blocks for a page (forward mode) or resolve a block by hash
    Blocks {
        /// Page name (case-insensitive). Omit when using --resolve.
        page: Option<String>,
        /// Filter by block type
        #[arg(long = "type", value_name = "TYPE", default_value = "all")]
        block_type: BlockTypeFilter,
        /// Look up a block by its BLAKE3 hash (hex prefix or full). Mutually exclusive with page.
        #[arg(long, value_name = "HASH")]
        resolve: Option<String>,
    },

    /// Launch interactive terminal UI
    Tui,

    /// Start local web server to browse the vault
    Serve {
        /// Port to listen on
        #[arg(short, long, default_value = "3000")]
        port: u16,
        /// Theme name (looks in .zetl/themes/<name>/)
        #[arg(long, default_value = "default")]
        theme: String,
        /// Enable multi-user collaborative editing mode
        #[arg(long)]
        collab: bool,
        /// Bootstrap the vault owner (first-time setup, requires --collab)
        #[arg(long, requires = "collab")]
        init_owner: bool,
        /// Display name for the vault owner (used with --init-owner)
        #[arg(long, default_value = "Owner", requires = "init_owner")]
        owner_name: String,
        /// Git HEAD poll interval for detecting external commits (e.g. "30s", "1m").
        /// Set to "0" to disable. Requires --collab.
        #[arg(long, default_value = "30s", requires = "collab", value_parser = parse_duration)]
        git_poll_interval: std::time::Duration,
    },

    /// Generate an invitation token for a new collaborator
    #[command(after_help = "Examples:\n  zetl invite --as alice --role editor\n  zetl invite --as alice --role reader --pages \"projects/*\"\n  zetl invite --as alice --role editor --expires 24h")]
    Invite {
        /// Your username (inviter)
        #[arg(long = "as")]
        as_user: String,
        /// Role for the invitee (reader, editor, admin)
        #[arg(long)]
        role: String,
        /// Optional page scope glob pattern
        #[arg(long)]
        pages: Option<String>,
        /// Expiry duration (e.g. "72h", "24h", "7d"; default: 72h)
        #[arg(long)]
        expires: Option<String>,
        /// Port the server is running on (for URL generation)
        #[arg(long, default_value = "3000")]
        port: u16,
        /// Host for the invitation URL
        #[arg(long, default_value = "localhost")]
        host: String,
    },

    /// Derive an agent token from a BIP39 mnemonic for headless API authentication
    #[command(after_help = "Examples:\n  zetl agent-token --mnemonic \"word1 word2 ... word12\"")]
    AgentToken {
        /// BIP39 mnemonic phrase (12 words)
        #[arg(long)]
        mnemonic: String,
    },

    /// Generate a static HTML site from the vault
    Build {
        /// Output directory
        #[arg(short, long, default_value = "dist")]
        out_dir: String,
        /// Theme name (looks in .zetl/themes/<name>/)
        #[arg(long, default_value = "default")]
        theme: String,
    },

    /// Launch the Xanadu-style two-pane view for a note
    View {
        /// Page title to open (launches a page picker when omitted)
        page: Option<String>,
        /// Lines shown per context card in non-focused mode (1–20)
        #[arg(long, default_value = "5", value_parser = clap::value_parser!(u8).range(1..=20), value_name = "N")]
        context_lines: u8,
        /// Percentage of terminal columns allocated to the main pane (30–80)
        #[arg(long, default_value = "58", value_parser = clap::value_parser!(u8).range(30..=80), value_name = "pct")]
        main_width: u8,
    },

    /// Theme management
    Theme {
        #[command(subcommand)]
        command: ThemeCommand,
    },

    /// Hook management
    Hook {
        #[command(subcommand)]
        command: HookCommand,
    },

    /// Agent lifecycle integration
    Agent {
        #[command(subcommand)]
        command: AgentCommand,
    },

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

    /// Compute graph-level diff against a git ref or historical snapshot
    Diff {
        /// Git ref or jj change-ID / time expression to use as the diff baseline
        #[arg(long, value_name = "REF")]
        from: Option<String>,

        /// Baseline date expression (ISO 8601 or natural language; alias for --from)
        #[arg(long, value_name = "DATE")]
        since: Option<String>,

        /// Filter output to one change category
        #[arg(long, value_name = "CATEGORY")]
        filter: Option<DiffFilter>,
    },

    /// Watch vault for file changes and emit NDJSON graph events
    Watch {
        /// Debounce window in milliseconds (default: 150; min 10, max 5000)
        #[arg(long, default_value = "150", value_parser = clap::value_parser!(u64).range(10..=5000))]
        debounce: u64,
        /// Shell command invoked once per event with event JSON on stdin
        #[arg(long)]
        exec: Option<String>,
    },

    /// Browse vault history timeline (requires --features history)
    ///
    /// Shows the list of temporal snapshots and allows querying graph evolution.
    #[cfg(feature = "history")]
    History {
        #[command(subcommand)]
        command: HistoryCommand,
    },
}

#[derive(Subcommand)]
pub enum ThemeCommand {
    /// List available themes (bundled + installed)
    List,
    /// Install a theme from a git repository
    Install {
        /// Theme source (user/repo, URL, or git@... with optional #ref)
        source: String,
        /// Subdirectory within the repository to use as theme root
        #[arg(long)]
        path: Option<String>,
        /// Override the installed theme directory name
        #[arg(long)]
        name: Option<String>,
        /// Overwrite existing theme directory
        #[arg(long)]
        force: bool,
    },
    /// Remove an installed theme
    Remove {
        /// Theme name to remove
        name: String,
    },
    /// Export a bundled theme to .zetl/themes/ for customisation
    Export {
        /// Bundled theme name to export
        name: String,
        /// Overwrite existing theme directory
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand)]
pub enum HookCommand {
    /// List all active hooks for the current vault and theme
    List {
        /// Theme name (looks in .zetl/themes/<name>/hooks/)
        #[arg(long, default_value = "default")]
        theme: String,
    },
    /// Run a named hook with real vault context
    Run {
        /// Hook lifecycle name (e.g. post-build, pre-build)
        name: String,
        /// Theme name (looks in .zetl/themes/<name>/hooks/)
        #[arg(long, default_value = "default")]
        theme: String,
        /// Extra JSON fields merged into the context (after --)
        #[arg(last = true)]
        extra: Vec<String>,
    },
}

#[derive(Subcommand)]
pub enum AgentCommand {
    /// Run an on-agent hook with task context (REQ-020-023)
    #[command(after_help = "Examples:\n  zetl agent run link-checker\n  zetl agent run summariser --pages \"Note A\" \"Note B\" --budget 4000")]
    Run {
        /// Agent task name
        name: String,
        /// Theme name (looks in .zetl/themes/<name>/hooks/)
        #[arg(long, default_value = "default")]
        theme: String,
        /// Target pages for the agent (empty = vault-wide)
        #[arg(long = "pages", num_args = 0..)]
        target_pages: Vec<String>,
        /// Token budget for the agent action (0 = unlimited)
        #[arg(long, default_value = "0")]
        budget: u32,
        /// Extra JSON fields merged into the context (after --)
        #[arg(last = true)]
        extra: Vec<String>,
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

/// Block type filter for the `blocks` command.
#[derive(Clone, ValueEnum)]
pub enum BlockTypeFilter {
    Heading,
    Paragraph,
    Spl,
    Code,
    Table,
    List,
    Blockquote,
    Frontmatter,
    All,
}

/// Category filter for `zetl diff`.
#[derive(Clone, ValueEnum)]
pub enum DiffFilter {
    Pages,
    Links,
    Orphans,
    #[value(name = "dead-links")]
    DeadLinks,
}

/// Subcommands for `zetl history` (requires --features history).
#[cfg(feature = "history")]
#[derive(Subcommand)]
pub enum HistoryCommand {
    /// List recent snapshots with timestamps and brief graph stats
    Timeline {
        /// Maximum number of snapshots to show (most recent first)
        #[arg(long, default_value = "20")]
        limit: usize,
    },
    /// Show the evolution of a specific page across snapshots
    Page {
        /// Page name (case-insensitive)
        name: String,
        /// Maximum number of snapshots to show
        #[arg(long, default_value = "20")]
        limit: usize,
    },
    /// Reverse-chronological timeline of graph-level deltas.
    ///
    /// Each row shows what changed between consecutive snapshots: pages added or
    /// removed, and net link-count deltas. Identical vault states (same
    /// vault_root_hash) are collapsed into a single entry.
    Log {
        /// Show only snapshots since this time expression (ISO 8601, relative
        /// natural language, or VCS ref). E.g. "2024-01-15", "3 days ago",
        /// "last monday", "HEAD~5".
        #[arg(long, value_name = "TIME-EXPR")]
        since: Option<String>,
        /// Maximum number of entries to show (most recent first)
        #[arg(long, default_value = "20")]
        limit: usize,
    },
}

/// Parse a human-friendly duration string like "30s", "5m", "1h", or "0" (disabled).
fn parse_duration(s: &str) -> Result<std::time::Duration, String> {
    let s = s.trim();
    if s == "0" {
        return Ok(std::time::Duration::ZERO);
    }
    let (num_str, multiplier) = if let Some(n) = s.strip_suffix('s') {
        (n, 1u64)
    } else if let Some(n) = s.strip_suffix('m') {
        (n, 60)
    } else if let Some(n) = s.strip_suffix('h') {
        (n, 3600)
    } else {
        // Assume seconds if no suffix.
        (s, 1)
    };
    let value: u64 = num_str
        .trim()
        .parse()
        .map_err(|_| format!("invalid duration: {s}"))?;
    Ok(std::time::Duration::from_secs(value * multiplier))
}
