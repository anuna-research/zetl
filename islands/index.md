# Component islands (SPEC-050)

Everything below is rendered by **untrusted code running in a dedicated Web Worker**. A
Worker has no DOM and no `window.zetl`; it posts a declarative element tree and the trusted
host paints it through a closed allowlist renderer (CON-5007). Open devtools (Console +
Application → Local Storage) to watch the runtime.

## Live rendering (Worker → Remote DOM)

The counter is re-painted by its Worker every second; the host reconciles the change:

:::counter{}
(static fallback shown when JS is off)
:::

## Inter-island messaging (the shell bus)

The **mirror** island is a *separate* Worker. It can't reach the counter directly — it only
hears `content:count` through the shell bus (`window.zetl`), with a monotonic `seq`:

:::mirror{}
(static fallback)
:::

Two sandboxed workers, coordinating only through capability-checked topics. The
`content:count` value is also **persisted** to `localStorage` (`zetl:topic:content:count`)
and re-applied before first paint on reload (no flash).

## Security — a hostile island, neutralised

This island's Worker deliberately tries to inject a `<script>`, an `<iframe>`, a
`javascript:` link, a remote tracking `<img>`, and an `on*` handler — and tries to publish
to a **trusted** topic it wasn't granted, plus an out-of-type value. The host strips every
dangerous node and the capability bridge denies the bad publishes (see the Console for
`denied:…`):

:::guard{}
(static fallback)
:::

## Progressive enhancement

Every island above has a `<noscript>` static fallback and works with JS disabled — the
island only *enhances*. The page also carries a default-deny **Content-Security-Policy**
`<meta>` as its first `<head>` child: `script-src` is strict (hashed), `connect-src 'none'`,
worker output is sandboxed.

---

*(scroll down — the last island uses lazy hydration)*

&nbsp;

&nbsp;

&nbsp;

&nbsp;

&nbsp;

&nbsp;

&nbsp;

&nbsp;

&nbsp;

&nbsp;

&nbsp;

&nbsp;

## Lazy hydration (`hydrate=visible`)

This island's Worker is **not created at page load** — it spawns only when this element
scrolls into view (`IntersectionObserver`). The timestamp shows when it actually mounted:

:::lazy{}
(static fallback)
:::
