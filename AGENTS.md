# AGENTS.md

## Purpose

This file defines the default rules for AI coding agents working in this repository.

The project is written in Rust and may include a terminal user interface (TUI).

Primary goals:

* correctness
* maintainability
* testability
* predictable behavior
* idiomatic Rust
* minimal unnecessary complexity

Prefer simple, explicit solutions over clever abstractions.

---

## Project Discovery

Before making changes:

1. Read `Cargo.toml` and determine:

   * Rust edition
   * workspace structure
   * enabled features
   * important dependencies
   * binary and library targets

2. Inspect the relevant modules before editing them.

3. Look for existing:

   * conventions
   * abstractions
   * tests
   * error types
   * event handling patterns
   * state management patterns

4. Reuse existing project patterns when they are reasonable.

Do not introduce a new architectural pattern if an existing one already solves the problem adequately.

If files such as these exist, treat them as additional sources of truth:

* `README.md`
* `ARCHITECTURE.md`
* `CONTRIBUTING.md`
* `docs/`
* nested `AGENTS.md`

More specific instructions in nested `AGENTS.md` files override this file for their directory tree.

---

# Rust Guidelines

## Toolchain

Follow the Rust version and edition configured by the repository.

Do not upgrade:

* Rust edition
* MSRV
* major dependencies
* workspace-wide tooling

unless the task explicitly requires it.

Prefer stable Rust unless the repository already uses nightly Rust.

---

## Code Style

Write idiomatic Rust.

Prefer:

* clear ownership
* borrowing over unnecessary cloning
* small focused functions
* explicit domain types
* enums for finite state
* iterators where they improve readability
* pattern matching for state transitions
* standard library types when sufficient

Avoid:

* unnecessary `clone()`
* unnecessary allocations
* deeply nested control flow
* large functions with multiple responsibilities
* premature generic abstractions
* premature optimization
* hidden global state

Run `rustfmt` on changed Rust code.

---

## Error Handling

Do not silently ignore errors.

Avoid `unwrap()` and `expect()` in production code unless the condition is a genuine invariant that cannot reasonably fail.

If `expect()` is appropriate, provide a message explaining the invariant.

Prefer propagating recoverable errors with `?`.

Errors caused by:

* user input
* filesystem state
* network operations
* configuration
* terminal capabilities
* external processes

must normally be handled gracefully rather than causing a panic.

Prefer structured domain errors in reusable/library layers.

Application boundaries may use higher-level error aggregation when appropriate.

Do not add an error-handling dependency solely because it is popular; follow the existing project approach.

---

## Types and Domain Modeling

Prefer making invalid states difficult to represent.

Use dedicated types when primitive values would otherwise have ambiguous meaning.

Prefer:

```rust
enum AppMode {
    Normal,
    Insert,
    Search,
}
```

over loosely related booleans such as:

```rust
is_insert_mode: bool,
is_search_mode: bool,
```

when those states are mutually exclusive.

Keep domain logic independent from presentation code whenever possible.

---

# Architecture

Prefer separating application behavior into layers with clear responsibilities.

A typical structure may look like:

```text
src/
├── main.rs
├── app.rs
├── event.rs
├── action.rs
├── tui.rs
├── ui/
│   ├── mod.rs
│   └── ...
├── domain/
│   └── ...
└── services/
    └── ...
```

This structure is illustrative, not mandatory.

Do not reorganize the project solely to match this example.

---

## Application State

Application state should have a clear owner.

Prefer explicit state transitions.

For interactive applications, a pattern similar to:

```text
Event -> Action -> State Update -> Render
```

is preferred when it fits the project.

Business logic should normally live in state/domain/update code rather than event handlers or rendering functions.

---

# TUI Guidelines

If this project contains a terminal user interface, keep terminal-specific code separated from application logic.

## Rendering

Rendering functions should ideally be:

* deterministic
* side-effect free
* dependent only on application state and layout information

Rendering code should not:

* perform network requests
* read files
* mutate domain state
* start background jobs
* contain business logic

Prefer:

```text
state -> render
```

rather than:

```text
render -> mutate state -> perform I/O
```

---

## Event Handling

Centralize translation of terminal events into application actions when practical.

For example:

```text
terminal event
      ↓
event mapping
      ↓
application action
      ↓
state transition
```

Keep key bindings discoverable and reasonably centralized.

Avoid spreading key handling across many unrelated widgets unless the architecture intentionally uses component-local event handling.

Handle relevant terminal events such as:

* keyboard input
* resize events
* quit requests

Mouse support should only be added when required.

---

## Terminal Lifecycle

Terminal setup and restoration must be reliable.

The application must attempt to restore the terminal when exiting due to:

* normal termination
* recoverable errors
* early returns

Where practical, prefer RAII-style guards for terminal cleanup.

Do not leave the user's terminal in:

* raw mode
* alternate screen
* hidden cursor state

after normal application termination.

---

## Layout

Do not assume a fixed terminal size.

UI code must tolerate:

* small terminals
* narrow terminals
* resize events
* empty or near-empty layout regions

Avoid arithmetic that can underflow when calculating layout sizes.

Prefer saturating or checked calculations where dimensions originate from runtime terminal sizes.

---

# Testing

Tests are required for meaningful behavior changes.

A feature is not considered complete until relevant behavior is tested.

## Testing Priorities

Prefer testing behavior rather than implementation details.

The preferred testing order is:

1. pure domain/state logic
2. action/state transitions
3. parsers and transformations
4. service boundaries
5. TUI rendering
6. end-to-end behavior where valuable

---

## Unit Tests

Add unit tests for:

* state transitions
* parsing
* validation
* calculations
* filtering
* sorting
* selection logic
* navigation logic
* boundary conditions

Pure application logic should normally be testable without creating a real terminal.

---

## Regression Tests

Every bug fix should include a regression test when reasonably possible.

The regression test should:

1. reproduce the previous incorrect behavior
2. verify the corrected behavior

Do not fix a reproducible logic bug without a test unless testing it is impractical.

If a regression test is impractical, explain why in the final summary.

---

## TUI Tests

Do not rely on a real interactive terminal for most UI tests.

When using Ratatui, prefer `TestBackend` or equivalent buffer-based rendering tests.

Useful TUI tests include:

* expected text is rendered
* selected item highlighting
* correct view for each application state
* empty-state rendering
* error-state rendering
* small-terminal rendering
* resizing behavior
* modal/dialog rendering

Prefer semantic assertions when practical.

Snapshot tests may be used when they improve readability and maintainability, but avoid excessive snapshots for trivial UI details.

A small UI change should not require updating dozens of unrelated snapshots.

---

## Property-Based Tests

Property-based testing may be useful for logic with large input spaces, such as:

* parsers
* serialization
* filtering
* sorting
* range calculations
* navigation indexes

Do not introduce property-based testing when a few clear example-based tests provide equivalent confidence.

---

## Integration Tests

Use `tests/` for public behavior that is better exercised externally.

Integration tests should avoid depending on:

* external network services
* machine-specific paths
* user configuration
* interactive terminal input

unless explicitly testing an integration that requires them.

Prefer temporary directories and deterministic fixtures.

---

## Test Determinism

Tests must be deterministic.

Avoid depending on:

* current time without injection
* random values without fixed seeds
* execution order
* network connectivity
* user's home directory
* global mutable state

Inject or abstract nondeterministic dependencies where necessary.

---

# Required Checks

Before considering a task complete, run the relevant checks.

At minimum:

```bash
cargo fmt --all -- --check
cargo check
cargo test
cargo clippy --all-targets -- -D warnings
```

For workspaces, prefer:

```bash
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

If all project features are designed to work together, also check:

```bash
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Do not blindly use `--all-features` if features are intentionally mutually exclusive.

When doctests exist, ensure they are also tested.

If `cargo-nextest` is configured, prefer:

```bash
cargo nextest run
```

for the normal automated test suite, while retaining `cargo test --doc` when doctests need to be checked separately.

Do not claim checks passed unless they were actually executed.

If a command cannot be executed, report that explicitly.

---

# Test Coverage

Coverage percentage is a signal, not the objective.

Prioritize coverage of:

* domain logic
* state transitions
* error paths
* edge cases
* previously reported bugs

Avoid writing low-value tests solely to increase a coverage percentage.

If the repository uses `cargo-llvm-cov`, use it to identify untested important branches.

---

# Dependencies

Before adding a dependency:

1. check whether the standard library already solves the problem
2. check whether an existing project dependency already provides the functionality
3. evaluate whether the dependency is maintained and appropriate
4. avoid adding large dependency trees for trivial functionality

Do not add dependencies without a concrete reason.

Do not replace established project dependencies solely based on personal preference.

---

# Async and Concurrency

Do not introduce async Rust unless the problem benefits from it.

If the project already uses an async runtime, follow the existing runtime.

Avoid blocking operations inside async tasks.

Long-running work should not block the TUI event/render loop.

Background work should communicate with application state through explicit mechanisms such as:

* channels
* messages
* actions

Avoid sharing mutable state unnecessarily.

---

# Performance

Correctness and clarity come before micro-optimization.

Optimize only when:

* there is a demonstrated issue
* the hot path is obvious
* profiling or measurement supports the change

For TUI applications, avoid expensive work during every render frame when the result can be calculated only when state changes.

---

# Unsafe Rust

Avoid `unsafe` unless genuinely required.

Any new `unsafe` block must:

* have a concrete reason
* document its safety invariants
* keep the unsafe scope as small as practical

Never introduce `unsafe` merely to avoid straightforward ownership or borrowing work.

---

# Logging and Diagnostics

Do not print debugging output directly into an active TUI.

Prefer the project's logging/tracing infrastructure.

Diagnostic information should not corrupt terminal rendering.

Remove temporary debugging statements before completing a task.

---

# Documentation

Public APIs should have documentation when their purpose is not obvious.

Comments should explain:

* why something exists
* invariants
* non-obvious tradeoffs
* surprising behavior

Avoid comments that merely restate the code.

Keep documentation synchronized with behavior.

---

# Scope of Changes

Keep changes focused on the requested task.

Do not perform unrelated refactoring unless it is necessary to implement the task safely.

Small opportunistic cleanup is acceptable when:

* it is directly adjacent to changed code
* it reduces complexity
* it does not significantly expand the diff

Avoid repository-wide formatting or renaming unless explicitly requested.

---

# Backwards Compatibility

Do not intentionally break existing:

* CLI arguments
* configuration formats
* persisted data
* public APIs
* key bindings

unless the task explicitly permits a breaking change.

When behavior must change, prefer migrations or backwards-compatible handling when practical.

---

# Agent Workflow

For non-trivial changes:

1. inspect the relevant code
2. understand the current behavior
3. identify the smallest appropriate design
4. implement the change
5. add or update tests
6. run formatting
7. run tests
8. run Clippy
9. review the resulting diff

Do not stop immediately after code compiles.

Compilation is not sufficient evidence of correctness.

---

# Completion Criteria

A task is complete when:

* requested behavior is implemented
* relevant tests exist and pass
* formatting passes
* Clippy passes for the relevant scope
* no obvious debug code remains
* errors are handled appropriately
* documentation is updated when necessary
* unrelated behavior has not been changed

In the final response, summarize:

* what changed
* important implementation decisions
* tests added or updated
* verification commands executed
* any remaining limitations or follow-up work

