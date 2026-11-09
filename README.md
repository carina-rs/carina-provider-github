# carina-provider-github

GitHub provider for [Carina](https://github.com/carina-rs/carina).

Manages a narrow set of GitHub resources via the GitHub REST API.

## Status

Pre-release. Ships as a `wasm32-wasip2` component using the Carina plugin
SDK. See [`docs/design.md`](docs/design.md) for the design.

## Initial resource scope

| Carina resource              | Wraps                                                        |
| ---------------------------- | ------------------------------------------------------------ |
| `github.actions.OidcSubject` | `GET` / `PUT /repos/{owner}/{repo}/actions/oidc/customization/sub` |

Stay narrow on purpose. New resources are added when a real use case
appears, not preemptively.

## OIDC subject customization

```crn
github.actions.OidcSubject {
  repository = "carina-rs/carina"
  claim_keys = [repo, context, job_workflow_ref]
}
```

`claim_keys` is an ordered, non-empty list of unique claims. Omit it to
use GitHub's default subject. Deleting the resource also resets the
subject to that default. Destruction protection is enabled by default;
opting out requires `directives { prevent_destroy = false }`.

Authentication uses `GITHUB_TOKEN`, with Actions write permission on the
repository for a fine-grained PAT, or the `repo` scope for a classic PAT.
The provider block has no configuration attributes.

Initialization registers the token with Carina's diagnostics session before
authorizing any HTTP request. Registration failures stop initialization;
the host masks registered tokens in returned errors and typed diagnostics.
Guest stdout and stderr are discarded. This component requires Carina
protocol 25 (`carina:provider@0.2.0`).

## Local development

Use a Carina CLI compatible with the SDK revision pinned in `Cargo.toml`.
Build the component (the `wasm32-wasip2` Rust target must be installed):

```sh
cargo build -p carina-provider-github --target wasm32-wasip2 --release
```

In a separate directory, create `main.crn` using an absolute path to the
built component. Use a repository in `carina-test` for acceptance testing:

```crn
provider github {
  source = "file:///absolute/path/to/target/wasm32-wasip2/release/carina-provider-github.wasm"
}

github.actions.OidcSubject {
  repository = "carina-test/oidc-subject"
  claim_keys = [repo, context, job_workflow_ref]
}
```

Set `GITHUB_TOKEN` in your environment, then install and run with the
local component trusted as a GitHub provider:

```sh
carina init
carina plan --trust-local-provider /absolute/path/to/target/wasm32-wasip2/release/carina-provider-github.wasm=github
carina apply --trust-local-provider /absolute/path/to/target/wasm32-wasip2/release/carina-provider-github.wasm=github
```

`carina init` asks you to consent to the component's egress to
`api.github.com`. Without a terminal (in scripts or CI), pass the source
as `carina init` prints it: `carina init --accept-egress <source>`.

Run `carina init` again after each rebuild to refresh the installed copy;
each rebuild asks for consent again.
Native tests use a fake GitHub API and run with `cargo test`.

## License

MIT — see [LICENSE](LICENSE).
