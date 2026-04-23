# Build Command

`ztl build` generates a static HTML site from your vault, with the same look and feel as [[Serve Command]] minus the edit button and save functionality.

## Usage

```bash
ztl -d ./my-vault build                  # generates dist/
ztl -d ./my-vault build --out-dir site   # custom output directory
```

## Flags

| Flag | Default | Description |
|------|---------|-------------|
| `--out-dir <path>` | `dist` | Output directory for the generated site |

## Output structure

```
dist/
  index.html              # vault overview with stats and page grid
  page/
    Some Page/index.html   # one page per note
    Another/index.html
```

## Features

- Rendered Markdown with syntax highlighting
- Backlink lists on each page
- Transclusion panels with SVG bridge connectors
- Page grid on the index page
- Clean URLs (`/page/Scanner/` instead of `/page/Scanner.html`)
- `history-index.json` export (when built with `--features history`) — see [[History Command]]

## Deployment

The output can be uploaded to any static host:

- GitHub Pages
- Netlify
- S3 + CloudFront
- Any web server

Preview locally:

```bash
python3 -m http.server -d dist 8080
```

See also: [[CLI Reference]], [[Serve Command]], [[View Command]]
