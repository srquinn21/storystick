# ADR-0005: Rust Core Compiled to WASM, Web Platform Not Native

**Status:** Superseded by [ADR-0008](0008-authoring-vs-output.md)

## Context

The core logic (ADR-0004) covers exactly the code where subtle bugs are costliest: replay, BOM computation, validation. Some of that logic is also agent-authored, which raises the value of compile-time correctness checking specifically. Separately, native distribution had been under consideration for precision Pencil input, which would have required a Swift/Rust FFI boundary.

## Decision

Core is Rust, compiled to WASM. Platform is a web app, not native.

## Alternatives Considered

TypeScript for the core. Rejected, weaker compile-time guarantees for exactly the logic, replay, BOM, validation, where correctness matters most.

Native app with a Swift/Rust FFI boundary, to support precision Pencil input. Rejected, precision Pencil input turned out not to be a real requirement, which removed the only reason to accept native-only distribution and the FFI boundary that comes with it.

## Consequences

Web distribution, cross-platform without an app store. A WASM boundary sits between the Rust core and the JS/TS UI shell. Agent-authored code in the core gets compiler-enforced correctness checks before it ships.
