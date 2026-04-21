# 015-evil-data-html

**Scenario:** `data:text/html` payload embedded in an `<a href>`
rendered from HTML-in-markdown. Both detectors fire: the `data:`
scheme and the raw HTML tag.

**Expected:** `dangerous-scheme`, `sanitiser-stripped`.

**Source:** OWASP XSS cheatsheet (data-URI variant).
