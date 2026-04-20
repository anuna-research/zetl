# 003-javascript-uri

**Scenario:** `javascript:` URI in a markdown link. The sanitiser
drops it at render, but the audit gate surfaces the author's intent.

**Expected:** `dangerous-scheme`.

**Source:** OWASP XSS cheatsheet + historical zetl BUG-016 scenario.
