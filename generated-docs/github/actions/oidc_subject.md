---
title: "github.actions.OidcSubject"
---


A repository's GitHub Actions OIDC subject claim customization

Destruction protection is enabled by default. To opt out, set `directives { prevent_destroy = false }`.

## Example

```crn
github.actions.OidcSubject {
  repository = "carina-rs/carina"
  claim_keys = [repo, context, job_workflow_ref]
}
```

## Argument Reference

### `claim_keys`

- **Type:** List\<[github.actions.OidcSubject.ClaimKey](#githubactionsoidcsubjectclaimkey)\> (items: 1..unbounded, unique items, ordered)
- **Required:** No

Ordered subject claims; omit to use GitHub's default subject

### `repository`

- **Type:** `github.actions.OidcSubject.Repository (pattern: /^[A-Za-z0-9-]+/(?:[A-Za-z0-9_-][A-Za-z0-9._-]*|\.[A-Za-z0-9_-][A-Za-z0-9._-]*|\.\.[A-Za-z0-9._-]+)$/ (Full))`
- **Required:** Yes
- **Create-only:** Yes
- **Identity:** Required (omitting this value leaves identity incomplete)

## Attribute Reference

### `use_default`

- **Type:** `Bool`
- **Read-only**

Whether the repository uses GitHub's default subject; derived from whether claim_keys is set

## Enum Values

### `github.actions.OidcSubject.ClaimKey`

- **Base type:** `String`

| DSL Identifier | Value |
|----------------|-------|
| `github.actions.OidcSubject.ClaimKey.repo` | `repo` |
| `github.actions.OidcSubject.ClaimKey.context` | `context` |
| `github.actions.OidcSubject.ClaimKey.actor` | `actor` |
| `github.actions.OidcSubject.ClaimKey.actor_id` | `actor_id` |
| `github.actions.OidcSubject.ClaimKey.base_ref` | `base_ref` |
| `github.actions.OidcSubject.ClaimKey.check_run_id` | `check_run_id` |
| `github.actions.OidcSubject.ClaimKey.enterprise` | `enterprise` |
| `github.actions.OidcSubject.ClaimKey.enterprise_id` | `enterprise_id` |
| `github.actions.OidcSubject.ClaimKey.environment` | `environment` |
| `github.actions.OidcSubject.ClaimKey.environment_node_id` | `environment_node_id` |
| `github.actions.OidcSubject.ClaimKey.event_name` | `event_name` |
| `github.actions.OidcSubject.ClaimKey.head_ref` | `head_ref` |
| `github.actions.OidcSubject.ClaimKey.issuer_scope` | `issuer_scope` |
| `github.actions.OidcSubject.ClaimKey.job_workflow_ref` | `job_workflow_ref` |
| `github.actions.OidcSubject.ClaimKey.job_workflow_sha` | `job_workflow_sha` |
| `github.actions.OidcSubject.ClaimKey.ref` | `ref` |
| `github.actions.OidcSubject.ClaimKey.ref_protected` | `ref_protected` |
| `github.actions.OidcSubject.ClaimKey.ref_type` | `ref_type` |
| `github.actions.OidcSubject.ClaimKey.repository` | `repository` |
| `github.actions.OidcSubject.ClaimKey.repository_id` | `repository_id` |
| `github.actions.OidcSubject.ClaimKey.repository_owner` | `repository_owner` |
| `github.actions.OidcSubject.ClaimKey.repository_owner_id` | `repository_owner_id` |
| `github.actions.OidcSubject.ClaimKey.repository_visibility` | `repository_visibility` |
| `github.actions.OidcSubject.ClaimKey.run_attempt` | `run_attempt` |
| `github.actions.OidcSubject.ClaimKey.run_id` | `run_id` |
| `github.actions.OidcSubject.ClaimKey.run_number` | `run_number` |
| `github.actions.OidcSubject.ClaimKey.runner_environment` | `runner_environment` |
| `github.actions.OidcSubject.ClaimKey.sha` | `sha` |
| `github.actions.OidcSubject.ClaimKey.workflow` | `workflow` |
| `github.actions.OidcSubject.ClaimKey.workflow_ref` | `workflow_ref` |
| `github.actions.OidcSubject.ClaimKey.workflow_sha` | `workflow_sha` |

