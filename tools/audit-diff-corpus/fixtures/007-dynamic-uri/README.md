# 007-dynamic-uri

**Scenario:** URL whose host/path is built from a template expression.
Static domain allowlisting is useless because the final target
depends on runtime values.

**Expected:** `dynamic-uri`.

**Source:** exfiltration-template survey — attackers use templating
to smuggle unknown hosts past static allowlists.
