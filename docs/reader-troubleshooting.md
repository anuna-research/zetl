# Reader Troubleshooting

> **Scope.** This document is for **readers** — people who received an
> invite link or enrolment URL to a capability-mode ztl wiki and hit an
> error page in their browser. It mirrors the troubleshooting section of
> the static `docs/reader.html` onboarding page that the wiki can ship
> from `dist/`. Operators deploying a capability-mode wiki should link
> their readers to whichever copy they prefer.
>
> The in-browser error page (the red banner the shim renders via
> `renderError` in `src/cap/shim/errors.ts`) contains an
> `[data-ztl-error-help]` link that deep-links to the matching anchor
> below — e.g. `#err-signature-failed`. Operators who host this
> document at a different URL can override the link base with the
> `ztl_READER_TROUBLESHOOTING_BASE` build-time env var (see
> `docs/capability-security.md`).

Every entry has the same shape:

- **What you saw.** The exact error copy the shim rendered.
- **What it means.** A plain-language explanation.
- **What to do.** Concrete next steps, in order. If a step requires
  contacting your **wiki operator** (the person or team who sent you
  the original invite), that is noted explicitly.

No step here asks you to tinker with cryptographic material. If any
advice ever suggests you *override a signature check*, *edit the URL*,
or *paste secrets into a console*, stop — that is a scam. The real
recovery path is always "ask the operator for a fresh invite."

## Table of contents

- [err-signature-failed — "This page's signature did not verify"](#err-signature-failed)
- [err-need-invite — "This page is only readable from a fresh invite URL"](#err-need-invite)
- [err-identity-unavailable — "Could not recover the reading identity"](#err-identity-unavailable)
- [err-tofu-failed — "Could not bind this device to the cohort passkey"](#err-tofu-failed)
- [err-decrypt-failed — "Could not decrypt this page"](#err-decrypt-failed)
- [err-envelope-malformed — "This page's envelope is malformed"](#err-envelope-malformed)
- [err-sw-purge-failed — "Could not clear stale service workers"](#err-sw-purge-failed)
- [err-lock-unavailable — "Your browser does not support the concurrency lock"](#err-lock-unavailable)
- [err-host-missing — "Capability-mode mount point is missing"](#err-host-missing)
- [err-internal — "An internal error occurred"](#err-internal)
- [fallback-prf-unavailable — "Running in fragment-required mode…"](#fallback-prf-unavailable) *(banner, not an error)*

---

## err-signature-failed

**What you saw.**

> This page's signature did not verify — possible tampering; contact your
> wiki operator

**What it means.** The page your browser received is not what the
operator of this wiki actually published. The wiki signs every page
with a long-lived key; your browser checks that signature before it
does anything else. When the signature does not match, the shim
refuses to decrypt and refuses to prompt you for a passkey.

This is the **most safety-critical** error in the list. It does not
necessarily mean someone is attacking you — the most common real-world
cause is that the operator rotated their signing key and a CDN cache
is still serving files signed with the old one. Either way, the right
move is to **stop**, not to retry.

**What to do.**

1. Do **not** reload repeatedly. The browser is protecting you.
2. Do **not** "work around" the error by opening the URL in a
   different browser. The error is not about your browser.
3. Close the tab.
4. Contact your wiki operator. Quote:
   - the URL of the page (you can safely include everything before the
     `#` fragment; omit the `#k=…` part),
   - the message "signature did not verify",
   - the approximate time you saw it.
5. The operator can check whether they recently rotated the vault
   signing key (`ztl cap rotate-signing-key`) without flushing the
   CDN cache; that sequence is what produces this error under normal
   operation.
6. Once they confirm the fix — usually "cache purged, retry now" —
   open the URL in a private/incognito window to bypass any local
   cache.

If the operator says **nothing changed on their side**, they should
treat this as a potential incident and follow the response steps in
`docs/signing.md` §5. Do not open the wiki again until they confirm.

---

## err-need-invite

**What you saw.**

> This page is only readable from a fresh invite URL on this device.
> Ask your wiki operator for a new invite URL.

**What it means.** The wiki does not know this browser on this
device. Either this is your first visit and the URL you opened was
missing its secret fragment (the part after `#`), or the invite has
already been consumed.

Invite links in delegated-URL mode are **single-device, single-first-use**
by design: the first browser to open them consumes the secret and
binds it to a passkey on that device. A second device opening the
same URL cannot re-bind.

**What to do.**

- Ask your wiki operator for a **new invite URL**.
- When you receive it, open the full URL — including everything after
  the `#` — in the browser you want to read from.
- Do not forward the URL to another device; request a separate invite
  per device instead.

---

## err-identity-unavailable

**What you saw.**

> Could not recover the reading identity for this cohort on this device.
> Ask your wiki operator to re-invite you.

**What it means.** Your browser has a passkey for this site but can
no longer find the wrapped reading key it expects to pair with it.
Common causes:

- you cleared site data or "storage" for this origin,
- you are in a private/incognito window that cannot see your main
  passkey store,
- your browser profile has been reset, migrated, or re-synced in a
  way that dropped the underlying IndexedDB entry,
- you have multiple browser profiles and opened the wiki in the wrong
  one.

**What to do.**

1. Check whether you are in a private/incognito window. If so, open a
   normal window.
2. Check that you are in the browser profile where you originally
   enrolled.
3. If neither applies — or if you recently cleared site data — ask
   your wiki operator to re-invite you. The stored binding is gone
   and only a fresh invite can rebuild it.

---

## err-tofu-failed

**What you saw.**

> Could not bind this device to the cohort passkey — TOFU registration
> failed. Reload to retry; if the problem persists, your browser or
> authenticator may not support the WebAuthn PRF extension.

**What it means.** The wiki needs a specific WebAuthn feature called
the **PRF extension** to derive the in-browser reading key from your
passkey. If your browser, device, or external authenticator does not
implement PRF, the bind step fails.

The other failure mode is ordinary: the passkey prompt was dismissed
or the biometric check failed.

**What to do.**

1. Reload the page. When the passkey prompt appears, approve it.
2. If you are using an external hardware key, make sure the key
   itself supports PRF. Most recent YubiKey 5 firmware does; older
   FIDO2 keys may not.
3. Update your browser. Chrome and Edge have shipped PRF support for
   a long while; Firefox added it in recent versions. Safari support
   is newer.
4. If it still fails, switch browsers to a recent Chromium- or
   Firefox-based build.
5. If none of that helps, tell your operator which browser and
   authenticator you are using. Some device/browser combinations
   genuinely cannot read a capability-mode wiki; the operator may be
   able to offer a different access path.

---

## err-decrypt-failed

**What you saw.**

> Could not decrypt this page — the invite may have been revoked or
> rotated. Ask your wiki operator for a new invite.

**What it means.** The signature was valid (so the page is genuinely
from the operator) but your reading key does not open it. The
overwhelmingly most common cause is that your access was **revoked or
rotated**: the operator removed you from the reader group, or rotated
cohort keys and you have not yet received the new invite.

**What to do.**

- Contact your wiki operator. Ask:
  - "Am I still in the reader group for this wiki?"
  - "Did you rotate cohort keys recently?"
- If you are still meant to have access, they will send a fresh invite.
- If you were intentionally removed, they will tell you.

A less common cause is that you were enrolled under a different cohort
than the page belongs to — in a multi-cohort wiki, each cohort has a
separate reader group. The operator can confirm.

---

## err-envelope-malformed

**What you saw.**

> This page's envelope is malformed and cannot be read. Reload; if the
> problem persists, contact your wiki operator.

**What it means.** The file your browser received is not in a shape
the shim knows how to parse. Usually a transient deploy artefact —
e.g. the CDN served a half-written file while a deploy was in flight.

**What to do.**

1. Wait a minute and reload.
2. If it persists, contact your wiki operator.

---

## err-sw-purge-failed

**What you saw.**

> Could not clear stale service workers on this origin — close all tabs
> for this site and try again.

**What it means.** An earlier version of the wiki registered a helper
script (a "Service Worker") in your browser, and the current shim
cannot remove it from this tab alone. Service workers are scoped
across tabs; closing just this one is not enough.

**What to do.**

- Close **every** open tab for this site.
- Open the URL fresh in a new tab.

If that still does not clear the error, reload with the cache bypassed
(**Ctrl+Shift+R** on Windows/Linux, **Cmd+Shift+R** on macOS). If it
*still* does not clear, contact your wiki operator.

---

## err-lock-unavailable

**What you saw.**

> Your browser does not support the concurrency lock this shim
> requires (navigator.locks). Use a recent Chromium- or Firefox-based
> browser.

**What it means.** The shim serialises concurrent tabs with the
browser's `navigator.locks` API so two tabs never step on each other
during the first-visit flow. Very old browsers do not implement it.

**What to do.** Switch browsers. Chrome, Edge, Brave, Arc, or any
Chromium-based browser released in the last couple of years will work.
Recent Firefox does too. Safari support is newer; if your Safari is on
a very old OS you may need to update the OS.

---

## err-host-missing

**What you saw.**

> Capability-mode mount point &lt;main data-ztl-capability&gt; is
> missing from the HTML shell.

**What it means.** The HTML scaffolding the shim mounts into is not
in the page. Almost always a build or deploy problem on the operator
side — not something you can fix from the browser.

**What to do.** Contact your wiki operator. Quote the URL and the
message. Do not try to patch the page locally.

---

## err-internal

**What you saw.**

> An internal error occurred while rendering this page. Reload; if the
> problem persists, contact your wiki operator.

**What it means.** The shim hit a condition it does not have a
specific message for. It is the catch-all kind.

**What to do.**

1. Reload once.
2. If it repeats, contact your wiki operator. Quote the URL. Let them
   know this is the generic "internal error" message, not one of the
   named kinds above — that distinction helps them triage.

---

## fallback-prf-unavailable

**What you saw.**

> Running in fragment-required mode — your browser does not advertise
> WebAuthn PRF, so this page must be reopened from the full invite URL
> on every visit.

This is a **banner**, not a red error page. The page still decrypted
and is readable below the banner.

**What it means.** Your browser or authenticator does not expose the
WebAuthn PRF extension the wiki uses to wrap your reading key with a
passkey. Without PRF the shim cannot store a per-device binding, so
every visit relies on the secret at the end of the URL. The wiki
operator has chosen "graceful fallback" over "hard fail" — you still
read the page, with a small but real widening of the URL-leak
surface.

**What to do.**

- **If you are fine with the trade-off.** Bookmark the invite URL
  (including its `#k=…` suffix) and reopen from the bookmark each
  visit. Treat the bookmark itself as the secret — do not copy it
  into chat, shorteners, or preview bots.
- **If you are not.** Switch to a browser that supports WebAuthn
  PRF and open a fresh invite there:
  - Chrome 116+ / Edge / Brave / Arc on any OS,
  - Firefox 128+ on desktop,
  - Safari on macOS Sonoma+ or iOS 17+.
- **If you are already on one of the supported browsers** and still
  see the banner, update the browser. PRF arrived over several
  releases; older-than-two-year builds often do not have it.
- **Enterprise or WebView browsers.** Some locked-down browsers
  advertise WebAuthn but omit PRF. Open the URL in a different
  browser on the same device, or ask your operator whether a
  different enrolment path is available.

**What does not help.**

- Reloading the page — the probe result is stable across reloads
  on the same browser.
- Clearing site data — there is no binding to clear in this mode.
- Copying the URL to another tab in the same browser — same result.

**Why this exists.** The spec (REQ-3412) calls for a fallback rather
than a hard error so readers on older devices still see content
rather than a wall. The banner is deliberately non-intrusive (a
single paragraph above the page, not a modal) so it does not get
in the way of reading. Operators monitoring their deployment will
see an OBS-3412 signal tick each time a visit falls back — if the
rate is unexpected, they may change the enrolment path.

---

## Asking your operator well

If you land here often enough to start a support conversation,
including the following in your first message makes everything
faster:

- The **URL** of the page that failed (everything before `#`).
- The **error kind** you saw (the slug after `err-` in the anchor
  above).
- Your **browser + version** and your **device**.
- Whether you are on a **new device / new browser / incognito
  window**.
- Whether you did anything that touched site data recently ("I
  cleared cookies", "I restored from backup", "I ran a privacy
  extension").

The operator has a counterpart reference at `docs/signing.md` and
`docs/capability-security.md`; they can map your symptoms to the
underlying cause once they know which error you saw.
