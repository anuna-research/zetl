---
title: Frontmatter Fields
tags: [reference, frontmatter, metadata]
---

# Frontmatter Fields

Frontmatter is YAML at the very top of a page, fenced by `---` lines. zetl reads a handful of reserved keys — everything else passes through untouched for templates, hooks, and search. **Nothing is required.** A page with no frontmatter at all is valid.

```markdown
---
title: Zettelkasten Method
tags: [pkm, method, luhmann]
date: 2024-06-03
status: draft
---

# Zettelkasten Method

Niklas Luhmann's slip-box system…
```

## Reserved keys

zetl inspects these during indexing and rendering:

| Key | Type | Meaning |
| --- | --- | --- |
| `title` | string | Display title. Falls back to the filename stem when absent. Does *not* rename the file or change its wikilink target. |
| `parser` | string | Force a specific parser for this page. Values: `commonmark` (default) or `pandoc`. See [[Writing Pages]]. |
| `tags` | list of strings | Surfaced in templates and in `zetl search --path`. Drives the tag cloud. |
| `description` | string | Plain-text `<meta name="description">` / social-preview text. Falls back to the first paragraph. |

No key is mandatory. Omit any of them and zetl infers a sensible default.

## Parser selection

The `parser:` key wins over every other selector. Precedence (highest to lowest):

1. `parser:` in the page's frontmatter
2. First matching `[[parse.rule]]` glob in `.zetl/config.toml`
3. Top-level `[parse] default` in `.zetl/config.toml`
4. `commonmark` (zetl's built-in default)

An unknown parser name (e.g. `parser: djot` when only `commonmark` is registered) produces a per-page error and the page is skipped — the rest of the build proceeds. Run `zetl ecosystem check` to see which parsers are available.

## Convention-only keys

The following are *conventions*, not reserved — zetl does not parse them, but templates and hooks consume them by name. Use whichever match your workflow.

| Key | Convention |
| --- | --- |
| `date` | Publication / last-edited date. Any ISO-8601 string. Themes often sort by this. |
| `status` | `draft`, `published`, `archived`. Hooks read it for publish-gating. |
| `author` | Override for multi-author vaults. |
| `aliases` | Alternate titles — surfaced in some themes; not currently resolved by the wikilink matcher. |

Anything you invent is fair game — add `project_code: Q2-2026`, `mood: 🔥`, `reading_time: 8m`. Templates see it via `page.frontmatter.<key>`.

## Template variables

Every HTML render receives a `page` object. The fields writers care about:

| Variable | Source |
| --- | --- |
| `page.title` | `title:` frontmatter, or the filename stem if absent. |
| `page.slug` | URL-safe page path, relative to vault root. |
| `page.frontmatter` | The entire parsed YAML as a JSON object. Access nested keys as `page.frontmatter.date`, `page.frontmatter.tags[0]`, etc. |
| `page.description` | Either the frontmatter `description:` or the first paragraph. |
| `page.content_html` | Rendered HTML body. |
| `page.content_raw` | Original Markdown source. |
| `page.backlinks` | List of inbound links. |
| `page.outlinks` | List of outbound links (with `is_dead` flag). |
| `page.breadcrumbs` | Directory path components. |
| `page.history` | Snapshot timeline. `null` when history is unavailable. |

`page.frontmatter` is the *raw* parsed document — any key you put in the YAML appears here verbatim. This is how templates pick up your custom fields without any template changes.

```jinja
{% if page.frontmatter.status == "draft" %}
  <aside class="draft-banner">Draft — not for circulation.</aside>
{% endif %}

{% for tag in page.frontmatter.tags %}
  <a class="tag" href="/tags/{{ tag }}">#{{ tag }}</a>
{% endfor %}
```

## Interaction with wikilinks

Frontmatter `title:` changes *display*, not *addressing*. A page at `notes/zettelkasten.md` is linked as `[[zettelkasten]]` or `[[Zettelkasten]]` regardless of whether its frontmatter declares `title: The Zettelkasten Method`. The rendered heading and the entry in every other page's backlinks will show the title-cased frontmatter value.

If you want `[[The Zettelkasten Method]]` to resolve, rename the file.

## Interaction with search

`zetl search` reads both the body *and* the serialised frontmatter text. A query for `status: draft` matches draft pages. `tags:` values are searchable by their string.

## Interaction with SPL

Frontmatter keys are ground-truth facts for SPL reasoning under `--features reason`. A rule can test `@page.frontmatter.status == "published"` (subject to the SPL syntax — see [[Writing SPL]]).

## Related

- [[Writing Pages]]
- [[Tags and Frontmatter]]
- [[Organising Your Vault]]
- [[CLI Overview]]
- [[Configuration]]
