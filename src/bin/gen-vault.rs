//! Synthetic vault generator (PERF-BUILD-2026-05-12 task: bench-fixture).
//!
//! Emits a deterministic synthetic vault to disk for use by the perf
//! benchmarks (large-vault build / scan / search). Same `--seed`,
//! `--pages`, `--avg-links`, and `--out` always produces byte-identical
//! output across runs and platforms.
//!
//! The pages mimic the on-disk shape that `zetl::scanner::parse_file`
//! expects:
//!   * YAML frontmatter (`---\ntitle: ...\ndate: ...\n---`)
//!   * 3-6 ATX headings, each with a paragraph of ~40-80 words
//!   * one fenced code block per page (so the parser exercises the
//!     code-block path)
//!   * ~K outbound `[[wikilinks]]` per page on average, including at
//!     least one transclusion `![[...]]`
//!   * link targets pseudo-randomly drawn — about 80% intra-vault, 20%
//!     phantom — so phantom-link reporting also gets exercised
//!
//! No new dependencies are added; randomness is provided by a tiny
//! inline xorshift64* PRNG (`Rng`) seeded from `--seed`.
//!
//! This binary has zero impact on production code paths — it is a
//! dev-tool only and lives under `src/bin/` so `cargo build` of the
//! library is unaffected.

use std::fs;
use std::io::{BufWriter, Write};
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::Parser;

/// Synthetic vault generator for zetl perf benchmarks.
#[derive(Debug, Parser)]
#[command(
    name = "gen-vault",
    about = "Generate a deterministic synthetic vault for perf benchmarking",
    version
)]
struct Cli {
    /// Number of pages to emit.
    #[arg(long, default_value_t = 1000)]
    pages: usize,

    /// Average outbound wikilinks per page (transclusion + plain links combined).
    #[arg(long = "avg-links", default_value_t = 12)]
    avg_links: usize,

    /// Output directory. Created if missing. Refuses to write into a
    /// non-empty directory unless --force.
    #[arg(long)]
    out: PathBuf,

    /// PRNG seed (determinism control). Same seed + same other flags
    /// always produces byte-identical output.
    #[arg(long, default_value_t = 42)]
    seed: u64,

    /// Allow writing into a non-empty output directory (overwrites
    /// colliding files; pre-existing extra files are left in place).
    #[arg(long)]
    force: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    run(&cli)
}

fn run(cli: &Cli) -> Result<()> {
    if cli.pages == 0 {
        bail!("--pages must be > 0");
    }

    // Create the output dir if missing; refuse non-empty unless --force.
    if cli.out.exists() {
        if !cli.out.is_dir() {
            bail!("--out path {} exists but is not a directory", cli.out.display());
        }
        let mut iter = fs::read_dir(&cli.out)
            .with_context(|| format!("failed to read --out dir {}", cli.out.display()))?;
        let non_empty = iter.next().is_some();
        if non_empty && !cli.force {
            bail!(
                "--out dir {} is not empty; pass --force to write into it",
                cli.out.display()
            );
        }
    } else {
        fs::create_dir_all(&cli.out)
            .with_context(|| format!("failed to create --out dir {}", cli.out.display()))?;
    }

    // Compute the zero-pad width once; matches the test acceptance
    // requirement that `page-NNNN.md` files line-sort by index.
    let pad = pad_width(cli.pages);

    // Pre-build the list of (folder-relative-path, page-name) tuples so
    // that link generation can target other pages by basename.
    let layout: Vec<PageLoc> = (0..cli.pages).map(|i| page_location(i, pad, cli.seed)).collect();

    // For each page write the markdown body.
    for (idx, loc) in layout.iter().enumerate() {
        let abs_path = cli.out.join(&loc.rel_path);
        if let Some(parent) = abs_path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create parent dir for {}", abs_path.display())
            })?;
        }

        let file = fs::File::create(&abs_path)
            .with_context(|| format!("failed to create {}", abs_path.display()))?;
        let mut w = BufWriter::new(file);
        write_page(&mut w, idx, &layout, cli.seed, cli.avg_links)
            .with_context(|| format!("failed to write {}", abs_path.display()))?;
        w.flush()
            .with_context(|| format!("failed to flush {}", abs_path.display()))?;
    }

    Ok(())
}

// ── Layout ─────────────────────────────────────────────────────────────

/// On-disk location of a synthesised page.
struct PageLoc {
    /// Path relative to `--out`, e.g. `topic-3/sub-1/page-0042.md`.
    rel_path: PathBuf,
    /// Basename without extension, e.g. `page-0042` — the page name
    /// `zetl` will resolve.
    page_name: String,
}

/// Compute the zero-pad width so that `page-NNNN.md` filenames are wide
/// enough for the largest page index. Min width is 4.
fn pad_width(pages: usize) -> usize {
    let mut w = 4;
    let mut n = pages.saturating_sub(1);
    let mut digits = 1;
    while n >= 10 {
        n /= 10;
        digits += 1;
    }
    if digits > w {
        w = digits;
    }
    w
}

/// Decide where page `idx` lives on disk. Distribution:
///   * 8 top-level folders (`topic-0` .. `topic-7`)
///   * ~30% pages live at the top level of their topic folder
///   * ~50% live one level deep (`topic-i/sub-j`)
///   * ~20% live two levels deep (`topic-i/sub-j/sub-k`)
///
/// The choice is fully deterministic in `seed` + `idx` so the generator
/// is reproducible.
fn page_location(idx: usize, pad: usize, seed: u64) -> PageLoc {
    let mut rng = Rng::new(seed.wrapping_add(0x9E37_79B9_7F4A_7C15).wrapping_add(idx as u64));
    let topic = (idx % 8) as u64; // even spread across 8 top folders
    let depth_pick = rng.next_u32() % 100;
    let depth: u32 = if depth_pick < 30 {
        0
    } else if depth_pick < 80 {
        1
    } else {
        2
    };

    let mut rel = PathBuf::from(format!("topic-{}", topic));
    if depth >= 1 {
        let sub = rng.next_u32() % 6;
        rel.push(format!("sub-{}", sub));
    }
    if depth >= 2 {
        let sub2 = rng.next_u32() % 4;
        rel.push(format!("sub-{}", sub2));
    }

    let page_name = format!("page-{:0width$}", idx, width = pad);
    rel.push(format!("{}.md", page_name));
    PageLoc {
        rel_path: rel,
        page_name,
    }
}

// ── Page body ──────────────────────────────────────────────────────────

/// Write one page's markdown body to `w`. The output is fully
/// determined by `(idx, layout, seed, avg_links)`.
fn write_page<W: Write>(
    w: &mut W,
    idx: usize,
    layout: &[PageLoc],
    seed: u64,
    avg_links: usize,
) -> std::io::Result<()> {
    // Per-page RNG: independent from layout-rng so changes to layout
    // distribution don't shift body content.
    let mut rng = Rng::new(seed.wrapping_add(0xD1B5_4A32_D192_ED03).wrapping_add(idx as u64));

    let title = format!("Page {:04}", idx);
    let date = deterministic_date(idx, seed);
    writeln!(w, "---")?;
    writeln!(w, "title: \"{}\"", title)?;
    writeln!(w, "date: {}", date)?;
    writeln!(w, "---")?;
    writeln!(w)?;

    // Choose link budget — Poisson-ish around avg_links via sum of two
    // uniform draws. Always at least 2 so we can guarantee one
    // transclusion + one plain link.
    let lo = avg_links.saturating_sub(3) as u32;
    let hi = (avg_links + 3) as u32;
    let span = hi.saturating_sub(lo).max(1);
    let link_budget = (lo + (rng.next_u32() % span)).max(2) as usize;

    // Decide how many headings (3..=6) and roughly partition links
    // across them.
    let heading_count = 3 + (rng.next_u32() as usize % 4); // 3..=6
    let mut links_per_heading = vec![0usize; heading_count];
    for slot in 0..link_budget {
        links_per_heading[slot % heading_count] += 1;
    }

    // Pick which heading owns the (single, mandatory) transclusion and
    // which owns the (single, mandatory) fenced code block.
    let transclude_heading = (rng.next_u32() as usize) % heading_count;
    let code_block_heading = (rng.next_u32() as usize) % heading_count;

    for h in 0..heading_count {
        let heading_text = HEADINGS[(idx + h) % HEADINGS.len()];
        writeln!(w, "## {}", heading_text)?;
        writeln!(w)?;

        // Paragraph of ~40-80 words.
        let word_count = 40 + (rng.next_u32() as usize % 41); // 40..=80
        write_lorem(w, &mut rng, word_count)?;
        writeln!(w)?;
        writeln!(w)?;

        // Fenced code block (once per page) — placed under
        // `code_block_heading`. Uses ``` not ~~~.
        if h == code_block_heading {
            writeln!(w, "```rust")?;
            writeln!(w, "fn page_{:04}() -> &'static str {{", idx)?;
            writeln!(w, "    \"synthetic page {} sample\"", idx)?;
            writeln!(w, "}}")?;
            writeln!(w, "```")?;
            writeln!(w)?;
        }

        // Outbound wikilinks for this heading. The mandatory
        // transclusion goes first inside `transclude_heading`.
        let n_links = links_per_heading[h];
        if n_links == 0 && h != transclude_heading {
            continue;
        }

        // We always emit the transclusion in the chosen heading even if
        // its link slot count was 0.
        let mut emitted_in_block = 0usize;
        if h == transclude_heading {
            let target = pick_target(&mut rng, idx, layout, /* phantom_chance */ 20);
            writeln!(w, "See also ![[{}]] for context.", target)?;
            emitted_in_block += 1;
        }

        // Fill the rest of this heading's link budget with plain
        // wikilinks, three per sentence, breaking lines as we go.
        if n_links > emitted_in_block {
            let remaining = n_links - emitted_in_block;
            let mut buf = String::from("Related: ");
            for k in 0..remaining {
                let target = pick_target(&mut rng, idx, layout, 20);
                if k > 0 {
                    buf.push_str(", ");
                }
                buf.push_str(&format!("[[{}]]", target));
            }
            buf.push('.');
            writeln!(w, "{}", buf)?;
        }
        writeln!(w)?;
    }

    Ok(())
}

/// Pick a wikilink target. About `phantom_chance` percent of the time
/// emit a phantom (non-existent) page name; otherwise pick a real
/// neighbour (avoiding self-links).
fn pick_target(rng: &mut Rng, self_idx: usize, layout: &[PageLoc], phantom_chance: u32) -> String {
    let pick = rng.next_u32() % 100;
    if pick < phantom_chance {
        // Phantom name — guaranteed not to clash with the
        // `page-NNNN` namespace.
        let n = rng.next_u32();
        format!("phantom-{:08x}", n)
    } else if layout.len() <= 1 {
        // Single-page vault: fall back to phantom.
        let n = rng.next_u32();
        format!("phantom-{:08x}", n)
    } else {
        let mut t = (rng.next_u32() as usize) % layout.len();
        if t == self_idx {
            t = (t + 1) % layout.len();
        }
        layout[t].page_name.clone()
    }
}

fn write_lorem<W: Write>(w: &mut W, rng: &mut Rng, words: usize) -> std::io::Result<()> {
    let mut line_words = 0usize;
    for i in 0..words {
        let word = LOREM[(rng.next_u32() as usize) % LOREM.len()];
        if i > 0 {
            if line_words >= 12 {
                writeln!(w)?;
                line_words = 0;
            } else {
                write!(w, " ")?;
            }
        }
        write!(w, "{}", word)?;
        line_words += 1;
    }
    writeln!(w, ".")?;
    Ok(())
}

/// Deterministic ISO-8601 date string drawn from `(idx, seed)`. The
/// year stays in [2020, 2026] so frontmatter parses cleanly under any
/// downstream date validation.
fn deterministic_date(idx: usize, seed: u64) -> String {
    let mut rng = Rng::new(seed.wrapping_add(0xC2B2_AE3D_27D4_EB4F).wrapping_add(idx as u64));
    let year = 2020 + (rng.next_u32() % 7); // 2020..=2026
    let month = 1 + (rng.next_u32() % 12); // 1..=12
    // Day capped at 28 to avoid month-length edge cases.
    let day = 1 + (rng.next_u32() % 28);
    format!("{:04}-{:02}-{:02}", year, month, day)
}

// ── PRNG ───────────────────────────────────────────────────────────────

/// Tiny xorshift64* PRNG. Sufficient quality for synthetic content
/// generation; see Marsaglia 2003 for the constants. Inline so we don't
/// need to add `rand` as a dependency just for the dev-tool.
struct Rng {
    state: u64,
}

impl Rng {
    fn new(seed: u64) -> Self {
        // Avoid the all-zero state which xorshift cannot escape.
        let s = if seed == 0 { 0xDEAD_BEEF_CAFE_F00D } else { seed };
        Rng { state: s }
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn next_u32(&mut self) -> u32 {
        // Take the top 32 bits — they have better statistical quality
        // than the bottom on xorshift64*.
        (self.next_u64() >> 32) as u32
    }
}

// ── Static word/heading pools ──────────────────────────────────────────

const HEADINGS: &[&str] = &[
    "Background",
    "Motivation",
    "Approach",
    "Design",
    "Implementation",
    "Discussion",
    "Open Questions",
    "Related Work",
    "Examples",
    "Notes",
    "Caveats",
    "Future Work",
    "References",
    "Summary",
];

const LOREM: &[&str] = &[
    "lorem", "ipsum", "dolor", "sit", "amet", "consectetur", "adipiscing", "elit", "sed", "do",
    "eiusmod", "tempor", "incididunt", "ut", "labore", "et", "dolore", "magna", "aliqua", "enim",
    "ad", "minim", "veniam", "quis", "nostrud", "exercitation", "ullamco", "laboris", "nisi",
    "aliquip", "ex", "ea", "commodo", "consequat", "duis", "aute", "irure", "in", "reprehenderit",
    "voluptate", "velit", "esse", "cillum", "fugiat", "nulla", "pariatur", "excepteur", "sint",
    "occaecat", "cupidatat", "non", "proident", "sunt", "culpa", "qui", "officia", "deserunt",
    "mollit", "anim", "id", "est", "laborum", "graph", "vault", "page", "link", "node", "edge",
    "merkle", "leaf", "scan", "build", "snapshot", "knowledge", "wikilink", "transclusion",
];
