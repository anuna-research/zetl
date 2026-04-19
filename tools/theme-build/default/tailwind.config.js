/** @type {import('tailwindcss').Config} */
module.exports = {
  content: [
    "../../../themes/default/**/*.html",
    // JS under themes/default/static/ emits runtime classes too
    // (e.g. index-toolbar.js builds .btn.btn-sm.btn-outline.mt-4.mx-auto).
    // Without this glob those utilities would be purged.
    "../../../themes/default/**/*.js",
    "../../../src/web/**/*.rs",
  ],
  // Classes emitted by Rust code (e.g. wikilink rendering) or constructed
  // dynamically — list them so Tailwind doesn't purge them.
  safelist: [
    "link",
    "link-primary",
    "link-error",
    "wikilink",
    "wikilink-dead",
    "wl-active",
    { pattern: /^(btn|card|alert|badge|menu|stat|stats|drawer|navbar|divider|breadcrumbs|tooltip|modal|dropdown|input|textarea|checkbox|radio|toggle|table|tabs|mask)(-[\w-]+)?$/ },
    { pattern: /^(bg|text|border)-(primary|secondary|accent|neutral|info|success|warning|error|base-[123]|content)$/ },
  ],
  theme: {
    extend: {},
  },
  plugins: [
    require("@tailwindcss/typography"),
    require("daisyui"),
  ],
  daisyui: {
    themes: ["light", "dark"],
    logs: false,
  },
};
