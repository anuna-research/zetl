---
title: LLM Wikis
tags: [concepts, agents, mcp, context]
---

# LLM Wikis

In April 2026, Andrej Karpathy published a short gist titled ["LLM Wiki"](https://gist.github.com/karpathy/442a6bf555914893e9891c11519de94f) describing a pattern for personal knowledge bases: instead of asking an LLM to rediscover information from raw documents on every query — as retrieval-augmented generation typically does — have the model incrementally build and maintain a persistent, interlinked Markdown directory. The wiki sits between you and the raw sources. Queries read the wiki first; new material is ingested into it; periodic "lint" passes find contradictions and stale claims. His summary: *"Obsidian is the IDE; the LLM is the programmer; the wiki is the codebase."*

This page is zetl's response: where the pattern fits what zetl already does, where zetl extends it, and where our priorities diverge.

---

## The pattern in one paragraph

Karpathy's three layers are **raw sources** (immutable, outside the wiki), **the wiki** (LLM-owned Markdown, graph-linked, with an `index.md` and a chronological `log.md`), and **the schema** — a `CLAUDE.md` or `AGENTS.md` that disciplines the agent into a "wiki maintainer rather than a generic chatbot." The three operations are **Ingest** (LLM reads a new source, writes a summary page, updates 10–15 cross-referenced pages, appends a log line), **Query** (LLM searches the wiki, synthesises, cites, optionally files the answer back as a new page), and **Lint** (periodic pass for contradictions, orphans, stale claims, missing links).

The historical pivot is a reframing of the Memex. Vannevar Bush's 1945 sketch failed not because the microfilm technology was inadequate — others eventually solved that — but because *the maintenance labour was never resolved*. Humans abandon associative trails because the bookkeeping cost outpaces the benefit. LLMs, in Karpathy's framing, close that gap: *"LLMs don't get bored, don't forget to update a cross-reference, and can touch 15 files in one pass."*

---

## What zetl already is

If you read [[Intellectual Heritage]], you've seen zetl's lineage: Zettelkasten, Memex, Xanadu, wiki. The design commitments that fall out of that lineage — local Markdown, graph-first, file-native, no lock-in — are precisely the substrate Karpathy's pattern assumes. You do not have to choose between *"run zetl"* and *"try the LLM wiki pattern."* They are the same workflow with different vocabulary.

Specifically:

| Karpathy's gist | zetl equivalent |
|-----------------|-----------------|
| Raw source files on disk | Vault is a plain Markdown folder; no database; see [[Local-first]] |
| Wiki graph of linked pages | `[[wikilinks]]`, auto-extracted into [[The Link Graph]] |
| Obsidian graph view as "shape of the wiki" | Same link graph, plus [[Following Links]] and interactive graph view |
| `index.md` maintained by the LLM | The graph is derived from wikilinks, not from a hand-kept index (see below) |
| `log.md` append-only chronological record | [[Page History]] and [[Time Travel]] — every page has its own history ledger |
| `qmd` or ad-hoc search scripts | `zetl search`, `zetl links`, `zetl backlinks`, `zetl similar` |
| `CLAUDE.md` / `AGENTS.md` as schema | Same files; place them at the vault root. See [[MCP Server]] for how agents discover them |
| "lint for dead links, orphans" | `zetl check` — dead wikilinks and orphaned pages, exit-coded for CI |
| "lint for contradictions" | `zetl reason status` — defeasibly-provable claims, literal by literal |

Karpathy's pattern describes what many zetl users already do. The gist's value is that it names the pattern crisply enough to defend it against the default — dumping files into a chat window and asking questions — which wastes both context and continuity.

---

## What zetl adds

Three things in Karpathy's sketch have natural extensions in zetl that the gist does not reach for.

### The graph is extracted, not maintained

Karpathy has the LLM keep `index.md` as a catalog of one-line summaries, and notes this *"works surprisingly well at moderate scale (~100 sources, ~hundreds of pages)."* It does — until the index drifts from the actual page set. Any hand-kept (or agent-kept) index eventually falls out of sync with what exists on disk, because ingestion and renaming are more frequent than full re-indexing.

zetl's graph is re-derived from `[[wikilinks]]` on every scan. `zetl index` is cheap (~30 ms for the ~50-page vault you are reading) and idempotent: delete the cache and rebuild, and you get the identical graph. You never have to ask whether the index reflects the files. The files are the index.

This is not an argument against `index.md` — a human-readable catalog is still valuable for the LLM to read first. It is an argument that the catalog should be *generated* from the graph rather than hand-maintained in parallel to it. `zetl graph dump` and the MCP `search` tool are where an agent would read the vault's shape before writing or querying.

### Contradictions are logical, not stylistic

The gist's lint pass flags *"contradictions, stale claims, orphan pages."* Orphans and stale claims are structural; a search pass finds them. Contradictions are different: two pages asserting incompatible facts is a claim about *logic*, not text. Flagging them is useful. Resolving them requires a system for saying *which claim wins, and why*.

zetl has one. [[What is Defeasible Reasoning]] gives you `normally`, `always`, and `except` — rules that can be defeated by more-specific rules, with explicit priority when defeaters tie. A page can assert *"this user can edit any page"*; a later page can assert *"…except the access-policy page, and this assertion is stronger"*; `zetl reason explain` shows the full derivation chain with source citations. Contradictions are not warnings: they are decidable.

This matters most when the LLM is the one writing the claims. Synthesis-by-LLM produces confident-sounding sentences. An agent that writes *"Project X ships in Q3"* on Monday and *"Project X is on hold"* on Thursday has not made a mistake — it has encountered new information. The lint pass should detect the pair; the reasoning engine should decide which one holds now, with provenance.

### The tool surface is typed

Karpathy's agent accesses the wiki by searching with `qmd`, reading files, and editing them. That works. The alternative, which zetl already ships, is an MCP server that exposes the vault as [typed tools](https://modelcontextprotocol.io): `search`, `get_page`, `links`, `backlinks`, `path`, `similar`, `check`, `reason`. See [[MCP Server]]. The difference is small for a solo user with a trusted agent and enormous for teams: tool calls are auditable, scoped by [[Capability URLs|delegate token]], and decoupled from filesystem access. An agent running against the MCP surface never touches your files directly; it asks zetl to, and zetl decides.

For [[Running a Team Server|team vaults]] this is not optional. The question *"whose wiki is this, and who can ingest into it?"* is answered by the same SPL-based ACL that governs human editors — an agent authenticates with a user's delegated token and operates under that user's permissions. See [[Access Control]].

---

## Where we differ

There is a real disagreement of emphasis, not of architecture.

Karpathy's framing is **LLM-first**: *"You never (or rarely) write the wiki yourself — the LLM writes and maintains all of it."* The human sources and asks; the LLM files. zetl's framing is **file-first**: the vault is yours, Markdown files on your disk, edited in whatever tool you like; the LLM is a useful collaborator that can read, suggest, and (with explicit permission) write.

In practice these compose. But they imply different defaults. If the LLM owns the wiki, you stop editing pages by hand because you are afraid of clobbering the agent's work. If the vault is the human's and the agent is a collaborator, the agent commits through the same git log you do, attributable and revertable. [[Co-editing]] and [[Invitations]] already make this distinction for human collaborators; extending it to agents costs nothing extra.

The divergence is clearest around `index.md`. Karpathy uses it because it is the LLM's map of its own work. zetl prefers a graph extracted from the files because the map is the territory. If you want both — a human-readable catalog and an auto-derived graph — run `zetl index` and check in a generated `Index.md` alongside it. The CLI already supports this through the `quickstart` theme (the page you are reading is served that way).

---

## How to run the pattern today

A minimal LLM-wiki setup on zetl:

```bash
# Create a vault
mkdir my-wiki && cd my-wiki

# Put an AGENTS.md at the root — this is the "schema" layer.
# Describe the wiki's conventions: naming, tagging, which folders
# hold raw sources vs synthesised pages, when to ingest vs query.

# Run the MCP server so your agent can reach the vault.
zetl -d . mcp                             # stdio, for Claude Desktop / Cursor
zetl -d . mcp --transport http --port 3100  # HTTP, for remote agents

# Scope what the agent can do.
zetl delegate --tools search,get_page,links,backlinks --expiry 30d

# Ingest a source by dropping it in a raw/ folder, then prompting the agent:
#   "Read raw/new-paper.pdf, summarise as a page under research/,
#    update any concept pages that cross-reference it, append a log entry."

# Lint weekly — structural + logical.
zetl -d . check                           # dead links, orphans
zetl -d . reason status                   # defeasible conclusions

# Query by talking to the agent, which calls MCP tools under the hood.
```

The `AGENTS.md` file is where your version of Karpathy's *"schema"* lives — naming conventions, folder layout, what counts as raw vs synthesised, when the agent should ask permission before writing. This is the one file that genuinely has to be hand-written; everything else either generates from the files or reads the schema and acts accordingly.

---

## The honest summary

Karpathy's gist is right about three things that are easy to miss. First, *"nothing is built up"* is the correct diagnosis of dumping-files-into-context as a workflow — you pay full retrieval cost every time, and whatever synthesis the model did last week is gone. Second, the Memex failed on maintenance labour, not on storage; any theory of personal knowledge tools that does not address who keeps the links fresh is incomplete. Third, the LLM-as-librarian framing is what makes the bookkeeping cost tractable for the first time.

zetl adds: the file layout should be the authority (not a separately-maintained index), contradictions deserve a logic (not just a warning), and the agent's access to the vault should go through a typed, scoped surface (not bare filesystem). The combination — a Karpathy-style LLM wiki running on top of zetl — is a workflow that does not require new infrastructure. Most of it ships today.

---

## Further reading

- Andrej Karpathy, ["LLM Wiki"](https://gist.github.com/karpathy/442a6bf555914893e9891c11519de94f), April 2026 — the original gist
- Vannevar Bush, "As We May Think", *The Atlantic*, July 1945 — the Memex essay
- [Model Context Protocol](https://modelcontextprotocol.io) — the standard zetl's agent surface implements

## Related

- [[Intellectual Heritage]]
- [[MCP Server]]
- [[What is Defeasible Reasoning]]
- [[The Link Graph]]
- [[Access Control]]
- [[Local-first]]
