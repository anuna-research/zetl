use clap::{Args, Parser, Subcommand, ValueEnum};

/// Vault scanning exclusion flags shared by `build`, `index`, `serve`,
/// `search`, and `watch`. See SPEC-026 for precedence semantics.
#[derive(Args, Debug, Clone, Default)]
pub struct ScanArgs {
    /// Gitignore-syntax pattern to exclude from the vault scan. Repeatable.
    /// Combines with `.ztlignore` and the default dotdir exclusion.
    #[arg(long = "exclude", value_name = "PATTERN", action = clap::ArgAction::Append)]
    pub exclude: Vec<String>,

    /// Disable the default exclusion of dotdirs (`.claude/`, `.obsidian/`, etc.).
    /// `.git/`, `.ztl/`, and `node_modules/` are still excluded.
    #[arg(long)]
    pub include_hidden: bool,
}

impl ScanArgs {
    /// Convert to scanner-facing options, propagating the global verbose flag.
    pub fn to_scan_options(&self, verbose: bool) -> crate::scanner::ScanOptions {
        crate::scanner::ScanOptions::default()
            .with_exclude_patterns(self.exclude.clone())
            .with_include_hidden(self.include_hidden)
            .with_verbose(verbose)
    }
}

impl Cli {
    /// Resolve the effective `ScanOptions` for this invocation.
    ///
    /// Commands that expose `--exclude` / `--include-hidden` use those
    /// values; all other commands fall back to defaults. The global
    /// `--verbose` flag is propagated in both cases (REQ-205, OBS-200).
    pub fn scan_options(&self) -> crate::scanner::ScanOptions {
        let verbose = self.verbose > 0;
        match &self.command {
            Command::Index { scan }
            | Command::Build { scan, .. }
            | Command::Serve { scan, .. }
            | Command::Search { scan, .. }
            | Command::Watch { scan, .. } => scan.to_scan_options(verbose),
            // SCAN-OPTS: intentional — commands without ScanArgs use defaults.
            _ => crate::scanner::ScanOptions::default().with_verbose(verbose),
        }
    }
}

#[derive(Parser)]
#[command(
    name = "ztl",
    version,
    about = "Bi-directional wikilink graph CLI for knowledge management, solo or team",
    after_help = "Examples:\n  ztl list                    List all pages\n  ztl links \"My Page\"         Show forward links\n  ztl search \"query\"          Search vault contents\n  ztl check                   Validate vault health\n  ztl serve                   Start local web server\n  ztl serve --collab          Start with multi-user auth\n\nLearn more: https://codeberg.org/anuna/ztl"
)]
pub struct Cli {
    /// Vault root directory
    #[arg(
        short = 'd',
        long,
        default_value = ".",
        env = "ztl_DIR",
        global = true
    )]
    pub dir: String,

    /// Output format (auto-detects: table for TTY, JSON for pipes)
    #[arg(
        short = 'f',
        long,
        default_value = "auto",
        env = "ztl_FORMAT",
        global = true
    )]
    pub format: OutputFormat,

    /// Force JSON output (shorthand for -f json)
    #[arg(long, global = true)]
    pub json: bool,

    /// Force full rescan, ignore cached index
    #[arg(long, env = "ztl_NO_CACHE", global = true)]
    pub no_cache: bool,

    /// Disable colored output
    #[arg(long, env = "NO_COLOR", global = true)]
    pub no_color: bool,

    /// Suppress non-essential output
    #[arg(short, long, global = true)]
    pub quiet: bool,

    /// Increase verbosity (repeat for more: -vv)
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    pub verbose: u8,

    /// Disable interactive prompts; fail non-zero if input would be needed
    #[arg(long, global = true)]
    pub no_input: bool,

    /// Query vault state at a historical point in time (requires --features history).
    /// Accepts ISO 8601 dates ("2024-01-15"), relative expressions ("3 days ago",
    /// "last monday"), or VCS refs ("HEAD~1", change-ID prefix).
    #[cfg(feature = "history")]
    #[arg(long, value_name = "TIME-EXPR", global = true)]
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
    Index {
        #[command(flatten)]
        scan: ScanArgs,
    },

    /// Query forward links from a page
    #[command(
        after_help = "Examples:\n  ztl links \"My Page\"              Direct links\n  ztl links \"My Page\" --depth 2    Two hops deep"
    )]
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
    #[command(
        after_help = "Examples:\n  ztl check                   Full vault health check\n  ztl check --dead-links      Show only broken links\n  ztl check --orphans          Show only unlinked pages"
    )]
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
        /// Theme name for hook discovery (looks in .ztl/themes/<name>/hooks/)
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
    #[command(
        after_help = "Examples:\n  ztl search \"wikilink\"                 Basic search\n  ztl search \"API\" --near \"Backend\"     Search near a page\n  ztl search \"TODO\" --case-sensitive    Exact case match"
    )]
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
        #[command(flatten)]
        scan: ScanArgs,
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
    #[command(
        after_help = "Examples:\n  ztl path \"Page A\" \"Page B\"           Find shortest path\n  ztl path \"Page A\" \"Page B\" --max-depth 5"
    )]
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

    /// Start local web server to browse the vault
    #[command(
        after_help = "Examples:\n  ztl serve                                        Single-user web UI\n  ztl serve --collab --init-owner --owner-name Jo   First-time collab setup\n  ztl serve --collab                                Multi-user mode\n  ztl serve --port 8080 --theme dark                Custom port and theme"
    )]
    Serve {
        /// Port to listen on
        #[arg(short, long, default_value = "3000")]
        port: u16,
        /// Theme name (looks in .ztl/themes/<name>/)
        #[arg(long, default_value = "default")]
        theme: String,
        /// Public directory whose files override generated pages (served first)
        #[arg(long)]
        public: Option<String>,
        /// Enable multi-user collaborative editing mode
        #[arg(long)]
        collab: bool,
        /// Bootstrap the vault owner (first-time setup, requires --collab)
        #[arg(long, requires = "collab")]
        init_owner: bool,
        /// Display name for the vault owner (used with --init-owner)
        #[arg(long, default_value = "Owner", requires = "init_owner")]
        owner_name: String,
        /// Public hostname for WebAuthn relying party (e.g. "mysite.fly.dev").
        /// Defaults to "localhost". Can also be set via ztl_HOSTNAME env var.
        /// Requires --collab.
        #[arg(long, requires = "collab", env = "ztl_HOSTNAME")]
        hostname: Option<String>,
        /// BIP39 mnemonic (12 words) to deterministically derive the server key.
        /// Useful for containerised deployments where the filesystem is ephemeral.
        /// Can also be set via ztl_SERVER_KEY_SEED env var. Requires --collab.
        #[arg(long, requires = "collab", env = "ztl_SERVER_KEY_SEED")]
        server_key_seed: Option<String>,
        /// Git HEAD poll interval for detecting external commits (e.g. "30s", "1m").
        /// Set to "0" to disable. Requires --collab.
        #[arg(long, default_value = "30s", requires = "collab", value_parser = parse_duration)]
        git_poll_interval: std::time::Duration,
        /// Skip every hook except theme hooks declared in `theme.toml`'s
        /// `[[theme.hooks]]` array (SPEC-032 REQ-3223). Intended as an
        /// audit / hostile-theme-inspection surface.
        #[arg(long)]
        safe_mode: bool,
        #[command(flatten)]
        scan: ScanArgs,
    },

    /// Generate an invitation token for a new collaborator
    #[command(
        after_help = "Examples:\n  ztl invite --as alice --role editor\n  ztl invite --as alice --role reader --pages \"projects/*\"\n  ztl invite --as alice --role editor --expires 24h"
    )]
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
    #[command(after_help = "Examples:\n  ztl agent-token --mnemonic \"word1 word2 ... word12\"")]
    AgentToken {
        /// BIP39 mnemonic phrase (12 words)
        #[arg(long)]
        mnemonic: String,
    },

    /// Derive an SSH ed25519 key from a BIP39 mnemonic and write it to a file.
    /// Useful for containerised deployments where a single seed phrase provides
    /// both the server key and the git SSH key.
    #[command(
        after_help = "Examples:\n  ztl derive-ssh-key --mnemonic \"word1 word2 ... word12\" --out /root/.ssh/id_ed25519"
    )]
    DeriveSshKey {
        /// BIP39 mnemonic phrase (12 words)
        #[arg(long, env = "ztl_SERVER_KEY_SEED")]
        mnemonic: String,
        /// Output path for the private key (default: stdout)
        #[arg(long)]
        out: Option<String>,
    },

    /// Generate a static HTML site from the vault
    Build {
        /// Output directory
        #[arg(short = 'o', long, alias = "out", default_value = "dist")]
        out_dir: String,
        /// Theme name (looks in .ztl/themes/<name>/)
        #[arg(long, default_value = "default")]
        theme: String,
        /// Public directory whose contents are copied over the output root (overrides generated pages)
        #[arg(long)]
        public: Option<String>,
        /// Canonical site URL (e.g. `https://vault.example`). Used to emit
        /// absolute og:image / twitter:image URLs so social-card scrapers
        /// can resolve them. Without this flag, image URLs are root-relative
        /// (`/slug/og.png`) which only works when the vault is served from
        /// the domain root.
        #[arg(long)]
        site_url: Option<String>,
        /// Skip every hook except theme hooks declared in `theme.toml`'s
        /// `[[theme.hooks]]` array (SPEC-032 REQ-3223). Vault hooks and
        /// undeclared theme hooks are skipped with a stderr line per skip.
        #[arg(long)]
        safe_mode: bool,
        /// Make mixed-parser configurations fatal (SPEC-033 REQ-3315).
        /// By default, pages whose resolved parser doesn't match a
        /// matched ecosystem hook's expected parser produce a warning;
        /// under `--strict-parsers` the warning is promoted to an error.
        #[arg(long)]
        strict_parsers: bool,
        #[command(flatten)]
        scan: ScanArgs,
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

    /// Plugin-ecosystem introspection (Pandoc filters, mdBook preprocessors, remark plugins)
    #[command(
        after_help = "Examples:\n  ztl ecosystem check               Report runtimes + configured hooks\n  ztl ecosystem check --json        Machine-readable report for CI pre-flight"
    )]
    Ecosystem {
        #[command(subcommand)]
        command: EcosystemCommand,
    },

    /// Inspect the ztl-ext AST for a page or diff two AST documents
    #[command(
        after_help = "Examples:\n  ztl ast sample notes/page.md                  Print canonical AST JSON\n  ztl ast sample notes/page.md --stage pre-parse  Print raw markdown input\n  ztl ast diff before.json after.json           Tree diff of two AST files"
    )]
    Ast {
        #[command(subcommand)]
        command: AstCommand,
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
        #[command(flatten)]
        scan: ScanArgs,
    },

    /// Browse vault history timeline (requires --features history)
    ///
    /// Shows the list of temporal snapshots and allows querying graph evolution.
    #[cfg(feature = "history")]
    History {
        #[command(subcommand)]
        command: HistoryCommand,
    },

    /// Issue a delegate JWT for MCP access
    #[cfg(feature = "mcp")]
    Delegate {
        /// Restrict to specific tools (comma-separated, e.g. "search,get,links")
        #[arg(long)]
        tools: Option<String>,
        /// Restrict to page scope glob (e.g. "projects/**")
        #[arg(long)]
        scope: Option<String>,
        /// Token expiry (e.g. "1h", "7d", "30d"). Default: no expiry
        #[arg(long)]
        expiry: Option<String>,
        /// BIP39 mnemonic (fallback if identity key not stored)
        #[arg(long)]
        mnemonic: Option<String>,
        /// Save derived key to ~/.config/ztl/identity.key
        #[arg(long)]
        save_key: bool,
    },

    /// Issue a delegate JWT (requires --features mcp)
    #[cfg(not(feature = "mcp"))]
    Delegate {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, num_args = 0..)]
        _args: Vec<String>,
    },

    /// Start an MCP server exposing ztl tools
    #[cfg(feature = "mcp")]
    Mcp {
        /// Transport mode
        #[arg(long, default_value = "stdio")]
        transport: McpTransport,
        /// Host to bind HTTP server to
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        /// Port for HTTP transport
        #[arg(long, default_value_t = 3100)]
        port: u16,
        /// Allow non-loopback bind without auth
        #[arg(long)]
        insecure: bool,
        /// Restrict which user DIDs may issue tokens (repeatable)
        #[arg(long)]
        allowed_issuer: Vec<String>,
        /// CORS origin for HTTP transport
        #[arg(long)]
        cors_origin: Option<String>,
    },

    /// Start an MCP server (requires --features mcp)
    #[cfg(not(feature = "mcp"))]
    Mcp {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, num_args = 0..)]
        _args: Vec<String>,
    },

    /// Generate shell completion script (bash, zsh, fish, elvish, powershell)
    Completions {
        /// Target shell
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },

    /// Generate a roff(7) man page on stdout (pipe to `man -l -` to preview)
    Man,

    /// Capability-URL static-site mode operations
    Cap {
        #[command(subcommand)]
        command: CapCommand,
    },
}

/// Subcommands for `ztl cap` (SPEC-034 REQ-3416 capability-URL
/// static-distribution mode).
///
/// The CLI surface is the full set of verbs called out in REQ-3416;
/// verbs whose implementation has not landed exit non-zero with a
/// `not-yet-implemented` diagnostic so operators discover the surface
/// without hitting a half-implemented workflow.
#[derive(Subcommand)]
pub enum CapCommand {
    /// Generate both the content-encryption secret and the vault-signing
    /// keypair. Emits to stdout exactly once — see SPEC-034 REQ-3419.
    #[command(
        after_help = "Examples:\n  ztl cap genkey                             Print ztl_CAP_SECRET + signing key\n\nSee SPEC-034 REQ-3419 / ADR-3405 for storage guidance."
    )]
    Genkey,

    /// Issue an invitation grant for a new reader.
    #[command(
        after_help = "Examples:\n  ztl cap invite alice --cohort eng --site-url https://wiki.example\n  ztl cap invite bob --cohort ops --expires 7d --pages 'projects/*' --site-url https://wiki.example\n  ztl cap invite carol --cohort eng --recipient age-recipient-v1:...   # hardened mode\n  ztl cap invite dan --cohort eng --via enrol-page --site-url https://wiki.example\n\nSee SPEC-034 REQ-3416 / REQ-3410."
    )]
    Invite {
        /// Human-readable invitee label (stored in grants.toml).
        name: String,
        /// Cohort id the grant belongs to.
        #[arg(long, value_name = "ID")]
        cohort: String,
        /// Expiry duration (e.g. "72h", "7d").
        #[arg(long, value_name = "DUR")]
        expires: Option<String>,
        /// Page scope glob restricting what this grant decrypts.
        #[arg(long, value_name = "GLOB")]
        pages: Option<String>,
        /// Pre-collected recipient pubkey (hardened mode — skip delegated
        /// URL generation). Must carry the `age-recipient-v1:` prefix.
        #[arg(long, value_name = "PUBKEY")]
        recipient: Option<String>,
        /// Print an `enrol.html` URL instead of generating the grant
        /// locally (hardened mode, REQ-3404). Only valid value today is
        /// `enrol-page`.
        #[arg(long = "via", value_name = "MODE")]
        via: Option<String>,
        /// Split the private key across URL + second factor (REQ-3430).
        #[arg(long)]
        split_key: bool,
        /// Canonical site URL (scheme + host) used to render the invite
        /// URL. Falls back to the `ztl_CAP_SITE_URL` env var.
        #[arg(long, value_name = "URL", env = "ztl_CAP_SITE_URL")]
        site_url: Option<String>,
        /// Landing slug the invite URL points at (the reader's first
        /// page). Defaults to `welcome`; operators rename their landing
        /// page by passing a different slug here.
        #[arg(long, value_name = "SLUG", default_value = "welcome")]
        slug: String,
    },

    /// List all issued grants.
    #[command(
        after_help = "Examples:\n  ztl cap list\n  ztl cap list --cohort eng\n  ztl cap list --output json"
    )]
    List {
        /// Restrict output to one cohort.
        #[arg(long, value_name = "ID")]
        cohort: Option<String>,
        /// Output format for this verb (overrides the global --format).
        #[arg(long, value_enum, default_value_t = OutputFormat::Auto)]
        output: OutputFormat,
    },

    /// Revoke an issued grant by id.
    Revoke {
        /// Grant id to revoke (see `ztl cap list`).
        grant_id: String,
    },

    /// Rotate a cohort's content-key salt (URLs remain stable per
    /// REQ-3402).
    Rotate {
        /// Cohort id to rotate.
        #[arg(long, value_name = "ID")]
        cohort: String,
    },

    /// Mark a grant as operator-confirmed-onboarded
    /// (`bound=true`; REQ-3416).
    Finalise {
        /// Grant id to finalise.
        grant_id: String,
        /// Reissue the delegated private key at finalisation time.
        #[arg(long)]
        rotate_grant: bool,
    },

    /// Share: not part of the SPEC-034 verb set today; reserved for a
    /// future workflow that bundles an invite URL with auxiliary metadata
    /// for out-of-band delivery.
    Share {
        /// Grant id whose invite artefacts should be re-emitted.
        grant_id: String,
    },

    /// Stale-grant and public-safety audit (REQ-3423 / REQ-3416).
    ///
    /// Exits non-zero if any grant has expired since the last build or
    /// if a configured public cohort is missing the required guardrails.
    Check {
        /// Run the public-safety audit only.
        #[arg(long)]
        public_safety: bool,
    },

    /// Mark past-expiry grants as revoked in-place.
    Sweep,

    /// SPAKE2 pubkey handoff between two operators (REQ-3408).
    ///
    /// The pairing runs in a single process per side so the SPAKE2
    /// session state (which the upstream crate does not serialise) can
    /// stay resident across the three message exchanges. Two roles:
    ///
    ///   * **Grantor** (`--grantor`, the default): generate a fresh
    ///     4-word BIP39 phrase, refuse reuse within 30 days (tracked
    ///     in `.ztl/caps/.pair-nonces`), start SPAKE2, print phrase +
    ///     outbound handshake, then BLOCK reading three lines from
    ///     stdin — peer handshake, peer pubkey (base64), peer HMAC
    ///     tag (base64). Verifies the HMAC and prints the authenticated
    ///     pubkey on success (exit 1 on mismatch).
    ///
    ///   * **Grantee** (`--grantee`): one-shot. Takes the grantor's
    ///     handshake + the phrase + the local pubkey, starts its own
    ///     SPAKE2 session, derives the shared key, and prints its
    ///     outbound handshake + HMAC tag of the pubkey. The operator
    ///     relays those two strings back to the grantor.
    ///
    /// The handoff is authenticated by the phrase: if the phrase
    /// differs on the two sides, SPAKE2 derives different keys and the
    /// HMAC verification fails.
    #[command(
        after_help = "Examples:\n  ztl cap pair --grantor\n                                              Start a new pairing (interactive — reads stdin)\n  ztl cap pair --grantee --peer <handshake> --phrase <4-words> --pubkey <b64>\n                                              Complete the pairing as the grantee\n\nSee SPEC-034 REQ-3408 (CLI) / CON-3407."
    )]
    Pair {
        /// Run the grantor side (default if neither flag is set).
        /// Blocks on stdin after printing phrase + handshake.
        #[arg(long, conflicts_with = "grantee")]
        grantor: bool,
        /// Run the grantee side (one-shot). Requires `--peer`,
        /// `--phrase`, `--pubkey`.
        #[arg(long, conflicts_with = "grantor")]
        grantee: bool,

        /// Peer's base64 SPAKE2 outbound message (grantor's handshake,
        /// from stdout of `ztl cap pair --grantor`). Required by
        /// `--grantee`.
        #[arg(long, value_name = "HANDSHAKE")]
        peer: Option<String>,
        /// 4-word BIP39 pairing phrase. Required by `--grantee`;
        /// optional for `--grantor` (test hook — seeds reuse a pinned
        /// phrase rather than drawing from the RNG).
        #[arg(long, value_name = "PHRASE", env = "ztl_CAP_PAIR_PHRASE")]
        phrase: Option<String>,
        /// 32-byte raw pubkey as base64 (standard, no padding).
        /// Required by `--grantee` — the pubkey being authenticated
        /// into the grantor's cohort.
        #[arg(long, value_name = "PUBKEY_B64")]
        pubkey: Option<String>,
    },

    /// Scan a vault diff for malicious-content patterns
    #[command(
        after_help = "Examples:\n  ztl cap audit-diff main HEAD                Scan changes since main\n  ztl cap audit-diff --corpus tools/audit-diff-corpus/fixtures/001-*\n                                              Run against a single corpus fixture\n  ztl cap audit-diff --corpus-root tools/audit-diff-corpus\n                                              Walk every fixture under the corpus root"
    )]
    AuditDiff {
        /// Git ref for the baseline vault state
        #[arg(value_name = "OLD_REF")]
        old_ref: Option<String>,
        /// Git ref for the new vault state (defaults to HEAD)
        #[arg(value_name = "NEW_REF")]
        new_ref: Option<String>,
        /// Scan a single corpus fixture directory (must contain `new/`;
        /// `baseline/` is optional). Mutually exclusive with git refs.
        #[arg(long, value_name = "DIR", conflicts_with_all = ["old_ref", "new_ref", "corpus_root"])]
        corpus: Option<std::path::PathBuf>,
        /// Walk every fixture directory under a corpus root (per
        /// REQ-3424's CI `audit-corpus` job). Fails on any fixture
        /// whose expected findings are missed.
        #[arg(long, value_name = "DIR", conflicts_with_all = ["old_ref", "new_ref", "corpus"])]
        corpus_root: Option<std::path::PathBuf>,
    },

    /// Rotate the Ed25519 vault-signing key. Requires rebuilding every
    /// page so old signatures are re-issued under the new key.
    RotateSigningKey,

    /// Print the operator checklist for removing the wiki from service
    /// (REQ-3431). Does not modify any files.
    EmergencyShutdown,
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
    /// Export a bundled theme to .ztl/themes/ for customisation
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
        /// Theme name (looks in .ztl/themes/<name>/hooks/)
        #[arg(long, default_value = "default")]
        theme: String,
    },
    /// Run a named hook with real vault context
    Run {
        /// Hook lifecycle name (e.g. post-build, pre-build)
        name: String,
        /// Theme name (looks in .ztl/themes/<name>/hooks/)
        #[arg(long, default_value = "default")]
        theme: String,
        /// Extra JSON fields merged into the context (after --)
        #[arg(last = true)]
        extra: Vec<String>,
    },
    /// Scaffold a new render-pipeline hook (skeleton + manifest + fixture).
    ///
    /// Writes the persistent-mode skeleton at
    /// `.ztl/hooks/<stage>.d/<name>.<ext>` and the sidecar manifest at
    /// `.ztl/hooks/<stage>.d/<name>.<ext>.toml` (the canonical form
    /// composition reads — do not rename to `<name>.toml`).
    /// SPEC-032 REQ-3225.
    #[command(
        after_help = "Examples:\n  ztl hook new transform callouts\n  ztl hook new pre-parse prelude --lang sh\n  ztl hook new post-render banner --lang js\n  ztl hook new transform smallcaps --ecosystem pandoc"
    )]
    New {
        /// Hook pipeline stage (pre-parse, transform, or post-render).
        stage: AuthoringStage,
        /// Extension id (also used as filename stem and template-var namespace).
        name: String,
        /// Implementation language for the scaffolded skeleton.
        #[arg(long, value_enum, default_value_t = HookLang::Py)]
        lang: HookLang,
        /// Scaffold against an ecosystem plugin adapter. Seeds the SPEC-033
        /// REQ-3312 manifest fields the chosen ecosystem requires (so
        /// `dry-run`/`build` work without further hand-edits) and, for
        /// `pandoc`, drops a starter identity Lua filter on disk at the
        /// `lua_filter` path.
        #[arg(long, value_enum)]
        ecosystem: Option<HookEcosystem>,
        /// Overwrite existing scaffold files.
        #[arg(long)]
        force: bool,
    },
    /// Run a render-pipeline hook against its test fixture and diff the
    /// output against the stored golden. Non-zero exit on mismatch.
    /// SPEC-032 REQ-3225 / TEST-3225.
    Test {
        /// Hook extension id to test. Looks for the hook under
        /// `.ztl/hooks/<stage>.d/<name>.*` and its fixture at
        /// `tests/hook-fixtures/<name>/`.
        name: String,
        /// Regenerate the golden from the hook's current output instead
        /// of diffing (equivalent to `cargo insta accept`).
        #[arg(long)]
        update: bool,
    },
    /// Capture a vault page's pre- and post-hook forms into the hook's
    /// fixture directory. SPEC-032 REQ-3225.
    Fixture {
        /// Vault-relative page path (e.g. `projects/q2.md`; the `.md`
        /// suffix is optional).
        #[arg(long, value_name = "PAGE")]
        from: String,
        /// Hook extension id whose fixture is populated.
        #[arg(long, value_name = "NAME")]
        hook: String,
    },
    /// Watch a hook's source file and stream its stderr; restarts the
    /// persistent-mode subprocess when the source changes.
    /// SPEC-032 REQ-3225.
    Watch {
        /// Hook extension id to watch.
        name: String,
    },
    /// Report per-hook coverage (matched pages, invocations, failures,
    /// latency) for the most-recent build, or a fresh dry-run against the
    /// current vault if no build coverage has been persisted.
    /// SPEC-032 REQ-3208 / CON-3208.
    #[command(
        after_help = "Examples:\n  ztl hook coverage\n  ztl hook coverage --json\n  ztl hook coverage --stage transform"
    )]
    Coverage {
        /// Theme name (looks in .ztl/themes/<name>/hooks/).
        #[arg(long, default_value = "default")]
        theme: String,
        /// Restrict the report to a single stage.
        #[arg(long, value_enum)]
        stage: Option<AuthoringStage>,
    },
    /// Evaluate a hook's selector against the vault and print the matched
    /// pages. The hook is NOT invoked. Exits 0 if any pages match; 1 if
    /// zero match. SPEC-032 REQ-3209.
    #[command(
        name = "dry-run",
        after_help = "Examples:\n  ztl hook dry-run transform/callouts\n  ztl hook dry-run pre-parse/prelude --limit 100\n  ztl -d demo-vault hook dry-run transform/callouts"
    )]
    DryRun {
        /// Hook selector in the form `<stage>/<name>`. `<stage>` is one of
        /// `pre-parse`, `transform`, or `post-render`; `<name>` matches the
        /// hook's extension_id (filename stem minus any leading `\d+-`
        /// ordering prefix).
        spec: String,
        /// Theme name (looks in .ztl/themes/<name>/hooks/).
        #[arg(long, default_value = "default")]
        theme: String,
        /// Maximum number of matched pages to print.
        #[arg(long, default_value = "50")]
        limit: usize,
    },
    /// Probe every composed hook and report supported stages, AST types,
    /// and schema version. SPEC-032 REQ-3216.
    ///
    /// Each hook is spawned, asked for its capability probe, and shut
    /// down cleanly. Non-zero exit when any hook's probe fails.
    #[command(
        after_help = "Examples:\n  ztl hook capabilities\n  ztl hook capabilities --json\n  ztl hook capabilities --stage transform"
    )]
    Capabilities {
        /// Theme name (looks in .ztl/themes/<name>/hooks/).
        #[arg(long, default_value = "default")]
        theme: String,
        /// Restrict the report to a single stage.
        #[arg(long, value_enum)]
        stage: Option<AuthoringStage>,
    },
}

/// Subcommands for `ztl ecosystem`.
#[derive(Subcommand)]
pub enum EcosystemCommand {
    /// Probe every registered ecosystem's runtime and report per-ecosystem
    /// detection + version + configured-hook count + reachable plugins.
    ///
    /// Exit 0 when every *configured* ecosystem is available; non-zero only
    /// when the vault's hooks reference an ecosystem whose runtime is
    /// missing or below the minimum version. The zero-configured state
    /// always exits 0.
    Check {
        /// Theme name (looks in `.ztl/themes/<name>/hooks/`).
        #[arg(long, default_value = "default")]
        theme: String,
        /// Emit machine-readable JSON instead of the table. Equivalent to
        /// the global `--json` flag; kept here so the per-command help is
        /// self-documenting.
        #[arg(long)]
        json: bool,
    },
}

/// Stage for `ztl hook new`. Matches the three on-disk `<stage>.d/`
/// directory names from SPEC-032 REQ-3201.
#[derive(Clone, ValueEnum, PartialEq, Eq, Debug)]
pub enum AuthoringStage {
    #[value(name = "pre-parse")]
    PreParse,
    Transform,
    #[value(name = "post-render")]
    PostRender,
}

impl AuthoringStage {
    pub fn as_str(&self) -> &'static str {
        match self {
            AuthoringStage::PreParse => "pre-parse",
            AuthoringStage::Transform => "transform",
            AuthoringStage::PostRender => "post-render",
        }
    }
}

/// Language selection for `ztl hook new`. The scaffolder writes a
/// persistent-mode skeleton in the chosen language with a shebang so
/// `chmod +x` suffices to make it runnable.
#[derive(Clone, ValueEnum, PartialEq, Eq, Debug)]
pub enum HookLang {
    Py,
    Js,
    Sh,
}

impl HookLang {
    pub fn ext(&self) -> &'static str {
        match self {
            HookLang::Py => "py",
            HookLang::Js => "js",
            HookLang::Sh => "sh",
        }
    }
}

/// Ecosystem adapter to target when scaffolding. The scaffolded manifest
/// carries the SPEC-033 REQ-3312 fields the chosen ecosystem requires:
///
/// - `pandoc` — `ecosystem = "pandoc"` + `lua_filter = "filters/<name>.lua"`,
///   plus a starter identity Lua filter on disk at that path.
/// - `mdbook` — `ecosystem = "mdbook"` + `exec = "mdbook-<name>"` +
///   `scope = "page"`. Place the preprocessor binary on PATH (or rename
///   `exec`) before `ztl build`.
/// - `remark` — `ecosystem = "remark"` + `package = "remark-<name>"`.
///   Install the npm package under the vault's `node_modules/`.
///
/// SPEC-033 REQ-3303/3304/3305.
#[derive(Clone, ValueEnum, PartialEq, Eq, Debug)]
pub enum HookEcosystem {
    Pandoc,
    Mdbook,
    Remark,
}

impl HookEcosystem {
    pub fn as_str(&self) -> &'static str {
        match self {
            HookEcosystem::Pandoc => "pandoc",
            HookEcosystem::Mdbook => "mdbook",
            HookEcosystem::Remark => "remark",
        }
    }
}

/// Pipeline stage at whose *input* boundary `ztl ast sample` emits a view
/// of the page. `pre-parse` prints the raw markdown text (frontmatter
/// stripped); `transform` prints the ztl-ext AST JSON; `post-render` prints
/// the rendered HTML fragment.
#[derive(Clone, ValueEnum, PartialEq, Eq, Debug, Default)]
pub enum AstStage {
    #[value(name = "pre-parse")]
    PreParse,
    #[default]
    Transform,
    #[value(name = "post-render")]
    PostRender,
}

#[derive(Subcommand)]
pub enum AstCommand {
    /// Print the canonical ztl-ext AST (or pre-parse text / post-render
    /// HTML) for a page file.
    Sample {
        /// Path to a Markdown file. Relative paths resolve against the
        /// current working directory (not the vault root) so the command is
        /// usable outside an indexed vault.
        file: String,
        /// Which pipeline stage's input to emit. Defaults to `transform`,
        /// which prints the AST JSON a transform hook would receive.
        #[arg(long, value_enum, default_value_t = AstStage::default())]
        stage: AstStage,
    },
    /// Tree-aware structural diff of two ztl-ext AST JSON documents.
    /// Exits non-zero when the diff is non-empty.
    Diff {
        /// Path to the *before* AST JSON file.
        before: String,
        /// Path to the *after* AST JSON file.
        after: String,
    },
}

#[derive(Subcommand)]
pub enum AgentCommand {
    /// Run an on-agent hook with task context (REQ-020-023)
    #[command(
        after_help = "Examples:\n  ztl agent run link-checker\n  ztl agent run summariser --pages \"Note A\" \"Note B\" --budget 4000"
    )]
    Run {
        /// Agent task name
        name: String,
        /// Theme name (looks in .ztl/themes/<name>/hooks/)
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
        /// Output format for the explanation (renamed from --format to
        /// avoid collision with the global -f/--format flag).
        #[arg(long = "as", default_value = "json")]
        output_as: ExplainFormat,
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
        /// Output format: spl (reconstructed SPL with provenance comments),
        /// json (structured theory). Renamed from --format to avoid collision
        /// with the global -f/--format flag.
        #[arg(long = "as", default_value = "json")]
        output_as: ExportFormat,
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

/// Transport mode for the MCP server.
#[derive(Clone, clap::ValueEnum, PartialEq)]
pub enum McpTransport {
    Stdio,
    Http,
}

/// Category filter for `ztl diff`.
#[derive(Clone, ValueEnum)]
pub enum DiffFilter {
    Pages,
    Links,
    Orphans,
    #[value(name = "dead-links")]
    DeadLinks,
}

/// Subcommands for `ztl history` (requires --features history).
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
