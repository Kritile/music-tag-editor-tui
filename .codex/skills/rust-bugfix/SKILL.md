---
name: rust-bugfix
description: Diagnose and fix reproducible Rust bugs with a focused regression test. Use for incorrect behavior, panics, parsing failures, state bugs, edge cases, and regressions in Rust projects.
---

# Fix a Rust bug

1. Reproduce the report or establish the smallest reliable failing case. Write down expected and actual behavior. Trace the responsible path before editing.
2. Locate the source of the defect: input parsing, domain logic, state transition, event mapping, rendering, I/O, configuration, or concurrency. Fix the layer where the behavior originates.
3. When practical, add a small regression test that fails before the fix and passes afterward. Assert observable behavior, not internal structure. If the failure cannot be tested reliably, explain why in the completion report.
4. Apply the smallest correct fix. Keep nearby changes limited to what makes the fix safe and testable. Avoid compensating for domain errors in TUI drawing code.
5. Check directly related boundaries such as empty input, first and last item, stale selection after removal, invalid input, Unicode, tiny terminal, and failed I/O when they apply. Add only meaningful tests.
6. Run the regression test, the relevant surrounding tests, and then the `rust-verify` workflow if available. Review the final diff.
