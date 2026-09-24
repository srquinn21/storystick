# ADR-0002: Not a Discrete-Event Simulator, Log Stays Ready For One

**Status:** Superseded by [ADR-0008](0008-authoring-vs-output.md)

## Context

Event sourcing (ADR-0001) means the model already has an ordered log of intent. That's also the data shape a discrete-event simulator (DES) would need for modeling time or resource contention, e.g. how long a cut actually takes, whether two operations compete for the same tool. The question is whether to build that simulation now.

## Decision

Not a DES simulator for v1. There's no need to model time or resource contention yet. But the log stays ready for a possible future DES layer: events record requests, not computed results, and timing data, if it's ever needed, lives on Tool/Operation definitions rather than being stamped per event.

## Alternatives Considered

Building DES simulation now. Rejected, it's unnecessary scope for v1 and works against the tenet of optimizing for build efficiency over simulation completeness.

Letting the log format drift toward whatever's convenient for v1, without regard for a future DES layer. Rejected, keeping events as requests rather than computed results costs nothing now and avoids a log-format migration if DES gets built later.

## Consequences

V1 ships with no time or scheduling simulation. A future DES layer remains possible without migrating existing project logs, since events already record intent rather than a computed outcome.
