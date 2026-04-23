---
title: Frontmatter
tags: [concepts, frontmatter, metadata]
---

# Frontmatter

**Frontmatter** is the YAML block at the top of a Markdown file, fenced by `---`. ztl reads it, exposes it in templates, and otherwise gets out of the way. It is entirely optional.

## What it looks like

```markdown
---
title: Zettelkasten Method
tags: [note-taking, knowledge-management]
date: 2026-01-14
status: draft
---
# Zettelkasten Method

The Zettelkasten is a slip-box system...
```

Everything between the two `---` fences is parsed as YAML. Everything after is the page body.

## What ztl does with it

Three things, all passive:

1. **Parses it once per page**, on indexing.
2. **Makes it available in templates** as `page.frontmatter.<key>`. If you're theming `ztl serve` or `ztl build`, you can read anything you've written — `{{ page.frontmatter.tags }}`, `{{ page.frontmatter.status }}`, anything. See [[Customising the Look]].
3. **Exposes it to hooks** via the vault context, so render-pipeline hooks and lifecycle scripts can make decisions based on metadata.

ztl does not require any specific field. No `title`, no `id`, no `uuid`. Your filename is your page name; the graph builds itself from wikilinks.

## What the community uses it for

None of these are built-in — they're conventions that happen to work:

| Field | Use |
|-------|-----|
| `tags` | A list for topical grouping; read by themes and `ztl check`-style tooling |
| `date` | Publication or creation date; often used for journal pages |
| `status` | `draft`, `published`, `archived` — render differently per status |
| `aliases` | Alternate names for the same page; used by some themes for search |
| `author` | For multi-author vaults |

A minimal page doesn't need any of this. A page with no frontmatter at all is completely fine:

```markdown
# Evergreen Notes

A note you tend over time, rather than one you write once and forget.
```

See [[Tags and Frontmatter]] for workflow-level patterns.

## Arbitrary keys are welcome

Anything valid YAML is fair game. Nested objects, lists, numbers, booleans — all accessible in templates:

```markdown
---
project:
  name: ztl
  version: 0.5.0
  contributors: [hugo, ana]
published: false
---
```

`page.frontmatter.project.name` is `"ztl"` in your theme. Use this for whatever structure your vault needs — book bibliographies, task metadata, research parameters — without asking permission.

## Reserved fields

A small number of keys have meaning to specific features. The one to know about today:

- **`parser:`** — picks a non-default Markdown parser (Pandoc, remark) for pages that need it. Only relevant if you've set up [[Plugin Ecosystems]]. Without a `parser:` field, ztl uses its built-in CommonMark parser.

More reserved-key documentation lives at [[Frontmatter Fields]]. If you never use those features, never think about them.

## A worked example: tag-filtered index

Here's why exposing frontmatter to templates matters. Given a vault of project notes each with:

```markdown
---
status: active
owner: ana
due: 2026-05-01
---
```

A custom `index.html` in `.ztl/themes/default/` can filter the page grid:

```jinja
{% for p in vault.pages if p.frontmatter.status == "active" %}
  <li><a href="/page/{{ p.slug }}/">{{ p.title }}</a> — due {{ p.frontmatter.due }}</li>
{% endfor %}
```

The vault stays plain Markdown; the presentation reads your metadata. See [[Customising the Look]].

## What frontmatter is not

- **Not required.** A vault of files with no YAML headers is a valid ztl vault.
- **Not validated.** ztl doesn't tell you a field is "wrong" — it doesn't know what your fields mean. Invalid YAML (bad indentation) is flagged; semantic choices are yours.
- **Not a schema.** If you want a schema, layer one on with [[Lifecycle Hooks]] or [[What is SPL|SPL rules]].

## Related

- [[Tags and Frontmatter]]
- [[Frontmatter Fields]]
- [[Writing Pages]]
- [[Customising the Look]]
