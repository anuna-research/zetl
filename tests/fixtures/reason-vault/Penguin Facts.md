# Penguin Facts

Penguins are birds that cannot fly. See also [[Bird Facts]].

```spl
; Penguins are birds that don't fly
(given penguin)
(always r-penguin-is-bird
  penguin
  bird)
(normally r-penguin-no-fly
  penguin
  (not flies))
(prefer r-penguin-no-fly r-bird-flies)
```

This overrides the default bird-flies rule for penguins.
