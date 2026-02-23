---
title: License Audit
---

# License Audit
```spl
(given discovered-license-risk)
(except d-license-risk discovered-license-risk (not decided-use-redis))
```

A license compliance review discovered that Redis uses a dual-license (RSL/SSPL)
that conflicts with our distribution requirements. This defeats the conclusion
`decided-use-redis` from [[Redis vs Memcached]].

See [[Deployment Checklist]] for how this affects production readiness.
