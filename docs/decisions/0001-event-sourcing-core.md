# ADR-0001: Event Sourcing as the Core Pattern

**Status:** Superseded by [ADR-0008](0008-authoring-vs-output.md)

## Context

The model needs undo, redo, and the ability to edit an earlier operation and have everything after it update automatically through replay.

## Decision

Core pattern is event sourcing. The operation log is an ordered record of intent, and current state is computed by folding that log.

## Alternatives Considered

Mutable state with a manual undo/redo stack. This handles sequential undo fine, but doesn't naturally support editing an operation in the middle of history and recomputing everything after it, a stack only unwinds back to a point, it doesn't replay forward from an edited point.

## Consequences

Replay, BOM computation, and validation all have to be pure functions over the log. The log is the source of truth, not a snapshot of current state. This is also what makes ADR-0003's schema-versioning approach necessary, old logs have to keep replaying correctly as the model grows.
