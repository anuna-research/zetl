# 011-known-domain-ok

**Scenario:** negative-case regression — baseline already references
`example.com`, so a new link to `www.example.com/x` (same host,
`www.` prefix) must not fire `unseen-domain`.

**Expected:** empty — no findings.

**Source:** hand-authored false-positive guard.
