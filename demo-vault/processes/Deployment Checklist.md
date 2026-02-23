---
title: Deployment Checklist
---

# Deployment Checklist

Before deploying to production, all conditions below must be verified.
```spl
(normally r-ready-prod
  (and verified-load-test verified-security-audit)
  ready-for-production)
```

## Checklist

- [ ] Load test passing — provides `verified-load-test`
- [ ] Security audit complete — provides `verified-security-audit`

Once both conditions are met, `ready-for-production` becomes defeasibly provable.
Note that [[License Audit]] findings may affect the [[Redis vs Memcached]] decision
independently of production readiness.
