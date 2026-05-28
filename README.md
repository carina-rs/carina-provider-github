# carina-provider-github

GitHub provider for [Carina](https://github.com/carina-rs/carina).

Manages a narrow set of GitHub resources via the GitHub REST API. The
current scope is whatever the registry dogfooding pipeline
(`carina-rs/registry`) needs in order to declare its own App credentials
and Org-level Actions configuration through Carina.

## Status

Bootstrapping. See [`docs/design.md`](docs/design.md) for the design and
[#1](https://github.com/carina-rs/carina-provider-github/issues) for
in-flight scope.

## Initial resource scope

| Carina resource              | Wraps                                                                       |
| ---------------------------- | --------------------------------------------------------------------------- |
| `github.actions.OrganizationSecret`   | `PUT /orgs/{org}/actions/secrets/{secret_name}` (libsodium sealed-box) |
| `github.actions.OrganizationVariable` | `PUT /orgs/{org}/actions/variables/{name}`                              |

Stay narrow on purpose. New resources are added when a real use case
appears, not preemptively.

## License

MIT — see [LICENSE](LICENSE).
