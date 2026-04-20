# 005-iframe-survives

**Scenario:** raw `<iframe>` HTML block in markdown. The sanitiser's
strip-content policy removes the whole element, so both
`sanitiser-stripped` must fire (attack intent) and the pre-sanitisation
tag is reported as raw HTML.

**Expected:** `sanitiser-stripped`.

**Source:** OWASP XSS cheatsheet.
