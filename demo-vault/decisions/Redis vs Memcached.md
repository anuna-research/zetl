---
title: Redis vs Memcached
---

# Redis vs Memcached

We evaluated both Redis and Memcached for our caching layer.
Redis offers persistence and pub/sub, which align with our requirements.
See [[Deployment Checklist]] for production readiness criteria.

```spl
(given evaluated-redis)
(given redis-supports-persistence)
(normally r-prefer-redis
  (and evaluated-redis redis-supports-persistence)
  decided-use-redis)
```

Based on these facts, we defeasibly conclude `decided-use-redis`.
This conclusion may be revisited if new evidence emerges — see [[License Audit]].
