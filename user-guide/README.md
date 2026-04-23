# ztl user guide

A comprehensive, ~40-page guide to ztl, written as a ztl vault.

Everything in this folder is plain Markdown with `[[wikilinks]]`. Start at [Index.md](Index.md).

## How to read it

Pick whichever you prefer:

- **In a web browser** — `make serve` (or `ztl serve --theme quickstart`), then open <http://localhost:3000>. The `quickstart` theme redirects `/` to the Quick Start page; without it you get ztl's default vault landing (stats + page grid) and have to click through to Index.
- **As a static site** — `make build` (or `ztl build --theme quickstart --out-dir site`), then upload `site/` to any HTTP host.
- **In a terminal** — run `ztl view Index` for the two-pane reader.
- **In Obsidian / Logseq / Foam / Dendron** — open this folder as a vault.
- **On GitHub / Codeberg** — the wikilinks render as plain text, but every page is readable.

## About

- **What is ztl?** See [getting-started/What is ztl.md](getting-started/What%20is%20ztl.md).
- **Canonical ztl source:** <https://codeberg.org/anuna/ztl>
- **Scope of this guide:** every feature shipped in the ztl CLI as of v0.5, aimed at writers and knowledge workers.

## Contributing

Found a mistake? Run `ztl check` on the vault and open an issue or PR at the ztl source repository.

## License

Same as ztl: [AGPL-3.0](../LICENSE).
