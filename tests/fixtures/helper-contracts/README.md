# helper-contracts corpus (SPEC-032 REQ-3210 / CON-3210)

Cross-implementation fixture corpus: each `*.json` file is a schema-valid
`Document` (per `tools/ztl-ast-schema-v1.json` v1.0) that must round-trip
**bit-for-bit** through every first-party helper library's identity
transform:

- **Rust** — `serde_json::from_value::<Document>(_)` + `to_value(_)`
- **Python** — `ztl_ast.run(lambda ast, ctx: ast)` (persistent mode)
- **JavaScript** — `ztl-ast-js` `run((ast) => ast)` (persistent mode)

Run via `cargo test --test helper_contracts_integration` — the test fails
with a side-by-side diff on any disagreement.

Each fixture should exercise a distinct slice of the v1 schema. Keep
fixtures small: the gate runs ~4 spawns per fixture (three helpers +
baseline), so corpus size affects CI time linearly.

## Adding a fixture

1. Drop a new `<name>.json` here that validates against the schema.
2. Ensure it is canonical: objects use insertion-order keys matching
   what the Rust helper emits (run through `ztl ast sample` if in
   doubt — byte-identical round-trip is the floor, not structural
   equivalence).
3. Run `cargo test --test helper_contracts_integration` locally with
   both `python3` and `node` on `PATH`.
