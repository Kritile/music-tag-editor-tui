---
name: rust-review
description: Review a Rust diff or pull request for concrete defects, regressions, unsafe assumptions, error handling, concurrency hazards, and missing behavioral tests. Use when asked to review Rust code or a completed Rust implementation.
---

# Review Rust code

1. Read the change and enough surrounding code, callers, tests, and configuration to understand the behavior. Prioritize correctness, safety, data loss, concurrency, recoverable errors, edge cases, and compatibility over style.
2. Inspect state transitions, indices and arithmetic, empty collections, input parsing, error paths, ownership choices, and public API changes. Flag `unwrap()` or `expect()` only where a real recoverable condition can trigger a panic. Note needless clones only when materially relevant.
3. For async or concurrent paths, check blocking operations, locks across `.await`, cancellation, unbounded work or queues, and realistic deadlocks. For TUI code, check event handling, navigation, small layouts, and terminal cleanup.
4. Check whether important changed behavior is tested and whether a bug fix has a regression test. Do not ask for tests of trivial implementation details.
5. Report concrete findings first, ordered by severity. For each, include file and location, the failure scenario, and a practical fix. Do not invent hypothetical failures or label a stylistic preference as a defect. If no material issues are found, say so and mention any meaningful verification limits.
