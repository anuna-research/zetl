# Open Graph background image — Midjourney prompt

`zetl build` generates a `og.png` (1200×630) for every built page using a
user-supplied background template composited underneath the page title. If
you drop a file at any of these paths, `zetl build` will pick it up
automatically:

- `<your-vault>/.zetl/static/og-background.png`
- `<your-theme>/static/og-background.png` (for a custom theme)

The template should be subtle enough to sit behind white text at the
left-centre of the frame. The build pipeline applies a dark bottom scrim
automatically for legibility.

## Recommended Midjourney prompt

```
A minimalist knowledge-graph visualisation, thin glowing nodes connected by
faint neural pathways, abstract bidirectional wikilinks forming a quiet
constellation, deep indigo and charcoal background with soft cyan and
violet highlights, subtle paper-noise texture, negative space heavy on the
left half of the frame for text overlay, cinematic low-contrast lighting,
wide editorial composition --ar 1200:630 --style raw --stylize 200 --v 6
```

## Alternative prompts

**Archive / library mood:**

```
A cross-section of an infinite library rendered as translucent stacked
paper, floating index cards with hand-drawn link arrows, muted parchment
tones against a dark walnut background, soft studio lighting, cinematic
depth of field, ample negative space on the left two-thirds of the frame
for typography overlay --ar 1200:630 --style raw --stylize 150 --v 6
```

**Abstract topology:**

```
Topographic contour map blended with circuit traces, soft gradient from
midnight blue to deep violet, faint gold highlights along the ridges, a
large calm negative-space area on the left for typography, high-end
editorial magazine aesthetic, subtle film grain --ar 1200:630 --style raw
--stylize 220 --v 6
```

## Tips

- The page title is drawn at roughly 72px inset from the left edge,
  starting around y=410 of the 630px canvas. Keep that region uncluttered.
- The vault name (smaller subtitle) sits at y=350.
- The zetl wordmark appears in the bottom-right corner.
- A dark gradient scrim is automatically overlaid on the bottom ~55% of
  the frame, so you can safely publish brighter or busier backgrounds.
- Output MUST be saved as PNG at 1200×630 (Midjourney at `--ar 1200:630`
  will be close enough; the renderer centre-crops to fit).
