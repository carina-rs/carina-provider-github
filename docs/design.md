# carina-provider-github — bootstrap design

<!-- constrained-by https://github.com/carina-rs/carina/issues/3342 -->
<!-- constrained-by https://github.com/carina-rs/registry/blob/main/docs/dogfooding-design.md -->

This document records the design decisions for the initial release of
`carina-rs/carina-provider-github`. It is intentionally narrow: only what
the registry dogfooding pipeline needs to declare a GitHub App's private
key + ID into the `carina-rs` Org-level Actions secrets/variables via
Carina, not a complete GitHub provider.

The implementation is split across PRs. **This PR adds the scaffold and
this document only**; the two resources land in follow-up PRs so each
review stays focused.

## Goals

- Manage GitHub Org-level Actions secrets and variables from a Carina
  `.crn` file, dogfooding the registry pipeline end-to-end.
- Mirror the structural conventions of `carina-rs/carina-provider-aws`
  / `-awscc` (Cargo workspace layout, WIT pin, CI matrix, release
  workflow) so that maintenance patterns transfer.
- Keep the surface small enough that the v1 binary builds cleanly to
  both native and `wasm32-wasip2`.

## Non-goals (explicit)

These are out of scope for the v1 release and will be added only when a
concrete use case appears:

- Repository secrets, repository variables, environment secrets.
- Repository management, team management, branch protections,
  CODEOWNERS, etc.
- OpenAPI / schema-driven codegen (the `aws` / `awscc` providers use
  schema-driven codegen; with two hand-rolled resources we do not need
  it yet).
- Listing this provider in `carina-rs/registry`. The registry pipeline
  that this provider is bootstrapping will eventually ingest it, but
  not in the initial release.

## Initial resource scope

| Carina resource                       | Wraps                                                                |
| ------------------------------------- | -------------------------------------------------------------------- |
| `github.actions.OrganizationSecret`   | `PUT /orgs/{org}/actions/secrets/{secret_name}`                      |
| `github.actions.OrganizationVariable` | `PUT /orgs/{org}/actions/variables/{name}`                           |

### Resource identity

GitHub returns no opaque ID for either resource — only the name is
stable across reads. Carina state keys both resources on `name` (the
secret/variable name) scoped under the configured `organization`. The
combined natural key is `{organization}/{name}`; this maps directly to
the REST URL path components, so reads, updates, and deletes can be
issued without any extra round-trip.

### Attribute shape (sketch — final form lands in the implementation PR)

```crn
provider "github" {
  organization = "carina-rs"
  // auth: see "Authentication" below
}

github.actions.OrganizationSecret {
  name              = "MY_SECRET"
  plaintext_value   = "..."           // write-only; never read back
  visibility        = github.actions.SecretVisibility.all
  // selected_repository_ids = [ ... ]  // only when visibility = selected
}

github.actions.OrganizationVariable {
  name       = "MY_VAR"
  value      = "..."
  visibility = github.actions.VariableVisibility.all
}
```

Notes:

- `plaintext_value` is a write-only attribute. GitHub's read endpoint
  never returns the secret value, so the provider treats it as
  `Value::Unknown` on read and reconciles drift via a hash stored in
  state (similar to how Terraform's `github` provider handles
  `plaintext_value`). The exact reconciliation shape is finalized in
  the resource-implementation PR.
- `visibility` enums use Carina's standard DSL spelling: snake_case
  identifiers (`all`, `private`, `selected`) bound to a namespaced
  type, per the project-wide DSL enum convention.

## Authentication

Both **Personal Access Token (PAT)** and **GitHub App** authentication
ship in v1. The provider's `auth` block is a tagged variant:

```crn
provider "github" {
  organization = "carina-rs"

  // Either:
  auth = github.Auth.pat {
    token = env.GITHUB_TOKEN  // or a literal string
  }

  // Or:
  auth = github.Auth.app {
    app_id          = 123456
    installation_id = 789012
    private_key_pem = file("github-app.pem")
  }
}
```

Rationale for shipping both at once even though the issue body called
out PAT as the bootstrap path:

- The chicken-and-egg argument (the App is the thing this provider is
  used to declare) applies to **the carina-rs Org's own App**, not to
  Apps in general. As soon as another consumer wants to drive the
  provider from CI under a pre-existing App installation, App auth is
  immediately load-bearing.
- App auth shares ~all infrastructure with PAT (request signing is the
  only delta — a short-lived installation token is minted via JWT and
  then carried in the same `Authorization: Bearer …` header that PAT
  uses). Splitting the two into separate PRs would duplicate the
  bring-up cost of the auth scaffold.
- The implementation cost is bounded: JWT signing uses `jsonwebtoken`
  (pure Rust, builds on `wasm32-wasip2`), and the installation-token
  fetch is one REST call cached for the lifetime of the provider
  process.

### Request execution

All HTTP traffic goes through `wasi:http/outgoing-handler@0.2`, the
same path the AWS providers use after carina#3254. The provider
constructs `http::Request` instances and sends them via
`carina-plugin-sdk`'s WASI HTTP wrapper; no `reqwest` / `hyper` /
`ureq` dependency is introduced.

## Secret-value encryption

GitHub requires Org Actions secret values to be encrypted with the
Org's libsodium **sealed-box** public key before upload. The provider:

1. Fetches the Org's public key via
   `GET /orgs/{org}/actions/secrets/public-key` (cached for the
   lifetime of the provider process).
2. Encrypts `plaintext_value` with sealed-box (Curve25519 + XSalsa20 +
   Poly1305) using the [`crypto_box`](https://crates.io/crates/crypto_box)
   crate.
3. Sends the base64-encoded ciphertext + `key_id` as the request body.

`crypto_box` is chosen over `sodiumoxide` because it is pure Rust and
builds cleanly for `wasm32-wasip2`. A spike compiled
`crypto_box = "0.9"` with the `seal` feature against `wasm32-wasip2`
without modification; no C toolchain is needed on the build host.

The `plaintext_value` attribute is treated as **write-only**: it never
appears in plan output diffs after the initial create, and Carina state
records only a salted hash of the most recent successful upload (so
plan can detect "the authored value changed since last apply" without
storing the secret).

## carina-core pin

This repo follows the same `.carina-core-min-rev` + per-`Cargo.toml`
`rev` pinning pattern as `carina-provider-aws` / `-awscc`, gated by
`scripts/check-carina-pin.sh` (copied from awscc with the repo name
swapped). The initial pin is the carina `main` head at bootstrap time:
`3a1c51d397ffa1e7725d32ebbac99c2edae5d27d`.

## Repo layout

```
carina-provider-github/
├── Cargo.toml                     # workspace
├── .carina-core-min-rev           # minimum required carina-core rev
├── .gitmodules                    # carina-plugin-wit submodule pin
├── carina-plugin-wit/             # submodule (shared WIT)
├── carina-provider-github/        # provider crate (lib + bin)
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs                # WASI entrypoint
│       ├── lib.rs                 # provider type + factory
│       ├── provider/              # auth, http client, resources
│       └── schemas/               # resource schemas (hand-written for v1)
├── docs/
│   └── design.md                  # this document
├── scripts/
│   └── check-carina-pin.sh        # pin guard, copied from awscc
├── .github/workflows/
│   ├── ci.yml                     # check / test / clippy / fmt / wasm-build
│   └── release.yml                # per-platform tarballs + wasm artifact
├── CLAUDE.md
├── README.md
└── LICENSE
```

No `cfn-schema-cache/`, no `carina-aws-types/`, no `carina-codegen-*/`
crate — GitHub's API is small enough at this scope that hand-written
schemas are cheaper than wiring up codegen.

## CI

The final `.github/workflows/ci.yml` (post resource-implementation PR)
runs:

| Job          | Command                                                |
| ------------ | ------------------------------------------------------ |
| `check`      | `cargo check`                                          |
| `test`       | `cargo test`                                           |
| `clippy`     | `cargo clippy -- -D warnings`                          |
| `fmt`        | `cargo fmt --check`                                    |
| `wasm-build` | `cargo build -p carina-provider-github --target wasm32-wasip2 --release` then `vela optimize` and upload-artifact |
| `carina-pin` | `./scripts/check-carina-pin.sh`                        |

In this scaffold PR only `check` / `test` / `clippy` / `fmt` are
enabled. `wasm-build` and `carina-pin` are added by the
resource-implementation PR once the carina-core git deps are wired in
— `carina-pin` would otherwise hard-fail on the empty dependency list,
and a no-dependency stub for `wasm32-wasip2` is not informative.

No `docs-drift` or `codegen-check` jobs yet — those exist in
`-aws`/`-awscc` because of schema-driven codegen we are not adopting.

## Release

`.github/workflows/release.yml` mirrors the awscc release matrix:
`aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-unknown-linux-gnu`,
`aarch64-unknown-linux-gnu`, plus a `wasm32-wasip2` artifact post-`vela`.
Triggered on `v*` tag push.

## Open questions

These are flagged here rather than deferred silently — each one needs a
decision before the resource-implementation PR is opened, but none
block the scaffold landing.

1. **Plaintext-value drift detection.** Hashing the authored value and
   storing the hash in state is the leading candidate (matches
   established Terraform-ecosystem precedent). Outstanding sub-question:
   what hash + salt scheme? `sha256(name || ":" || plaintext)` with a
   per-state salt is one option. Decided in the resource PR.
2. **`visibility = selected` repository ID validation.** Plan-time
   validation would require either a read of the Org's repos or trust
   in user-supplied IDs. Initial position: trust user input, surface
   GitHub's 4xx response on apply. Revisited if it turns out to be a
   common footgun.
3. **App installation-token caching across `plan` → `apply`.** Each
   `carina` subcommand invocation is a fresh provider process, so the
   token is minted once per command. No cross-invocation cache for v1.

## References

- Issue: <https://github.com/carina-rs/carina/issues/3342>
- Tracking issue: <https://github.com/carina-rs/registry/issues/5>
- Design source: <https://github.com/carina-rs/registry/blob/main/docs/dogfooding-design.md>
- Blocks: <https://github.com/carina-rs/infra/issues/94>
