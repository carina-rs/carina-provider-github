# GitHub Provider

The GitHub provider manages a narrow set of GitHub settings through the GitHub REST API. Declare resources with names such as `github.actions.OidcSubject`; the `provider github` block has no configuration attributes.

```crn
provider github {
}

github.actions.OidcSubject {
  repository = "carina-rs/carina"
  claim_keys = [repo, context, job_workflow_ref]
}
```

## Authentication

The provider reads a token from the `GITHUB_TOKEN` environment variable. A fine-grained personal access token needs Actions write permission on each managed repository; a classic token needs the `repo` scope.

```console
GITHUB_TOKEN=<token> carina plan .
```

## Scope

The provider stays narrow on purpose: new resources are added when a real use case appears. Each resource page lists its arguments and the values the provider reads back.
