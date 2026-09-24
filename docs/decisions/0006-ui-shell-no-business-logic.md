# ADR-0006: UI Shell Carries No Business Logic

**Status:** Superseded by [ADR-0008](0008-authoring-vs-output.md)

## Context

Given core logic is isolated from the UI (ADR-0004) and the core is Rust/WASM (ADR-0005), the UI shell's (TypeScript, Three.js) responsibilities need a hard boundary.

## Decision

The UI shell renders mesh buffers Rust computes, and captures raw interaction, drag position, raycasting hit-tests, using Three.js's own camera/scene math, then relays that upward. Rust decides what an interaction means for the model. A candidate drag position becoming an actual Placement is Placement's anchor-rule and grid-snapping logic, not the UI's.

## Alternatives Considered

Letting the UI shell interpret interactions directly, e.g. anchor-snapping logic written in TypeScript. Rejected, this duplicates model logic across the WASM boundary and reintroduces the portability problem ADR-0004 exists to avoid.

## Consequences

A clean input/output boundary: raw interaction data goes into Rust, computed meaning and mesh buffers come out. A future renderer other than Three.js only has to reimplement rendering and interaction capture, not model semantics.
