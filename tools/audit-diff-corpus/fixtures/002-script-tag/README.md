# 002-script-tag

**Scenario:** classic `<script>` injection lifted from the OWASP XSS
cheatsheet. The sanitiser strips both tag and content, so runtime is
safe, but the intent must surface for review.

**Expected:** `sanitiser-stripped` (and `raw-html` is _not_ expected
because the sanitiser fully removes the fragment — nothing survives).

**Source:** OWASP XSS Filter Evasion cheatsheet.
