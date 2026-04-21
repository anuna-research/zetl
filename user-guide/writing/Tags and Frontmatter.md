---
title: Tags and Frontmatter
tags: [writing, tags, frontmatter, metadata]
---

# Tags and Frontmatter

Tags are how you group pages without forcing them into folders. Frontmatter
is how you attach any other structured data — dates, statuses, authors,
book ISBNs, whatever your vault needs — that zetl and your templates can
both see.

## YAML frontmatter basics

Frontmatter is a YAML block at the very top of a page, fenced by `---`:

```markdown
---
title: Seeing Like a State
tags: [book, political-theory, legibility]
author: James C. Scott
year: 1998
status: finished
rating: 5
---

# Seeing Like a State

...
```

Anything inside the fences is frontmatter; everything after the closing `---`
is the body. zetl reads frontmatter on every scan and exposes it to
templates, search, and hooks. See [[Frontmatter]] for the conceptual model
and [[Frontmatter Fields]] for the reserved-key reference.

## Tagging — two syntaxes

zetl recognises tags in two places.

**Frontmatter tags.** A YAML list under the `tags:` key:

```yaml
tags: [rust, cli, reference]
```

This is the canonical form. Tags here are structured metadata — they're
first-class, they show up in templates, and they don't clutter your prose.

**Inline hashtags.** `#word` tokens inside the body of the page:

```markdown
# Async I/O Notes

Working through #rust #async patterns today. The `tokio` runtime...
```

Both forms work; zetl surfaces them together. Inline tags are convenient for
tagging a paragraph mid-thought; frontmatter tags are better for page-level
classification. A typical page uses frontmatter for 2–4 durable tags and
skips inline tags entirely.

## Finding pages by tag

There's no dedicated `--tag` flag, but `zetl search` is frontmatter-aware and
matches tag text directly:

```bash
zetl search 'rust'               # any page with 'rust' anywhere
zetl search 'tags:.*rust'        # regex — frontmatter list entries
zetl search '#rust'              # inline hashtag form
```

For anything programmatic, export the graph and filter:

```bash
zetl export --json | jq '.pages[] | select(.tags[] | contains("rust"))'
```

See [[Searching]] for the full set of search options and [[Following Links]]
for graph export.

## Frontmatter beyond tags

zetl treats frontmatter as an open map. You can attach any keys you like, and
templates see them via `page.frontmatter.<key>`. Some common patterns:

```yaml
---
title: Meeting with Priya 2026-04-18
tags: [meeting, project-x]
date: 2026-04-18
attendees: [priya, hugo, sam]
status: done
follow-up: 2026-04-25
---
```

A few reserved keys have special meaning to zetl or to the default theme —
`title`, `tags`, `date`, `status`, `draft`, `aliases`. See
[[Frontmatter Fields]] for the full list and the semantics. Custom keys are
left alone: `attendees`, `rating`, `isbn`, `project` — pick whatever schema
fits your vault.

## Templates see your frontmatter

Anything you put in frontmatter is available when `zetl build` renders the
page. If your theme's `page.html` includes:

```jinja
{% if page.frontmatter.status %}
  <span class="status">{{ page.frontmatter.status }}</span>
{% endif %}
{% if page.frontmatter.date %}
  <time datetime="{{ page.frontmatter.date }}">{{ page.frontmatter.date }}</time>
{% endif %}
```

then every page that sets `status:` or `date:` renders accordingly. This is
how you build a book-journal card, a task status pill, or an RSS feed from
page metadata without leaving Markdown. See [[Customising the Look]].

## Keep it small

Frontmatter is a pointy tool. Two heuristics:

- **If it's prose, put it in the body.** Frontmatter is for facts a machine
  might want to filter on, not for narrative.
- **If you add a key, use it.** Dead frontmatter keys that no template or
  query ever reads just add noise. Grep your theme for `page.frontmatter.`
  occasionally and prune.

## Related

- [[Frontmatter]]
- [[Frontmatter Fields]]
- [[Organising Your Vault]]
- [[Searching]]
- [[Customising the Look]]
