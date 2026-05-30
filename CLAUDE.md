# CLAUDE.md

This file provides guidance to Claude Code when working with the
carina-provider-github repository.

## Repository Overview

This is the GitHub provider for [Carina](https://github.com/carina-rs/carina),
split out as a standalone repository. It manages a narrow set of GitHub
resources (Org-level Actions secrets/variables initially) via the GitHub
REST API and exposes them as Carina resources.

It depends on `carina-core`, `carina-plugin-sdk`, and
`carina-provider-protocol` via git dependencies from the main carina
repository. The scaffold PR does not yet wire these in — that happens
in the follow-up resource-implementation PR. See
[`docs/design.md`](docs/design.md).

## Build and Test Commands

```bash
# Build
cargo build

# Run all tests
cargo test

# Build WASM target
cargo build -p carina-provider-github --target wasm32-wasip2 --release

# Run clippy
cargo clippy -- -D warnings

# Format check
cargo fmt --check
```

## Crate Structure

- **carina-provider-github**: The GitHub provider implementation.
  Builds as both a native binary and a WASM component.

## Dependencies on carina (main repo)

This repository will depend on crates from
`github.com/carina-rs/carina`:

- `carina-core` — Core types, parser, traits
- `carina-plugin-sdk` — Plugin SDK for building providers
- `carina-provider-protocol` — Protocol definitions for provider
  communication

These are pinned by exact `rev` in `Cargo.toml`. The pin is verified by
`scripts/check-carina-pin.sh` (a CI job). For local development, you
can override them in `.cargo/config.toml`:

```toml
[patch."https://github.com/carina-rs/carina"]
carina-core = { path = "../carina/carina-core" }
carina-plugin-sdk = { path = "../carina/carina-plugin-sdk" }
carina-provider-protocol = { path = "../carina/carina-provider-protocol" }
```

## Git Workflow

### Worktree-Based Development

```bash
git worktree add .worktrees/<branch-name> -b <branch-name> main   # Create worktree
git worktree list                                                  # List worktrees
git worktree remove .worktrees/<branch-name>                       # Delete worktree (from the main worktree)
```

### Submodule Initialization

This repo uses a git submodule for `carina-plugin-wit/`. After
`git pull` or creating a new worktree, initialize the submodule:

```bash
git submodule update --init --recursive
```

Without this, builds will fail because `wit_bindgen::generate!` cannot
find the WIT files.

## Code Style

- **Commit messages**: Write in English
- **Code comments**: Write in English
