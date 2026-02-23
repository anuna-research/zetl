# Conflicts Page

This page has rules that create an unresolved conflict.

```spl
; Two defeasible rules with opposite conclusions, no superiority
(given evidence-a)
(given evidence-b)
(normally r-approve
  evidence-a
  approved)
(normally r-reject
  evidence-b
  (not approved))
```

Without a superiority relation, `approved` vs `~approved` is ambiguous.
