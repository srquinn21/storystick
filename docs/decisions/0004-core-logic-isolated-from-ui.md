# ADR-0004: Core Logic Isolated From the UI/View Layer

**Status:** Superseded by [ADR-0008](0008-authoring-vs-output.md)

## Context

The tool should be able to port to another platform without rewriting the underlying model.

## Decision

Core logic is isolated from the UI/view layer.

## Alternatives Considered

Embedding business logic directly in UI components. Rejected, this ties replay, BOM computation, and validation logic to one rendering stack and blocks portability.

## Consequences

Enables the Rust-core, thin-UI-shell split in ADR-0005 and ADR-0006. A future UI rewrite or new platform target doesn't require reimplementing the model, only the rendering and interaction layer.
