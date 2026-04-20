// Browser-side HTML sanitiser (CON-3408 STEP 4). Defense-in-depth: the
// Rust `ammonia` pass at build time is the authoritative enforcement,
// but the ciphertext body crosses a cryptographic trust boundary so the
// shim re-applies the allowlist on the decrypted plaintext before
// injecting it into the DOM. Allowlist mirrors `tools/sanitiser-config.toml`.

const ALLOWED_TAGS = new Set([
  "a", "abbr", "article", "aside", "b", "bdi", "bdo", "blockquote",
  "br", "caption", "cite", "code", "data", "dd", "del", "details",
  "dfn", "div", "dl", "dt", "em", "figcaption", "figure", "footer",
  "h1", "h2", "h3", "h4", "h5", "h6", "header", "hr", "i", "img",
  "ins", "kbd", "li", "main", "mark", "nav", "ol", "p", "pre",
  "q", "rp", "rt", "ruby", "s", "samp", "section", "small", "span",
  "strong", "sub", "summary", "sup", "table", "tbody", "td", "tfoot",
  "th", "thead", "time", "tr", "u", "ul", "var", "wbr",
]);

const STRIP_WITH_CHILDREN = new Set([
  "script", "style", "iframe", "object", "embed", "template", "math",
  "base", "frame", "frameset", "noframes", "noscript", "xmp", "svg",
  "link", "meta", "form", "button", "input", "select", "textarea",
]);

const GENERIC_ATTRS = new Set([
  "id", "class", "title", "lang", "dir", "role", "aria-label",
  "aria-hidden", "aria-describedby", "aria-labelledby",
]);

const PER_TAG_ATTRS: Record<string, Set<string>> = {
  a: new Set(["href", "hreflang", "rel", "target", "type", "download"]),
  img: new Set(["src", "alt", "width", "height", "loading", "decoding"]),
  blockquote: new Set(["cite"]),
  q: new Set(["cite"]),
  time: new Set(["datetime"]),
  th: new Set(["scope", "colspan", "rowspan", "abbr", "headers"]),
  td: new Set(["colspan", "rowspan", "headers"]),
  col: new Set(["span"]),
  colgroup: new Set(["span"]),
  ol: new Set(["start", "type", "reversed"]),
  li: new Set(["value"]),
  details: new Set(["open"]),
  data: new Set(["value"]),
  code: new Set(["data-lang"]),
  pre: new Set(["data-lang"]),
};

const URL_ATTRS = new Set(["href", "src", "cite", "action", "formaction"]);
const DENIED_SCHEMES = /^\s*(javascript|data|vbscript|file|about):/i;

/// Sanitise a raw HTML string to the REQ-3421 allowlist and return the
/// cleaned HTML as a string. The result is safe to assign to
/// `.innerHTML` on a capability-mode host element.
export function sanitiseHtml(html: string): string {
  const doc = new DOMParser().parseFromString(`<body>${html}</body>`, "text/html");
  scrub(doc.body);
  return doc.body.innerHTML;
}

function scrub(node: Element): void {
  // Walk children defensively (live collection would shift under our feet).
  const kids = Array.from(node.children);
  for (const child of kids) {
    const tag = child.tagName.toLowerCase();
    if (STRIP_WITH_CHILDREN.has(tag)) {
      child.remove();
      continue;
    }
    if (!ALLOWED_TAGS.has(tag)) {
      // Unknown tag: unwrap (keep text children, drop element).
      unwrap(child);
      continue;
    }
    pruneAttrs(child, tag);
    scrub(child);
  }
}

function unwrap(el: Element): void {
  const parent = el.parentNode;
  if (!parent) return;
  while (el.firstChild) parent.insertBefore(el.firstChild, el);
  parent.removeChild(el);
}

function pruneAttrs(el: Element, tag: string): void {
  const perTag = PER_TAG_ATTRS[tag];
  for (const attr of Array.from(el.attributes)) {
    const name = attr.name.toLowerCase();
    if (name.startsWith("on")) {
      el.removeAttribute(attr.name);
      continue;
    }
    const allowed = GENERIC_ATTRS.has(name) || (perTag !== undefined && perTag.has(name));
    if (!allowed) {
      el.removeAttribute(attr.name);
      continue;
    }
    if (URL_ATTRS.has(name) && DENIED_SCHEMES.test(attr.value)) {
      el.removeAttribute(attr.name);
    }
  }
}
