# carina-provider-github — design

<!-- constrained-by https://github.com/carina-rs/carina-provider-github/issues/4 -->
<!-- constrained-by https://github.com/carina-rs/carina/issues/4193 -->

This document records the design of the first release of
`carina-rs/carina-provider-github`. It replaces the May 2026 bootstrap
design, which targeted Org-level Actions secrets and variables for the
registry dogfooding pipeline. That consumer went away when
carina-rs/carina#3342 was closed as not planned, and the design also
predated the current plugin SDK.

The first release manages exactly one resource. More resources are added
only when a concrete use case appears.

## Resource: `github.actions.OidcSubject`

A repository's GitHub Actions OIDC subject-claim customization: which
claims GitHub puts into the `sub` of the OIDC token that workflows in that
repository receive. Cloud trust policies (for example AWS IAM role trust
conditions) match on this value, so it is security-relevant configuration
that today is changed by hand.

```crn
github.actions.OidcSubject {
  repository = "carina-rs/carina"
  claim_keys = [repo, context, job_workflow_ref]
}
```

It wraps the REST endpoints
`GET` / `PUT /repos/{owner}/{repo}/actions/oidc/customization/sub`
(<https://docs.github.com/en/rest/actions/oidc>).

### Attributes

| Attribute    | Shape                                   | Notes |
| ------------ | --------------------------------------- | ----- |
| `repository` | string, `owner/repo`                    | Required, create-only, identity. |
| `claim_keys` | ordered list of `ClaimKey`, length ≥ 1, unique | Optional. Omitted means GitHub's default subject. |

There is no `mode` attribute. Whether the repository uses the default is
expressed by whether `claim_keys` is present, so "custom mode with no
keys" and "default mode with keys" cannot be written.

`repository` is matched against `^[A-Za-z0-9-]+/[A-Za-z0-9._-]+$` by a
refined string, so a malformed value is rejected by `carina validate`.

`claim_keys` is ordered because GitHub builds the subject in list order
(`repo:…:context:…:job_workflow_ref:…`). Reordering it changes the
subject, so it is a real change and is compared positionally.

Empty lists are rejected by the list length minimum. Duplicates are
rejected by the declarative unique-elements refinement from
carina-rs/carina#4193, so `carina validate`, plan and the LSP all report
them. GitHub also requires unique keys.

### `ClaimKey`

A closed enum, `github.actions.OidcSubject.ClaimKey`. Values are GitHub's
own spellings, which are already snake_case, so there are no aliases. The
DSL accepts the bare form (`repo`), as other enum lists do.

The values are `repo` and `context` (subject segments GitHub documents for
customization) plus the token claims GitHub publishes in `claims_supported`
at `https://token.actions.githubusercontent.com/.well-known/openid-configuration`,
except the registered JWT claims (`sub`, `aud`, `exp`, `iat`, `iss`, `jti`,
`nbf`): `actor`, `actor_id`, `base_ref`, `check_run_id`, `enterprise`,
`enterprise_id`, `environment`, `environment_node_id`, `event_name`,
`head_ref`, `issuer_scope`, `job_workflow_ref`, `job_workflow_sha`, `ref`,
`ref_protected`, `ref_type`, `repository`, `repository_id`,
`repository_owner`, `repository_owner_id`, `repository_visibility`,
`run_attempt`, `run_id`, `run_number`, `runner_environment`, `sha`,
`workflow`, `workflow_ref`, `workflow_sha`.

Custom-property keys (`repo_property_<name>`) have no fixed name and are
not supported in the first release. A claim GitHub adds later needs a
provider release.

### Operations

- **Create**: `PUT {"use_default": false, "include_claim_keys": [...]}`
  when `claim_keys` is set, otherwise `PUT {"use_default": true}`. The
  setting always exists on a repository, so create is "set", not
  "insert"; the identifier is the `owner/repo` string.
- **Read**: `GET`. `use_default: true` reads back as `claim_keys` absent.
  `use_default: false` with keys reads back as those keys in order. A
  repository that is not on the default but has no keys (it inherits an
  org template, which this provider never writes) reads back as an empty
  list. The DSL cannot author an empty list, so plan shows a change and
  apply rewrites it. A 404 for the repository reads as not found.
- **Update**: the same `PUT` with the desired state. Removing
  `claim_keys` resets to the default.
- **Delete**: `PUT {"use_default": true}`, which GitHub documents as the
  way to reset a repository to the default subject. A 404 for the
  repository counts as already deleted.
- **Default `prevent_destroy`** (carina#4150) is declared on the type from
  the start, because deleting it changes the subject every downstream
  trust policy matches on. Users opt out with
  `directives { prevent_destroy = false }`.

All operations are synchronous. Create/update/delete tokens are `Never`.
A request that fails before it is transmitted, or with a definite 4xx, is
reported as not applied. A timeout or 5xx after transmit keeps the attempt
pending, so the next apply re-reads instead of assuming success.

`use_immutable_subject` and `sub_claim_prefix` are not managed. Requests
never send `use_immutable_subject`, so this provider leaves it as is.

## Authentication

The provider reads `GITHUB_TOKEN` from its environment. The host passes
only that variable to a GitHub-kind provider (carina-plugin-host
`GITHUB_ENV_ALLOWLIST`); AWS credentials never reach it, and the provider
must not import the AWS credentials interface. There is no `auth` block
and no in-provider token minting. A missing token is a configuration
error at initialization, before any request.

Required permission: Actions write on the repository (fine-grained PAT),
or the `repo` scope (classic PAT). Whether an Actions workflow
`GITHUB_TOKEN` with `permissions: actions: write` is enough is verified by
the acceptance run, not assumed.

## Network

`export_provider!(GithubProvider, http, egress: ["api.github.com"])`. An
exact host matches only that host. The host enforces the declaration per
request, consent is recorded by `carina init` in `carina-providers.lock`,
and private, loopback and metadata addresses are denied for the GitHub
kind. HTTP goes through `carina_plugin_sdk::wasi_http::send_request`.

## Trust and naming

The host classifies `github.com/carina-rs/carina-provider-github` and the
registry name `carina-rs/github` as the GitHub kind, and requires the
configured provider name to be exactly `github`. For local development,
load a built WASM through `source = "file:///…/carina-provider-github.wasm"`
with `--trust-local-provider <path>=github`.

## Crate shape

- One crate, `carina-provider-github`, built as `wasm32-wasip2` only.
  There are no native release binaries and no `native` feature.
- Dependencies: `carina-plugin-sdk` pinned by exact `rev` (the SDK
  re-exports `carina-core` and `carina-provider-protocol`), `http`,
  `serde`, `serde_json`. No `wit-bindgen` and no `carina-plugin-wit`
  submodule: since carina-rs/carina#4191 the SDK owns the WIT and the
  bindings.
- The GitHub API sits behind a small trait. The WASM build uses an HTTP
  implementation, and native tests use a fake. Tests drive create through
  `carina_plugin_sdk::dispatch_create`.
- `scripts/check-carina-pin.sh` and `.carina-core-min-rev` keep the
  pinning pattern of the AWS providers.
- A copy of carina's `scripts/check-provider-sdk-bindings.sh` runs in CI,
  so a direct `wit-bindgen` or bindings dependency cannot come back.

## Testing

- Unit tests through the fake API: create/read/update/delete, default
  versus custom read-back, the org-template read-back, 404 handling, and
  that requests never carry `use_immutable_subject`.
- Schema tests: validation rejects a malformed `repository`, an empty
  `claim_keys` and a duplicate key; bare and qualified enum spellings are
  accepted.
- A real-WASM acceptance test runs plan/apply/destroy against a repository
  in the `carina-test` org only. carina-rs repositories are never used as
  test targets.

## CI and release

CI runs `check`, `test`, `clippy -D warnings`, `fmt --check`, a
`wasm32-wasip2` release build, the pin check and the bindings guard.

Release uses `carina-rs/provider-release` (the reusable WASM build plus
the `github-release`, `publish` and `yank` actions, pinned by SHA), as the
AWS providers do. The first release goes to the dev registry. A formal
release tag needs the user's approval.

## Not in the first release

- Organization-level OIDC templates, repository secrets/variables,
  rulesets, environments, and any other GitHub resource.
- `repo_property_*` claim keys and `use_immutable_subject`.
- GitHub App authentication.
- Following a repository rename. The identity is the `owner/repo` string.
  GitHub redirects renamed repositories, but this provider does not
  rewrite state to the new name.
