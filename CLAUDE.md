# CLAUDE.md

This file provides guidance to Claude Code when working with the
carina-provider-github repository.

## Repository Overview

This is the GitHub provider for [Carina](https://github.com/carina-rs/carina),
split out as a standalone repository. It manages a narrow set of GitHub
resources (a repository's Actions OIDC subject claim first) via the GitHub
REST API and exposes them as Carina resources. See
[`docs/design.md`](docs/design.md).

## Build and Test Commands

```bash
# Check
cargo check

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
  It ships only as a `wasm32-wasip2` component; native builds exist for
  tests, which run against a fake GitHub API.

## Dependencies on carina (main repo)

This repository depends only on `carina-plugin-sdk` from
`github.com/carina-rs/carina`, pinned by exact `rev` in `Cargo.toml` and
checked by `scripts/check-carina-pin.sh`. The SDK re-exports
`carina-core` (`carina_plugin_sdk::core`) and `carina-provider-protocol`
(`carina_plugin_sdk::protocol`).

The SDK owns the WIT and the generated bindings (carina-rs/carina#4191).
Do not add `wit-bindgen`, a `carina-plugin-bindings-*` crate, or a
`carina-plugin-wit` submodule to this repository.

For local development against a carina checkout, override the git
dependency in `.cargo/config.toml`:

```toml
[patch."https://github.com/carina-rs/carina"]
carina-plugin-sdk = { path = "../carina/carina-plugin-sdk" }
```

## Git Workflow

### Worktree-Based Development

```bash
git worktree add .worktrees/<branch-name> -b <branch-name> main   # Create worktree
git worktree list                                                  # List worktrees
git worktree remove .worktrees/<branch-name>                       # Delete worktree (from the main worktree)
```

## Code Style

- **Commit messages**: Write in English
- **Code comments**: Write in English
