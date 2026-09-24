# ADR-0003: Additive-Only Schema Evolution, Versioned Per Event Type

**Status:** Superseded by [ADR-0008](0008-authoring-vs-output.md)

## Context

The model's vocabulary (Tool, Operation, Feature, Mill, and so on) will keep growing over time. Existing project logs, built under event sourcing (ADR-0001), have to keep replaying correctly as that happens.

## Decision

Schema evolves additively only, versioned per event type rather than per document.

## Alternatives Considered

Per-document or global schema versioning, with a whole-project migration whenever any event type changes. Rejected, this forces a migration step even for projects that never touched the changed event type.

Allowing breaking changes to an event's shape. Rejected, this breaks replay for any existing log that used the old shape.

## Consequences

Old logs keep replaying without migration. New fields and event types can be added freely. An existing event type's meaning can't be silently repurposed or broken once it's shipped, changes to it have to stay additive.
