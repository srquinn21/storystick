# Story Stick

## Docs conventions

`docs/` uses numbered prefixes for the small, fixed set of foundational docs where reading order matters (e.g. `01-product-definition.md`, `02-architecture.md`). Don't number anything else, order between feature docs or decisions isn't meaningful the same way, and renumbering a growing set is churn nobody wants.

- `docs/features/` — one file per feature, plain descriptive name, no numeric prefix.
- `docs/decisions/` — architecture decision records (ADRs), numbered sequentially since chronological order is the point. `0001-event-sourcing-core.md`, `0002-...`. Use the template below.

Decisions drive the architecture doc, not the other way around: write the ADR first, then reflect the settled decision in the architecture doc. If a call is still open, it belongs in the architecture doc's Open Questions, not a new ADR.

### ADR template

```markdown
# ADR-NNNN: <Title>

**Status:** Accepted

## Context
What problem needed solving, what forces were at play.

## Decision
What was decided.

## Alternatives Considered
What else was on the table, and why it lost.

## Consequences
What gets easier, what gets harder, what this commits us to.
```
