# carina-provider-github

GitHub provider for [Carina](https://github.com/carina-rs/carina).

Manages a narrow set of GitHub resources via the GitHub REST API.

## Status

Pre-release. See [`docs/design.md`](docs/design.md) for the design and
[#4](https://github.com/carina-rs/carina-provider-github/issues/4) for the
first resource.

## Initial resource scope

| Carina resource              | Wraps                                                        |
| ---------------------------- | ------------------------------------------------------------ |
| `github.actions.OidcSubject` | `GET` / `PUT /repos/{owner}/{repo}/actions/oidc/customization/sub` |

Stay narrow on purpose. New resources are added when a real use case
appears, not preemptively.

## License

MIT — see [LICENSE](LICENSE).
