---
title: Static Site Export
tags: [reading, build, publishing]
---

# Static Site Export

`ztl build` turns your vault into a folder of plain HTML, CSS, and JavaScript. No runtime, no database, no server process — every page is a file you can upload to any static host. The output looks the same as [[Web Server]] minus the editor.

## Why build a static site

A static site is the right deliverable when:

- You want to publish research notes, a personal wiki, or project docs to the public internet.
- You want a vault readable from any browser with zero operational overhead.
- You want something to hand to collaborators, reviewers, or your future self that doesn't depend on the `ztl` binary being installed.
- You want archival: HTML works the same way in 2040 as it does in 2026.

Because nothing on the published site writes back, the edit button is absent and the save/delete endpoints aren't emitted. Everything else — wikilinks, backlinks, transclusion panels with SVG bridges, the graph widget, full-text search, per-page history (if built with `--features history`) — lands in the output as static HTML and JSON.

## Running it

```bash
ztl build                             # writes ./dist/
ztl -d ~/notes build
ztl build --out-dir site              # custom output directory
ztl build --theme paper               # pick a theme
ztl build --site-url https://vault.example    # absolute URLs for social cards
```

Preview the result locally with any static file server:

```bash
python3 -m http.server -d dist 8080
```

Then open `http://localhost:8080/`.

## Output structure

```
dist/
  index.html                 # vault landing page (stats + page grid)
  _static/                   # theme static assets (CSS, JS, vendor bundles)
  _graph.html                # full-screen graph view
  _history.html              # vault recent-changes view (history builds only)
  page/
    Zettelkasten Method/
      index.html             # one page per note
      _history.html          # per-page timeline (history builds only)
    Project Notes/
      index.html
```

Each note lives at a stable URL (`/page/<slug>/`) driven by the page title, so `[[Zettelkasten Method]]` in source becomes `<a href="/page/Zettelkasten Method/">` in HTML. Slugs survive across builds as long as the page title doesn't change.

## Flags that matter

| Flag | Default | Purpose |
|------|---------|---------|
| `-o, --out-dir <DIR>` | `dist` | Output directory. Wiped and rewritten on each build. |
| `--theme <THEME>` | `default` | Theme from `.ztl/themes/<name>/`. See [[Customising the Look]]. |
| `--public <DIR>` | — | Files copied over the output root after generation (good for `CNAME`, `robots.txt`). |
| `--site-url <URL>` | — | Canonical URL; used for absolute `og:image` URLs so social scrapers resolve them. |
| `--safe-mode` | off | Skip every hook except ones the theme explicitly declares. |
| `--at <TIME-EXPR>` | — | Build a past snapshot. See [[Time Travel]]. |
| `--strict-parsers` | off | Promote mixed-parser warnings to errors. |

## Hosting

`dist/` is a plain folder. Any of these work:

- **GitHub Pages / Codeberg Pages** — commit `dist/` to a pages branch, or emit straight into your pages repo.
- **Netlify / Cloudflare Pages / Vercel** — point at the repo, set the build command to `ztl build`, publish directory `dist`.
- **S3 + CloudFront, Backblaze B2, any object store** — `aws s3 sync dist/ s3://bucket/` and serve via the usual static-site front.
- **Your own server** — `rsync -a --delete dist/ user@host:/var/www/vault/` with nginx or Caddy in front.

There is no runtime for any of these to support — the graph widget, search, and SPA navigation shell are all client-side JS bundled under `_static/`. No CDN fallback, no analytics pings.

## Building vs serving

Rule of thumb: use [[Web Server]] while you're writing and linking; use `ztl build` when you want to publish. Many workflows run both — `ztl serve` during authoring, `ztl build` from CI on every push to main.

## Related

- [[Web Server]]
- [[Customising the Look]]
- [[Capability URLs]]
- [[Time Travel]]
- [[CLI Overview]]
