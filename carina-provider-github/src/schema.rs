use carina_plugin_sdk::core::schema::{
    AttributeSchema, AttributeType, ListElements, PatternMatch, PatternSpec, ResourceSchema,
    TypeIdentity, ValidatedResourceSchema,
};

pub(crate) const CLAIM_KEYS: &[&str] = &[
    "repo",
    "context",
    "actor",
    "actor_id",
    "base_ref",
    "check_run_id",
    "enterprise",
    "enterprise_id",
    "environment",
    "environment_node_id",
    "event_name",
    "head_ref",
    "issuer_scope",
    "job_workflow_ref",
    "job_workflow_sha",
    "ref",
    "ref_protected",
    "ref_type",
    "repository",
    "repository_id",
    "repository_owner",
    "repository_owner_id",
    "repository_visibility",
    "run_attempt",
    "run_id",
    "run_number",
    "runner_environment",
    "sha",
    "workflow",
    "workflow_ref",
    "workflow_sha",
];

pub(crate) fn oidc_subject() -> ValidatedResourceSchema {
    ResourceSchema::new("actions.OidcSubject")
        .with_description("A repository's GitHub Actions OIDC subject claim customization")
        .attribute(
            AttributeSchema::required(
                "repository",
                AttributeType::refined_string(
                    Some(TypeIdentity::new(
                        Some("github"),
                        ["actions", "OidcSubject"],
                        "Repository",
                    )),
                    Some(PatternSpec::new(
                        r"^[A-Za-z0-9-]+/(?:[A-Za-z0-9_-][A-Za-z0-9._-]*|\.[A-Za-z0-9_-][A-Za-z0-9._-]*|\.\.[A-Za-z0-9._-]+)$",
                        PatternMatch::Full,
                    )),
                    None,
                    None,
                ),
            )
            .create_only()
            .identity()
            .build(),
        )
        .attribute(
            AttributeSchema::optional(
                "claim_keys",
                AttributeType::refined_list(
                    AttributeType::enum_(
                        TypeIdentity::new(Some("github"), ["actions", "OidcSubject"], "ClaimKey"),
                        Some(CLAIM_KEYS.iter().map(|key| (*key).into()).collect()),
                        Vec::new(),
                        None,
                        None,
                    ),
                    true,
                    Some((Some(1), None)),
                    ListElements::Unique,
                ),
            )
            .with_description("Ordered subject claims; omit to use GitHub's default subject")
            .build(),
        )
        .attribute(
            AttributeSchema::provider_populated("use_default", AttributeType::bool())
                .with_description(
                    "Whether the repository uses GitHub's default subject; derived from whether claim_keys is set",
                )
                .build(),
        )
        .default_prevent_destroy()
        .validate()
        .expect("the OIDC subject schema is valid")
}
