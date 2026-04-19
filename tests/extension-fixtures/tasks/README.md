# Tasks extension fixture

Golden-HTML gate for the canonical `tasks` extension (SPEC-032 REQ-3212).
The fixture exercises Obsidian-style task-list syntax:

- `- [ ]` — open task
- `- [x]` / `- [X]` — completed task
- Inline `@due(YYYY-MM-DD)` — optional due date, surfaced as a `<time>`
  element and stripped from the visible label.

The committed runner (`tools/xtask/src/runners.rs::tasks_runner`) is the
thin in-process stub the task description specifies. It renders the HTML
shape a real theme would produce after reading the template vars a
persistent hook emits (`page.ext.tasks.open`, `page.ext.tasks.done`,
`page.ext.tasks.due`). Real vaults run the capture + var emission
through an ecosystem plugin (Obsidian Tasks subset, SPEC-033 §13 Q4);
the stub exists to gate the theme CSS + template contract without
pulling an external binary into CI.

## Regenerate after edits

```
cargo xtask update-golden tasks
```

Review the diff carefully before committing — a surprising change often
indicates a regression in the stub, not a fixture refresh.
