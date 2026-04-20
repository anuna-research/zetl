# 010-raw-html-div

**Scenario:** author drops an apparently-harmless `<div>` into
markdown. The sanitiser passes it, but the audit gate still surfaces
it so a human eyeballs the structural change before merge.

**Expected:** `raw-html`.

**Source:** hand-authored — false-positive-shaped detector regression
check.
