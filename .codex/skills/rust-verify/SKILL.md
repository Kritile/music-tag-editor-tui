---
name: rust-verify
description: Verify Rust code changes before completion by formatting, compiling, linting, and running relevant tests. Use after implementing or modifying Rust code, before declaring a coding task complete, and when diagnosing CI failures.
---

# Rust verification workflow

Use this skill after meaningful Rust code changes.

The goal is to verify the change rather than merely confirm that it compiles.

## 1. Inspect the project

Before running commands, inspect:

- `Cargo.toml`
- workspace configuration
- configured features
- existing CI configuration
- available development tools

Determine whether this is:

- a single crate
- a Cargo workspace
- a binary
- a library
- or a combination

Do not assume that all features can be enabled simultaneously.

## 2. Formatting

Run:

```bash
cargo fmt --all -- --check
