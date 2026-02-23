# Multi Block Page

This page has multiple SPL blocks.

```spl
; First block: weather facts
(given sunny)
(normally r-sunny-dry
  sunny
  dry)
```

Some text between blocks.

<!-- HTML comment with ```spl
(given hidden-fact)
``` should be excluded -->

```spl
; Second block: rain facts
(given windy)
(normally r-windy-umbrella
  windy
  need-umbrella)
```

More text after the second block.
