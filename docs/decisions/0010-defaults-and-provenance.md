# ADR-0010: Every Field Gets a Value, Provenance Tracks Whether It's Been Reviewed

**Status:** Accepted

## Context

Several unrelated metadata fields, grain direction, material/species, template-derived thickness, have each independently landed on the same shape of decision during design: give a sane default rather than leave the field empty, but let the user override it. Solving that field-by-field risks inconsistent handling, and risks losing the distinction between "this is a real guess nobody has confirmed" and "this is what the user actually decided," which matters, the same reasoning that made Hardware resolution "can't be left undecided indefinitely" in the original model applies here too: a default that's never been looked at can silently produce a wrong BOM, ordering birch when maple was intended, with no warning it ever happened.

## Decision

No metadata field is ever null or unset at the software level. Every field resolves to one of two kinds of concrete state:

- **A guessed value**, where a reasonable system-wide default exists, birch for plywood, oak for boards, grain running the long axis for solid lumber. The field holds that value immediately, no user action required.
- **An explicit "undecided" marker**, where no reasonable system-wide guess exists, tool assignment being the clearest example, there's no sane default for which saw is in someone's shop. This is still a real, tracked, displayable value, not the absence of one, it's just a placeholder that means "not yet decided" rather than a guess.

Independent of the value, every field also carries **provenance**: system default (nobody's touched it), template-inherited (came from a named type template, see ADR-0009), or user-confirmed (explicitly set or reviewed). Provenance is what lets the tool tell "this is probably right" apart from "this is definitely right," without ever making the field itself empty.

When generating a build plan, fields still sitting on system-default provenance get surfaced as a non-blocking flag, the same treatment already established for undecided tools and unresolved hardware: never blocking, always visible.

## Alternatives Considered

Leaving fields genuinely unset until the user provides input. Rejected: forces every downstream consumer, BOM, bin-packing, to special-case "what if this is missing," and breaks the pattern already used everywhere else in this project, sane default plus override, never a forced blocking decision.

Applying defaults without tracking provenance, just silently defaulting forever with nothing ever flagged. Rejected: this is exactly the failure mode Hardware resolution was designed to prevent, a default nobody reviewed shouldn't be able to reach a purchase decision looking identical to one the user actually confirmed.

## Consequences

Every metadata field in the data model needs a provenance tag alongside its value, one small, consistent piece of schema, not bespoke per-field handling. Build plan generation gains a standard, reusable check: summarize which fields are still on system-default provenance, same category and same non-blocking treatment as any other build warning.

Left open, not decided here: where the actual default *values* themselves (birch, oak, and whatever else) get configured. Presumably user-editable rather than fixed in code, but the mechanism isn't decided, same deferral as ADR-0009's template-authoring UX.
