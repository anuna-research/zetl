# Xanadu Lineage

Ted Nelson's Project Xanadu envisioned a hypertext system with visible connections between documents — "parallel pages" where you could see the source and destination of every link simultaneously.

## The vision

In Xanadu, links aren't hidden behind underlined text. When you read a document, you see its connections to other documents rendered alongside it. The original and the referenced text appear side by side, connected by visible "bridge" lines.

## How zetl implements it

zetl's [[View Command]] is a terminal-native interpretation of Xanadu transclusion:

- **Left pane** — the current note, with numbered `[N]` anchor glyphs at each [[concepts/Wikilinks|wikilink]]
- **Bridge column** — color-coded connectors pairing anchors to context cards
- **Right pane** — context cards: excerpts from forward-linked pages

The [[Serve Command]] and [[Build Command]] implement the same design with SVG bridge connectors in the browser.

## Why it matters for PKM

In a Zettelkasten, notes are meant to be read in context of their connections. Xanadu-style visible links make those connections tangible:

- You see what a page links to without navigating away
- Context cards give you enough of the linked page to understand the connection
- Bridge connectors make the structure of knowledge visible

## Transclusion vs embedding

zetl's context cards are read-only excerpts, not full transclusion in the Xanadu sense. The `![[embed]]` wikilink syntax requests content embedding, but zetl's view renders a context card rather than inlining the full page.

## Design decisions

See [[Xanadu View Design]] for the architectural choices behind zetl's implementation, and [[SPEC-009 Xanadu View]] for the full specification.

See also: [[View Command]], [[Serve Command]], [[Xanadu View Design]], [[SPEC-009 Xanadu View]]
