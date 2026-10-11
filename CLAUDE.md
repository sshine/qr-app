## End-to-end tests

Playwright drives a real browser from the Nix devshell. There is no npm install step.

```
just e2e
```

Changing the frontend earns a test only when the change carries a property worth stating: a flow
that must complete, a rule the interface claims to enforce, state that must survive a reload. Do not
add one for restyling, copy edits, or markup no invariant depends on.

Before writing or changing anything under `e2e/`, read `.claude/skills/playwright-tests/SKILL.md`.
