# 008-object-embed

**Scenario:** `<object>` + `<embed>` tags carrying remote payloads.
Both sit in the sanitiser's strip-content denylist.

**Expected:** `sanitiser-stripped`.

**Source:** OWASP XSS cheatsheet (plugin-based XSS).
