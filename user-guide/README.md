# zetl user guide

A comprehensive, ~40-page guide to zetl, written as a zetl vault.

Everything in this folder is plain Markdown with `[[wikilinks]]`. Start at [Index.md](Index.md).

## How to read it

Pick whichever you prefer:

- **In a web browser** — `make serve` (or `zetl serve --theme quickstart`), then open <http://localhost:3000>. The `quickstart` theme redirects `/` to the Quick Start page; without it you get zetl's default vault landing (stats + page grid) and have to click through to Index.
- **As a static site** — `make build` (or `zetl build --theme quickstart --out-dir site`), then upload `site/` to any HTTP host.
- **In a terminal** — run `zetl view Index` for the two-pane reader.
- **In Obsidian / Logseq / Foam / Dendron** — open this folder as a vault.
- **On GitHub / Codeberg** — the wikilinks render as plain text, but every page is readable.

## About

- **What is zetl?** See [getting-started/What is zetl.md](getting-started/What%20is%20zetl.md).
- **Canonical zetl source:** <https://codeberg.org/anuna/zetl>
- **Scope of this guide:** every feature shipped in the zetl CLI as of v0.5, aimed at writers and knowledge workers.

## Contributing

Found a mistake? Run `zetl check` on the vault and open an issue or PR at the zetl source repository.

## License

Same as zetl: [AGPL-3.0](../LICENSE).
