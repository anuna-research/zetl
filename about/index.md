# What's possible (and what's next)

## Working today

**Content directives (SPEC-049)** — `:::container{}`, `::leaf{}`; typed props
(`string`/`bool`/`int`/`number`/`url`/`enum`); body rendered + sanitised in isolation;
nested directives with a per-provenance barrier; default-deny invocability; a sound
build-time HTML-context lint (a prop reaching a JS/CSS/URL context fails the build).

**Component islands (SPEC-050)** — sandboxed Web-Worker islands that paint via a controlled
element renderer (Remote DOM); a retained, replay-on-subscribe message bus; inter-island
messaging by topic with capability-scoped grants; typed payload recognition; persisted
topics with pre-paint; lazy hydration (`load`/`idle`/`visible`/`media`); per-page CSP.

## Needs more work

- **Click/input interactivity in content islands** — the Worker model is render-only today
  (worker → host paint, host → worker bus). Forwarding DOM events back to the Worker is a
  follow-up.
- **Trusted in-realm islands** (direct `window.zetl` + DOM event handlers, e.g. a clickable
  theme toggle) work, but are invoked from theme templates rather than content directives,
  and need wiring into the serve path.
- **`zetl serve`** uses a different render path that doesn't yet inject island assets — the
  islands demo requires `zetl build` + a static host.
- Both specs are **non-converged strawmen**: the untrusted-content boundary wants a human
  security review + fuzzing before production use.

See [[index]] for directives and [[islands]] for the live island demos.
