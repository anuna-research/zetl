---
title: "SPEC-012: zetl — Customisable Web Templates for Serve and Build"
version: 0.1.0
status: draft
audience: agent, human
date: 2026-03-01
---

# SPEC-012: zetl — Customisable Web Templates for Serve and Build

## Information Table

| Field          | Value                                                          |
| -------------- | -------------------------------------------------------------- |
| Document ID    | SPEC-012                                                       |
| Title          | zetl — Customisable Web Templates for Serve and Build          |
| Version        | 0.1.0                                                          |
| Status         | Draft                                                          |
| Author         | Agent (USDD Protocol v1.0.0)                                   |
| Date           | 2026-03-01                                                     |
| Audience       | Agent, Human                                                   |
| Trace          | USDD Agent Protocol v1.0.0                                     |
| Parent         | SPEC-001: zetl — Bi-directional Link Graph CLI                 |
| Related        | SPEC-009: zetl view — Xanadu-Inspired Transclusion TUI         |

---

## 1. Overview

zetl's `serve` and `build` commands render a vault as a navigable website — `serve` runs a live development server with edit capabilities, and `build` generates a static site for deployment. Both commands currently generate all HTML in Rust code via `format!()` string concatenation in `html.rs`, `routes.rs`, and `build.rs`. CSS comes from CDN links (DaisyUI/Tailwind), JavaScript is inline, and the layout is hardcoded. This means any customisation — loading custom JS like Fountain.js for screenplay rendering, changing the layout, adding analytics, applying a brand stylesheet — requires recompiling zetl.

This specification introduces a **template engine** that decouples vault data from HTML rendering. zetl adopts a "headless CMS" pattern: structured vault data (pages, links, backlinks, frontmatter, stats) is passed to a Jinja2-compatible template engine, which renders it into HTML. The current UI becomes the **default theme** — shipped embedded in the binary — which users can override partially or fully by placing template files in their vault's `.zetl/templates/` directory.

### 1.1 Core Insight

A static site generator without user-customisable templates is not a static site generator — it's just an HTML exporter. The value of zetl's `serve` and `build` commands is not the specific HTML they produce, but the structured vault data they expose: resolved wikilinks, backlink graphs, transclusion cards, search indices, breadcrumbs. Separating data from presentation unlocks the entire design space: screenplay vaults, documentation sites, digital gardens, research wikis — each with distinct layouts, scripts, and styles — all from the same underlying vault.

### 1.2 Design Philosophy

1. **Data, not markup.** Templates receive structured, serializable context objects. The template decides what to render and how. zetl's job is to provide complete, well-typed data — not HTML fragments.
2. **Override partially, inherit the rest.** Template inheritance (`{% extends %}`, `{% block %}`) means a user can override a single block (e.g., add a `<script>` tag to the head) without rewriting the entire layout. Overriding `page.html` while inheriting the default `base.html` is a first-class use case.
3. **Zero-config default.** With no `.zetl/templates/` directory, the output is pixel-identical to the current hardcoded HTML. The template engine is invisible until the user opts in.
4. **Static assets are first-class.** A `.zetl/static/` directory is served verbatim (in `serve` mode) or copied to the output (in `build` mode). Templates can reference these assets via `/_static/` paths. No build toolchain, no bundler — just files.
5. **Frontmatter is structured data.** YAML frontmatter is parsed into a key-value object accessible in templates. This enables content-type-aware rendering: `{% if page.frontmatter.format == "fountain" %}` to load a screenplay renderer.

### 1.3 Scope

**In scope:**

- Template engine integration (Minijinja) for `serve` and `build` HTML generation
- Default theme templates extracted from current hardcoded HTML (pixel-identical output)
- User template overrides from `.zetl/templates/` with fallback to built-in defaults
- Template inheritance (`{% extends %}`, `{% block %}`, `{% include %}`)
- Structured template context: vault metadata, page content, links, backlinks, stats, breadcrumbs
- Static asset serving from `.zetl/static/` at `/_static/` in serve mode
- Static asset copying to output directory in build mode
- YAML frontmatter parsing into structured template data
- Mode-aware rendering (`serve` vs `build`) so templates can include/exclude edit UI

**Out of scope:**

- Theme packaging or distribution (future SPEC — theme marketplace, `zetl theme install`)
- Template hot-reload in serve mode (future enhancement — requires file watcher integration with SPEC-008)
- Sass/Less/TypeScript compilation (users bring pre-compiled assets)
- Server-side includes or dynamic data fetching in templates
- Custom Minijinja filters or functions beyond the built-in set (future SPEC)
- RSS/Atom feed generation (future SPEC — natural extension once templates exist)
- Multi-vault theming (each vault has its own `.zetl/templates/`)

---

## 2. User Profiles

### 2.1 Digital Gardener

```
Role: Knowledge worker publishing a personal wiki / digital garden
Goals:
  - Customise the look and feel of their published vault
  - Add a custom stylesheet and fonts
  - Include analytics (Plausible, Umami) via a script tag
  - Change the sidebar layout or add a footer
Constraints:
  - Comfortable with HTML/CSS, basic Jinja2 syntax
  - Does not want to fork or recompile zetl
  - Wants changes to take effect immediately on `zetl serve`
Daily workflow:
  1. Create `.zetl/templates/base.html` to add analytics script
  2. Run `zetl serve` — see changes reflected in the browser
  3. Run `zetl build` — static site includes the custom template
  4. Deploy the `dist/` directory to a static host
```

### 2.2 Screenplay Author

```
Role: Writer using Fountain markup in Markdown files
Goals:
  - Render pages with `format: fountain` frontmatter using Fountain.js
  - Keep non-screenplay pages rendering with the default theme
  - Load Fountain.js from `.zetl/static/fountain.js`
Constraints:
  - Needs conditional template logic based on frontmatter
  - Fountain.js is a client-side library loaded via <script> tag
  - Does not want to modify non-screenplay page rendering
Daily workflow:
  1. Add `format: fountain` to screenplay page frontmatter
  2. Place `fountain.js` in `.zetl/static/`
  3. Create `.zetl/templates/page.html` with conditional Fountain loading
  4. Run `zetl serve` — screenplay pages render with Fountain.js
```

### 2.3 Documentation Engineer

```
Role: Technical writer maintaining project documentation as a zetl vault
Goals:
  - Apply corporate branding (logo, colours, fonts)
  - Add navigation breadcrumbs with custom styling
  - Include versioned documentation links in the sidebar
  - Generate a static documentation site for hosting
Constraints:
  - Must match corporate style guide
  - Needs full control over base layout
  - Cannot use CDN-hosted CSS (air-gapped environment)
  - Pre-compiled CSS/JS assets placed in `.zetl/static/`
Daily workflow:
  1. Override `base.html` with corporate layout
  2. Place brand CSS/JS in `.zetl/static/`
  3. Use `zetl build -o docs/` to generate static site
  4. Commit `docs/` to repository for GitHub Pages deployment
```

### 2.4 Agent Operator

```
Role: AI agent building and publishing a knowledge base
Goals:
  - Programmatically generate templates for custom vault rendering
  - Write frontmatter-driven conditional templates
  - Use `zetl build` output for downstream processing
Constraints:
  - Requires predictable, documented template context (JSON-serializable)
  - Must know exactly what variables are available in each template
  - Needs structured error messages when templates fail to render
Daily workflow:
  1. Write `.zetl/templates/page.html` with structured data access
  2. Run `zetl build` and verify output programmatically
  3. Inspect template errors from structured CLI output
```

---

## 3. Architecture

### 3.1 ADR-012: Template Engine — Minijinja

```
ADR-012: Template Engine — Minijinja

Status: Proposed

Context:
  zetl's serve and build commands generate HTML via Rust format!()
  string concatenation. To support user-customisable templates, a
  template engine is needed. Two mature Jinja2-compatible engines
  exist in the Rust ecosystem:

  A. Tera — established (used by Zola), feature-rich, larger dependency
  B. Minijinja — by Armin Ronacher (creator of Jinja2), lighter, more
     correct to Jinja2 semantics

Decision:
  Use Minijinja.

  Rationale:
  - Written by the creator of Jinja2 — canonical semantics, not an
    approximation. Template syntax matches Jinja2/Nunjucks/Django
    exactly, so existing knowledge transfers directly.
  - Lighter dependency footprint than Tera (~faster compile times).
    zetl's lean dependency set is a design value.
  - Supports template inheritance (extends, block), includes, filters,
    macros — all required for the override-partially pattern.
  - Built-in source/loader feature for dynamic template loading with
    fallback — directly supports the "check disk, fall back to
    built-in" pattern.
  - Actively maintained, good error messages with line numbers.

Consequences:
  + Canonical Jinja2 syntax — largest possible user knowledge base
  + Smaller binary size and faster compile times than Tera
  + Built-in loader abstraction simplifies fallback logic
  + Excellent error messages aid user debugging
  - Tera is more established in the Rust ecosystem (Zola uses it)
  - Fewer community-contributed filters than Tera (mitigated by
    Minijinja's built-in filter set being sufficient for HTML templates)
```

### 3.2 Template Resolution Architecture

```
Template Loading Order:

  1. Check .zetl/templates/<name>.html on disk
  2. If not found, fall back to built-in default (include_str!)
  3. Template inheritance resolves across both sources:
     - User page.html can {% extends "base.html" %} where base.html
       is the built-in default
     - User base.html overrides the entire shell; built-in page.html
       still works if it only uses blocks defined in base.html

Implementation:
  Minijinja's Environment::set_loader() accepts a closure that
  returns Option<String>. The closure:
    fn load(name: &str) -> Option<String> {
        // 1. Check disk
        let disk_path = vault_root.join(".zetl/templates").join(name);
        if disk_path.exists() {
            return Some(fs::read_to_string(disk_path).ok()?);
        }
        // 2. Fall back to built-in
        match name {
            "base.html"   => Some(include_str!("templates/base.html")),
            "index.html"  => Some(include_str!("templates/index.html")),
            "page.html"   => Some(include_str!("templates/page.html")),
            "folder.html" => Some(include_str!("templates/folder.html")),
            _ => None,
        }
    }
```

### 3.3 Template File Structure

```
.zetl/
  templates/           # user overrides (all optional)
    base.html          # master layout — blocks: head, styles, content, sidebar, scripts
    index.html         # vault landing page ({% extends "base.html" %})
    page.html          # single page view ({% extends "base.html" %})
    folder.html        # folder index   ({% extends "base.html" %})
  static/              # served verbatim at /_static/ (serve) or copied to dist/_static/ (build)
    fountain.js
    custom.css
    logo.png
```

### 3.4 Template Data Context

Serializable Rust structs are converted to `minijinja::Value` via Serde. Each template type receives a context object with the following shape:

#### All templates receive:

```
vault.name              : String       — vault root directory name
vault.pages[]           : [{title, slug, outlink_count, backlink_count}]
vault.stats.total_pages : usize
vault.stats.total_links : usize
vault.stats.dead_links  : usize
vault.stats.orphans     : usize
search_index            : String       — JSON array for client-side fuzzy search
zetl.version            : String
```

#### `page.html` additionally receives:

```
page.title              : String
page.slug               : String
page.content_html       : String       — rendered markdown (wikilinks resolved)
page.content_raw        : String       — raw markdown source
page.frontmatter        : Object       — parsed YAML frontmatter as key/value pairs
page.backlinks[]        : [{title, slug, line}]
page.outlinks[]         : [{title, slug, is_dead, color}]
page.breadcrumbs[]      : [{title, slug}]
page.transclusion_cards : String       — pre-rendered transclusion HTML (for default theme)
page.is_new             : bool         — true if page doesn't exist yet (edit mode)
page.raw_escaped        : String|null  — HTML-escaped raw content for edit textarea
mode                    : "serve"|"build" — allows templates to include/exclude edit UI
```

#### `index.html` additionally receives:

```
vault.pages[]  (as above, with link counts)
```

#### `folder.html` additionally receives:

```
folder.name             : String
folder.slug             : String
folder.breadcrumbs[]    : [{title, slug}]
folder.subfolders[]     : [{name, slug, page_count}]
folder.pages[]          : [{title, slug, outlink_count, backlink_count}]
folder.total_pages      : usize
```

### 3.5 Context Struct Design

```rust
use serde::Serialize;

#[derive(Serialize)]
pub struct VaultContext {
    pub name: String,
    pub pages: Vec<PageEntry>,
    pub stats: StatsContext,
}

#[derive(Serialize)]
pub struct PageEntry {
    pub title: String,
    pub slug: String,
    pub outlink_count: usize,
    pub backlink_count: usize,
}

#[derive(Serialize)]
pub struct StatsContext {
    pub total_pages: usize,
    pub total_links: usize,
    pub dead_links: usize,
    pub orphans: usize,
}

#[derive(Serialize)]
pub struct ZetlMeta {
    pub version: String,
}

#[derive(Serialize)]
pub struct BreadcrumbEntry {
    pub title: String,
    pub slug: String,
}

#[derive(Serialize)]
pub struct BacklinkEntry {
    pub title: String,
    pub slug: String,
    pub line: usize,
}

#[derive(Serialize)]
pub struct OutlinkEntry {
    pub title: String,
    pub slug: String,
    pub is_dead: bool,
    pub color: String,
}

#[derive(Serialize)]
pub struct PageContext {
    pub title: String,
    pub slug: String,
    pub content_html: String,
    pub content_raw: String,
    pub frontmatter: serde_json::Value,
    pub backlinks: Vec<BacklinkEntry>,
    pub outlinks: Vec<OutlinkEntry>,
    pub breadcrumbs: Vec<BreadcrumbEntry>,
    pub transclusion_cards: String,
    pub is_new: bool,
    pub raw_escaped: Option<String>,
}

#[derive(Serialize)]
pub struct FolderContext {
    pub name: String,
    pub slug: String,
    pub breadcrumbs: Vec<BreadcrumbEntry>,
    pub subfolders: Vec<SubfolderEntry>,
    pub pages: Vec<PageEntry>,
    pub total_pages: usize,
}

#[derive(Serialize)]
pub struct SubfolderEntry {
    pub name: String,
    pub slug: String,
    pub page_count: usize,
}
```

### 3.6 Component Diagram

```
                          ┌──────────────────────────┐
                          │       User vault          │
                          │                           │
                          │  .zetl/                   │
                          │    templates/             │
                          │      page.html (optional) │
                          │    static/                │
                          │      fountain.js          │
                          │      custom.css           │
                          │                           │
                          │  notes/*.md               │
                          └──────────┬───────────────┘
                                     │
                                     ▼
┌────────────────────────────────────────────────────────────────┐
│                        zetl binary                             │
│                                                                │
│  ┌──────────┐    ┌──────────────┐    ┌──────────────────────┐ │
│  │ Scanner  │───►│  VaultData   │───►│  Context Builders    │ │
│  │ (index)  │    │ (graph,files)│    │  (context.rs)        │ │
│  └──────────┘    └──────────────┘    │                      │ │
│                                       │  VaultData → struct  │ │
│                                       │  PageContext         │ │
│                                       │  FolderContext       │ │
│                                       │  IndexContext        │ │
│                                       └──────────┬───────────┘ │
│                                                  │             │
│  ┌───────────────────────────────────────────────┼───────────┐ │
│  │              Template Engine (engine.rs)       │           │ │
│  │                                               ▼           │ │
│  │  ┌─────────────────────────────────────────────────────┐  │ │
│  │  │           Minijinja Environment                     │  │ │
│  │  │                                                     │  │ │
│  │  │  Loader:                                            │  │ │
│  │  │    1. .zetl/templates/<name>.html (disk)            │  │ │
│  │  │    2. Built-in defaults (include_str!)              │  │ │
│  │  │                                                     │  │ │
│  │  │  render("index.html", context) → String             │  │ │
│  │  │  render("page.html",  context) → String             │  │ │
│  │  │  render("folder.html", context) → String            │  │ │
│  │  └─────────────────────────────────────────────────────┘  │ │
│  └───────────────────────────────────────────────────────────┘ │
│                          │                                     │
│              ┌───────────┴───────────┐                         │
│              ▼                       ▼                         │
│  ┌──────────────────┐   ┌──────────────────────┐              │
│  │   serve (axum)   │   │   build (static gen)  │              │
│  │                  │   │                        │              │
│  │  Routes call     │   │  Iterates pages,       │              │
│  │  engine.render() │   │  calls engine.render() │              │
│  │                  │   │  writes HTML files      │              │
│  │  /_static/ →     │   │                        │              │
│  │  ServeDir from   │   │  Copies .zetl/static/  │              │
│  │  .zetl/static/   │   │  to dist/_static/      │              │
│  └──────────────────┘   └──────────────────────┘              │
│                                                                │
│  ┌─────────────────────────────────────────────────────┐      │
│  │  Built-in Default Templates (embedded via           │      │
│  │  include_str!)                                      │      │
│  │                                                     │      │
│  │  src/web/templates/                                 │      │
│  │    base.html   — DaisyUI shell, search modal,       │      │
│  │                  sidebar, responsive drawer          │      │
│  │    index.html  — vault landing page, stats grid      │      │
│  │    page.html   — page view, backlinks, transclusion  │      │
│  │    folder.html — folder index, subfolder/page cards  │      │
│  └─────────────────────────────────────────────────────┘      │
└────────────────────────────────────────────────────────────────┘
```

### 3.7 ADR-013: Static Asset Serving Strategy

```
ADR-013: Static Asset Serving — Direct File Serving

Status: Proposed

Context:
  User templates need to reference custom JS, CSS, images, and fonts.
  Two approaches were considered:

  A. Serve .zetl/static/ at /_static/ using tower-http ServeDir
     (or a manual handler in axum)
  B. Embed static assets in templates via data URIs or inline <style>/<script>

Decision:
  Option A — serve from .zetl/static/ at /_static/.

  Rationale:
  - Files are served/copied as-is with correct MIME types
  - No size limits (data URIs bloat HTML for large assets)
  - Browser caching works naturally
  - Build mode copies the directory to dist/_static/ (simple cp -r)
  - Matches the convention of every static site generator (Hugo, Jekyll,
    Zola, 11ty)

Consequences:
  + Simple mental model: put a file in .zetl/static/, reference it as
    /_static/filename in templates
  + No transformation or bundling step
  + Works identically in serve and build modes
  - Requires an additional route in the axum server
  - tower-http fs feature adds a small dependency (or a manual handler
    avoids the dependency)
```

### 3.8 ADR-014: Frontmatter Parsing

```
ADR-014: Frontmatter Parsing — serde_yaml to JSON Value

Status: Proposed

Context:
  zetl already strips YAML frontmatter from Markdown content before
  rendering (strip_frontmatter in markdown.rs). However, the frontmatter
  data itself is discarded. Templates need access to frontmatter as
  structured data for conditional rendering.

  Options:
  A. Parse YAML into serde_json::Value (generic key-value)
  B. Parse YAML into a typed Rust struct (fixed schema)
  C. Pass raw YAML string to templates

Decision:
  Option A — parse to serde_json::Value.

  Rationale:
  - Frontmatter schemas vary per vault and per page. A typed struct
    would require zetl to anticipate all possible frontmatter fields.
  - serde_json::Value maps naturally to Minijinja's Value type
  - Templates access fields dynamically: page.frontmatter.format,
    page.frontmatter.tags, page.frontmatter.author
  - serde_yaml can deserialize directly to serde_json::Value

Consequences:
  + Fully flexible — any YAML frontmatter is accessible
  + No schema maintenance burden on zetl
  + Natural mapping to template variable access
  - No compile-time type checking on frontmatter access (template errors
    at render time, not compile time — but this is inherent to templates)
  - Adds serde_yaml dependency
```

---

## 4. Requirements

### 4.1 Functional Requirements

```
REQ-012-001: Template Engine Integration

The system SHALL use Minijinja as the template engine for rendering
HTML in both `serve` and `build` commands.

The template engine SHALL support:
  a) Template inheritance ({% extends %}, {% block %})
  b) Template includes ({% include %})
  c) Conditional logic ({% if %}, {% elif %}, {% else %})
  d) Iteration ({% for %})
  e) Filters (e.g., {{ title|upper }})
  f) Macros ({% macro %})

FOR all user roles
WITH Jinja2-compatible syntax

Trace:
  - TEST-012-001
  - ADR-012
```

```
REQ-012-002: Default Theme

The system SHALL ship a complete set of default templates embedded in
the binary via include_str!. These templates SHALL produce HTML output
that is pixel-identical to the current hardcoded HTML generation in
html.rs, routes.rs, and build.rs.

The default templates SHALL consist of:
  a) base.html — master layout with blocks: head, styles, content, sidebar, scripts
  b) index.html — vault landing page
  c) page.html — single page view with backlinks and transclusion
  d) folder.html — folder index with subfolder and page cards

FOR all user roles
WITH zero-config default behaviour (no .zetl/templates/ directory required)

Trace:
  - TEST-012-002
```

```
REQ-012-003: User Template Override

The system SHALL load user templates from .zetl/templates/ when present,
falling back to built-in defaults for any template not overridden.

The loading order SHALL be:
  a) Check .zetl/templates/<name>.html on disk
  b) If not found, use the built-in default

Template inheritance SHALL work across both sources: a user page.html
can extend the built-in base.html, and vice versa.

FOR all user roles
WITH immediate effect on next `zetl serve` request or `zetl build` run

Trace:
  - TEST-012-003
  - CON-012-001
```

```
REQ-012-004: Template Data Context

The system SHALL provide structured, serializable context objects to
each template, containing:
  a) Vault metadata: name, page list with link counts, stats
  b) Search index: JSON array for client-side fuzzy search
  c) zetl version string
  d) Template-specific data as defined in §3.4

Context objects SHALL be Serde-serializable Rust structs converted to
Minijinja values.

FOR all user roles
WITH complete data coverage — all information currently used by the
hardcoded HTML generation SHALL be available in the context

Trace:
  - TEST-012-004
```

```
REQ-012-005: Static Asset Serving (Serve Mode)

The system SHALL serve files from .zetl/static/ at the URL path
/_static/ during `zetl serve`.

Files SHALL be served with correct MIME types inferred from file
extension.

If .zetl/static/ does not exist, the /_static/ route SHALL return
404 for all requests (no error on startup).

FOR all user roles

Trace:
  - TEST-012-005
  - CON-012-002
  - ADR-013
```

```
REQ-012-006: Static Asset Copying (Build Mode)

The system SHALL copy the contents of .zetl/static/ to
{out_dir}/_static/ during `zetl build`.

If .zetl/static/ does not exist, no _static/ directory SHALL be
created in the output.

The copy SHALL preserve the directory structure within .zetl/static/.

FOR all user roles

Trace:
  - TEST-012-006
  - CON-012-003
  - ADR-013
```

```
REQ-012-007: YAML Frontmatter Parsing

The system SHALL parse YAML frontmatter (content between `---` fences
at the start of a Markdown file) into a structured key-value object
accessible in templates as page.frontmatter.

Frontmatter parsing SHALL:
  a) Parse valid YAML into serde_json::Value
  b) Return an empty object ({}) for pages with no frontmatter
  c) Return an empty object ({}) for pages with malformed YAML
     (with a warning logged)
  d) Not alter the existing strip_frontmatter behaviour for
     markdown rendering

FOR all user roles

Trace:
  - TEST-012-007
  - ADR-014
```

```
REQ-012-008: Mode-Aware Rendering

The system SHALL pass a `mode` variable to all templates with the
value "serve" or "build", allowing templates to conditionally
include or exclude content based on the rendering mode.

The default page.html template SHALL use this variable to include
edit UI (edit button, save handler, JavaScript) only in serve mode.

FOR all user roles

Trace:
  - TEST-012-008
```

```
REQ-012-009: Template Error Reporting

When a template fails to render (syntax error, missing variable,
inheritance error), the system SHALL:
  a) In serve mode: return an HTML error page with the error message,
     template name, and line number
  b) In build mode: print the error to stderr with template name and
     line number, and exit with a non-zero exit code
  c) Never silently produce empty or partial HTML output

FOR all user roles

Trace:
  - TEST-012-009
  - OBS-012-001
```

### 4.2 Non-Functional Requirements

```
NFR-012-001: Rendering Performance

Template rendering SHALL add no more than 5ms per page compared to the
current format!() string concatenation, for vaults with ≤ 10,000 pages.

The template engine SHALL be initialized once and reused across all
render calls within a single serve session or build run.
```

```
NFR-012-002: Binary Size Impact

The Minijinja dependency SHALL add no more than 500KB to the compiled
binary size (release build, stripped).
```

```
NFR-012-003: Backward Compatibility

With no .zetl/templates/ directory present, the output of `zetl serve`
and `zetl build` SHALL be identical to the output produced by the
current hardcoded HTML generation. This is a hard requirement for
Phase 1 — no visual regressions.
```

```
NFR-012-004: Template Syntax Compatibility

Template syntax SHALL be compatible with Jinja2, Nunjucks, and Django
template syntax. Users familiar with any of these systems SHALL be
able to write zetl templates without learning a new syntax.
```

---

## 5. Contract Specifications

```
CON-012-001: Template Override Convention

User templates are placed in .zetl/templates/ relative to the vault
root. The following template names are recognized:

  base.html    — master layout (defines blocks)
  index.html   — vault landing page (extends base.html)
  page.html    — single page view (extends base.html)
  folder.html  — folder index (extends base.html)

Any other .html files in .zetl/templates/ can be used via
{% include "filename.html" %} from within recognized templates.

Template loading precedence:
  1. .zetl/templates/<name>.html (user override)
  2. Built-in default (embedded in binary)

Template inheritance works across both sources. A user page.html
can {% extends "base.html" %} where base.html resolves to the
built-in default if the user has not overridden it.

Example: Override only the page template

  .zetl/templates/page.html:
    {% extends "base.html" %}
    {% block content %}
      <h1>{{ page.title }}</h1>
      {{ page.content_html|safe }}
      {% if page.frontmatter.format == "fountain" %}
        <script src="/_static/fountain.js"></script>
      {% endif %}
    {% endblock %}

Implements: REQ-012-003
Verified by: TEST-012-003
```

```
CON-012-002: Static Asset URL Convention (Serve)

Files placed in .zetl/static/ are served at /_static/ during
`zetl serve`.

  .zetl/static/fountain.js  →  GET /_static/fountain.js
  .zetl/static/css/main.css →  GET /_static/css/main.css
  .zetl/static/img/logo.png →  GET /_static/img/logo.png

MIME types are inferred from file extension:
  .js   → application/javascript
  .css  → text/css
  .png  → image/png
  .jpg  → image/jpeg
  .svg  → image/svg+xml
  .woff2 → font/woff2

If .zetl/static/ does not exist, all /_static/* requests return 404.

Implements: REQ-012-005
Verified by: TEST-012-005
```

```
CON-012-003: Static Asset Output Convention (Build)

During `zetl build`, if .zetl/static/ exists, its contents are copied
to {out_dir}/_static/ preserving directory structure.

Example:
  .zetl/static/fountain.js  →  dist/_static/fountain.js
  .zetl/static/css/main.css →  dist/_static/css/main.css

If .zetl/static/ does not exist, no _static/ directory is created
in the output.

Implements: REQ-012-006
Verified by: TEST-012-006
```

```
CON-012-004: Template Context Variables

Templates receive the following variables. All values are
JSON-serializable.

Global context (available in all templates):
  vault          : object
    .name        : string  — vault root directory name
    .pages[]     : array of {title, slug, outlink_count, backlink_count}
    .stats       : object
      .total_pages : integer
      .total_links : integer
      .dead_links  : integer
      .orphans     : integer
  search_index   : string  — JSON array for client-side search
  zetl           : object
    .version     : string

page.html additional context:
  page           : object
    .title       : string
    .slug        : string
    .content_html: string  — rendered HTML (wikilinks resolved)
    .content_raw : string  — raw markdown source
    .frontmatter : object  — parsed YAML frontmatter (arbitrary keys)
    .backlinks[] : array of {title, slug, line}
    .outlinks[]  : array of {title, slug, is_dead, color}
    .breadcrumbs[]: array of {title, slug}
    .transclusion_cards : string — pre-rendered HTML
    .is_new      : boolean
    .raw_escaped : string|null
  mode           : "serve" | "build"

folder.html additional context:
  folder         : object
    .name        : string
    .slug        : string
    .breadcrumbs[]: array of {title, slug}
    .subfolders[]: array of {name, slug, page_count}
    .pages[]     : array of {title, slug, outlink_count, backlink_count}
    .total_pages : integer

Implements: REQ-012-004
Verified by: TEST-012-004
```

---

## 6. Test Specifications

```
TEST-012-001: Template Engine Renders Default Templates

Scenario: Minijinja renders all built-in templates without error
Given: A vault with at least 3 pages and 1 subfolder
When: zetl build is run with no .zetl/templates/ directory
Then:
  - All pages render to HTML without template errors
  - index.html is generated at the output root
  - Each page generates a {slug}/index.html file
  - Folder indices are generated for each subfolder
  - Exit code is 0

Verifies: REQ-012-001
```

```
TEST-012-002: Default Theme Output Matches Current HTML

Scenario: Template-rendered output is identical to hardcoded output
Given: A vault with pages, backlinks, dead links, and subfolders
When: zetl build is run with default templates (no .zetl/templates/)
Then:
  - The generated HTML contains the same DaisyUI layout structure
  - The search modal is present with the search index embedded
  - Sidebar navigation lists all pages
  - Page views include backlinks and transclusion panel
  - Folder indices show subfolder and page cards
  - All inline CSS and JavaScript is present

Verifies: REQ-012-002
```

```
TEST-012-003: User Template Override with Fallback

Scenario: User overrides page.html while base.html falls back to default
Given: A vault with .zetl/templates/page.html containing:
       {% extends "base.html" %}
       {% block content %}
       <div class="custom-banner">Custom!</div>
       {{ page.content_html|safe }}
       {% endblock %}
When: zetl serve is running and a page is requested
Then:
  - The response HTML contains <div class="custom-banner">Custom!</div>
  - The response HTML contains the default base.html layout (DaisyUI shell)
  - The sidebar, search modal, and other base.html elements are present

Scenario: User overrides base.html
Given: A vault with .zetl/templates/base.html containing a minimal layout
When: zetl build is run
Then:
  - All pages use the user's base.html layout
  - Built-in page.html, index.html, folder.html still render correctly
    within the user's base layout (assuming block names match)

Scenario: No .zetl/templates/ directory
Given: A vault with no .zetl/ directory or no templates/ subdirectory
When: zetl serve or zetl build is run
Then:
  - Built-in defaults are used for all templates
  - No errors or warnings about missing templates

Verifies: REQ-012-003
```

```
TEST-012-004: Template Context Contains All Required Data

Scenario: Page template receives complete context
Given: A vault with a page "Test Page" that has:
       - 3 outgoing wikilinks (1 dead)
       - 2 backlinks
       - YAML frontmatter with {format: "fountain", tags: ["drama"]}
When: The page is rendered via zetl serve
Then:
  - page.title == "Test Page"
  - page.slug contains the kebab-case slug
  - page.content_html contains rendered HTML with resolved wikilinks
  - page.content_raw contains the raw markdown source
  - page.frontmatter.format == "fountain"
  - page.frontmatter.tags == ["drama"]
  - page.backlinks has 2 entries with title, slug, and line fields
  - page.outlinks has 3 entries, 1 with is_dead == true
  - page.breadcrumbs is populated for nested pages
  - vault.stats.total_pages reflects the vault size
  - vault.stats.dead_links >= 1
  - search_index is a valid JSON array string
  - zetl.version matches the binary version

Verifies: REQ-012-004
```

```
TEST-012-005: Static Asset Serving in Serve Mode

Scenario: Static files are served at /_static/
Given: A vault with .zetl/static/test.js containing "console.log('ok')"
       and .zetl/static/css/style.css containing "body { color: red; }"
When: zetl serve is running
Then:
  - GET /_static/test.js returns 200 with content-type application/javascript
  - GET /_static/css/style.css returns 200 with content-type text/css
  - Response bodies match file contents
  - GET /_static/nonexistent.js returns 404

Scenario: No static directory
Given: A vault with no .zetl/static/ directory
When: zetl serve is running
Then:
  - GET /_static/anything returns 404
  - No error on server startup

Verifies: REQ-012-005
```

```
TEST-012-006: Static Asset Copying in Build Mode

Scenario: Static files are copied to output
Given: A vault with .zetl/static/app.js and .zetl/static/css/main.css
When: zetl build -o dist is run
Then:
  - dist/_static/app.js exists with matching content
  - dist/_static/css/main.css exists with matching content

Scenario: No static directory
Given: A vault with no .zetl/static/ directory
When: zetl build -o dist is run
Then:
  - dist/_static/ does not exist
  - Build completes successfully

Verifies: REQ-012-006
```

```
TEST-012-007: Frontmatter Parsing

Scenario: Valid YAML frontmatter is parsed
Given: A page with frontmatter:
       ---
       format: fountain
       tags:
         - drama
         - screenplay
       author: "Jane Doe"
       ---
When: The page is rendered
Then:
  - page.frontmatter.format == "fountain"
  - page.frontmatter.tags == ["drama", "screenplay"]
  - page.frontmatter.author == "Jane Doe"

Scenario: No frontmatter
Given: A page with no --- fences at the start
When: The page is rendered
Then:
  - page.frontmatter is an empty object ({})

Scenario: Malformed YAML
Given: A page with frontmatter containing invalid YAML:
       ---
       key: [unclosed
       ---
When: The page is rendered
Then:
  - page.frontmatter is an empty object ({})
  - A warning is logged (not an error — page still renders)

Verifies: REQ-012-007
```

```
TEST-012-008: Mode-Aware Rendering

Scenario: Serve mode includes edit UI
Given: Default templates are in use
When: A page is rendered in serve mode
Then:
  - mode == "serve"
  - The HTML contains the edit button and save JavaScript

Scenario: Build mode excludes edit UI
Given: Default templates are in use
When: A page is rendered in build mode
Then:
  - mode == "build"
  - The HTML does not contain the edit button or save JavaScript

Verifies: REQ-012-008
```

```
TEST-012-009: Template Error Reporting

Scenario: Syntax error in user template (serve mode)
Given: .zetl/templates/page.html contains {% if unclosed %}
When: A page is requested via zetl serve
Then:
  - The response is an HTML error page (not a 500 with no body)
  - The error message includes "page.html" and a line number
  - The error message describes the syntax issue

Scenario: Syntax error in user template (build mode)
Given: .zetl/templates/page.html contains {{ undefined_var.deep.access }}
When: zetl build is run
Then:
  - stderr contains the error with template name and line number
  - Exit code is non-zero
  - No partial HTML files are left in an inconsistent state

Verifies: REQ-012-009
```

---

## 7. Observability

```
OBS-012-001: Template Error Logging

Template rendering errors SHALL be logged with:
  - Template name (e.g., "page.html")
  - Line number within the template
  - Error description (syntax error, undefined variable, etc.)
  - The page/slug being rendered when the error occurred

In serve mode, errors SHALL be logged to stderr AND returned as an
HTML error page in the response.

In build mode, errors SHALL be logged to stderr and cause a non-zero
exit code.
```

```
OBS-012-002: Template Loading Diagnostics

When --verbose is set, the system SHALL log:
  - Which templates were loaded from disk (.zetl/templates/)
  - Which templates fell back to built-in defaults
  - The vault root path used for template resolution

Example output:
  [zetl] template "page.html" loaded from .zetl/templates/page.html
  [zetl] template "base.html" using built-in default
  [zetl] template "index.html" using built-in default
  [zetl] template "folder.html" using built-in default
```

---

## 8. Default Template Block Structure

The default `base.html` template defines the following blocks that users can override:

```
{% block head %}        — <head> contents: meta tags, title, CDN links
{% block styles %}      — additional <style> or <link> tags
{% block sidebar %}     — sidebar navigation content
{% block content %}     — main content area
{% block scripts %}     — <script> tags before </body>
```

Example: Adding analytics without rewriting the layout

```jinja2
{# .zetl/templates/base.html #}
{% extends "base.html" %}

{% block scripts %}
  {{ super() }}
  <script defer data-domain="example.com" src="https://plausible.io/js/script.js"></script>
{% endblock %}
```

Note: The above pattern requires Minijinja's support for `{{ super() }}` within blocks, which allows appending to a parent block rather than replacing it entirely.

---

## 9. Phased Implementation

### Phase 1: Refactor — Extract templates (zero behaviour change)

**Goal:** Replace hardcoded HTML string building with template rendering. The current UI remains pixel-identical.

**Files created:**
- `src/web/templates/base.html` — extracted from `html.rs::layout()`
- `src/web/templates/index.html` — extracted from `routes.rs::index_handler()` + `build.rs::render_index()`
- `src/web/templates/page.html` — extracted from `routes.rs::page_handler()` + `build.rs::render_page()`
- `src/web/templates/folder.html` — extracted from `routes.rs::render_folder_index()` + `build.rs::render_folder_index()`
- `src/web/engine.rs` — `TemplateEngine` struct wrapping `minijinja::Environment`
- `src/web/context.rs` — serializable context structs

**Files modified:**
- `Cargo.toml` — add `minijinja` dependency (with `source` feature)
- `src/web/mod.rs` — add `pub mod engine; pub mod context;`, store `TemplateEngine` in `WebState`
- `src/web/routes.rs` — replace `format!()` HTML building with `state.engine.render_page(...)`
- `src/web/build.rs` — replace `format!()` HTML building with `engine.render_page(...)`
- `src/web/html.rs` — most functions become unused (remove `sidebar_html`, `layout`; keep `urlencoding`, `html_escape`, `search_index_json`)

**Key principle:** The default templates produce the same HTML as the current Rust code.

**Verification:** `cargo build` succeeds. `zetl serve` and `zetl build` produce identical HTML. Existing tests pass.

### Phase 2: User template override

**Goal:** Load user templates from `.zetl/templates/` with fallback to built-in defaults.

**Files modified:**
- `src/web/engine.rs` — extend `TemplateEngine::new()` to accept `vault_root` path; use `set_loader()` with disk-then-builtin fallback
- `src/web/mod.rs` — pass `vault_root` to `TemplateEngine` constructor

**Verification:** Drop a `page.html` in `.zetl/templates/`, verify it renders. Remove it, verify fallback to default.

### Phase 3: Static asset serving

**Goal:** Serve user files from `.zetl/static/` at `/_static/`.

**Files modified:**
- `src/web/mod.rs` — add `/_static/` route serving from `.zetl/static/`
- `src/web/build.rs` — copy `.zetl/static/` to `{out_dir}/_static/` after HTML generation
- `Cargo.toml` — optionally add `tower-http` with `fs` feature, or implement a manual static file handler

**Verification:** Place a JS file in `.zetl/static/`, verify it's served and copied.

### Phase 4: Frontmatter parsing

**Goal:** Parse YAML frontmatter into structured template data.

**Files modified:**
- `Cargo.toml` — add `serde_yaml`
- `src/web/markdown.rs` — add `parse_frontmatter(content: &str) -> serde_json::Value`
- `src/web/context.rs` — populate `PageContext.frontmatter` from parsed frontmatter

**Verification:** Create a page with `format: fountain` in frontmatter, verify `page.frontmatter.format` is accessible in templates.

---

## 10. Traceability Matrix

| Requirement    | Contract(s)   | Test(s)        | ADR(s)  | OBS        |
| -------------- | ------------- | -------------- | ------- | ---------- |
| REQ-012-001    | —             | TEST-012-001   | ADR-012 | —          |
| REQ-012-002    | —             | TEST-012-002   | —       | —          |
| REQ-012-003    | CON-012-001   | TEST-012-003   | —       | OBS-012-002|
| REQ-012-004    | CON-012-004   | TEST-012-004   | —       | —          |
| REQ-012-005    | CON-012-002   | TEST-012-005   | ADR-013 | —          |
| REQ-012-006    | CON-012-003   | TEST-012-006   | ADR-013 | —          |
| REQ-012-007    | —             | TEST-012-007   | ADR-014 | —          |
| REQ-012-008    | —             | TEST-012-008   | —       | —          |
| REQ-012-009    | —             | TEST-012-009   | —       | OBS-012-001|

---

## 11. Open Questions

1. **Should template hot-reload be supported in serve mode?**
   If a user edits `.zetl/templates/page.html` while `zetl serve` is running, should the next request pick up the change automatically? This requires either re-creating the Minijinja environment on each request (simple, slight performance cost) or integrating with SPEC-008's file watcher to invalidate the template cache. Recommendation: reload on every request in serve mode (templates are small, parsing is fast); cache in build mode.

2. **Should custom Minijinja filters be supported?**
   Users might want filters like `{{ page.content_raw|wordcount }}` or `{{ page.title|slugify }}`. Minijinja supports registering custom filters. This could be a future SPEC or a built-in set of zetl-specific filters. Recommendation: defer to a future SPEC; the built-in Minijinja filter set (upper, lower, trim, length, join, etc.) is sufficient for the initial release.

3. **Should the default theme be extractable?**
   A `zetl theme export` command could write the built-in templates to `.zetl/templates/` as a starting point for customisation. This is convenient but could be confusing (user now has a copy that diverges from updates). Recommendation: document that users can copy from the source repository; a dedicated command is a future enhancement.

4. **How should template errors interact with the build process?**
   If one page fails to render, should `zetl build` abort immediately or continue rendering other pages and report errors at the end? Recommendation: fail fast — a partial build is worse than no build, because it might be deployed accidentally.

5. **Should `{{ super() }}` be required for block extension?**
   Minijinja supports `{{ super() }}` to include the parent block's content when overriding. The default templates should be designed so that common use cases (adding a script, adding a stylesheet) work with `{{ super() }}` in the `scripts` and `styles` blocks. This is a template design decision, not an engine decision.

---

## 12. Future Considerations

| Feature | Description |
| --- | --- |
| Theme packaging | `zetl theme install <url>` — download and install community themes |
| Template hot-reload | Integrate with SPEC-008 watch mode for live template reloading |
| Custom filters | Register zetl-specific Minijinja filters (wordcount, reading_time, date formatting) |
| RSS/Atom feeds | `feed.xml` template for RSS generation during build |
| Sitemap generation | `sitemap.xml` template for SEO |
| 404 page | Custom `404.html` template for serve mode |
| Partial templates | Convention for `.zetl/templates/partials/` for reusable snippets |
| Theme export | `zetl theme export` to copy built-in templates as customisation starting point |
| Asset pipeline | Optional PostCSS/esbuild integration for CSS/JS processing |

---

**END OF SPEC-012**
