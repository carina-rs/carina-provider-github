//! GitHub Actions OIDC subject customization for Carina.

#![allow(clippy::result_large_err)]

pub mod api;
mod schema;

use api::{GithubApi, GithubToken, HttpGithubApi, TransportError};
use carina_plugin_sdk::core::schema::{
    ReferenceForms, ValidatedConfigSchema, ValidatedResourceSchema,
};
use carina_plugin_sdk::types::*;
use carina_plugin_sdk::{
    CarinaProvider, CreateAttempt, CreateStep, CreateVerdict, DeleteOutcome, DesiredAttribute,
    DesiredResource, Never, NormalizedResource, Pending, PendingDelete, PlanOp, Poll, PollRequest,
    PollVerdict, Resume, ResumeStep, ResumeVerdict, Sent, UpdateOutcome,
};
use http::{Method, Request, Response};
use serde::Deserialize;
use serde_json::json;
use std::collections::BTreeMap;
use std::fmt;

#[derive(Default)]
pub struct GithubProvider<A = HttpGithubApi> {
    api: A,
    token: Option<GithubToken>,
}

impl<A> fmt::Debug for GithubProvider<A> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GithubProvider")
            .field("token", &self.token)
            .finish_non_exhaustive()
    }
}

fn error(kind: ProviderErrorKind, message: impl Into<String>) -> ProviderError {
    ProviderError {
        kind,
        message: message.into(),
        resource_id: None,
        cause: None,
        provider_name: Some("github".into()),
        operation: None,
        status: None,
        code: None,
        request_id: None,
    }
}

struct ApiFailure {
    error: ProviderError,
    uncertain: bool,
}

enum StatePresence {
    InState,
    NotInState,
}

impl From<ProviderError> for ApiFailure {
    fn from(error: ProviderError) -> Self {
        Self {
            error,
            uncertain: false,
        }
    }
}

#[derive(Deserialize)]
struct OidcSubject {
    use_default: bool,
    #[serde(default)]
    include_claim_keys: Vec<String>,
}

fn validate_repository(repository: &str) -> Result<(), ProviderError> {
    let valid = repository.split_once('/').is_some_and(|(owner, repo)| {
        !owner.is_empty()
            && !repo.is_empty()
            && repo != "."
            && repo != ".."
            && owner
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-')
            && repo
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
    });
    if valid {
        Ok(())
    } else {
        Err(error(
            ProviderErrorKind::InvalidInput,
            "repository must match owner/repo (letters, digits and GitHub name punctuation only)",
        ))
    }
}

fn claim_keys(value: &Value) -> Result<Vec<String>, ProviderError> {
    let invalid = || {
        error(
            ProviderErrorKind::InvalidInput,
            "claim_keys must be a non-empty list of github.actions.OidcSubject.ClaimKey values",
        )
    };
    let values: Vec<&str> = match value {
        Value::StringList(keys) => keys.iter().map(String::as_str).collect(),
        Value::List(keys) => keys
            .iter()
            .map(|key| match key {
                Value::String(key) => Ok(key.as_str()),
                _ => Err(invalid()),
            })
            .collect::<Result<_, _>>()?,
        _ => return Err(invalid()),
    };
    if values.is_empty() {
        return Err(invalid());
    }
    values
        .into_iter()
        .map(|key| {
            let key = key
                .strip_prefix("github.actions.OidcSubject.ClaimKey.")
                .unwrap_or(key);
            if !schema::CLAIM_KEYS.contains(&key) {
                return Err(invalid());
            }
            Ok(key.to_string())
        })
        .collect()
}

fn attributes(repository: &str, keys: Option<Vec<String>>) -> BTreeMap<String, Value> {
    let mut attributes = BTreeMap::from([
        ("repository".into(), Value::String(repository.into())),
        ("use_default".into(), Value::Bool(keys.is_none())),
    ]);
    if let Some(keys) = keys {
        attributes.insert(
            "claim_keys".into(),
            Value::List(keys.into_iter().map(Value::String).collect()),
        );
    }
    attributes
}

fn normalized_use_default(
    resource_type: &str,
    claim_keys: DesiredAttribute<&Value>,
    use_default: DesiredAttribute<&Value>,
) -> Option<Value> {
    if resource_type != "actions.OidcSubject" || !matches!(use_default, DesiredAttribute::Absent) {
        return None;
    }
    Some(Value::Bool(matches!(claim_keys, DesiredAttribute::Absent)))
}

impl<A: GithubApi> GithubProvider<A> {
    fn request(
        &self,
        method: Method,
        repository: &str,
        keys: Option<&[String]>,
        state_presence: StatePresence,
    ) -> Result<Response<Vec<u8>>, ApiFailure> {
        validate_repository(repository)?;
        let token = self.token.as_ref().ok_or_else(|| error(ProviderErrorKind::InvalidInput, "GitHub provider configuration: initialize with a non-empty GITHUB_TOKEN before making requests"))?;
        let operation = format!("actions.OidcSubject.{method}");
        let mut request = Request::builder()
            .method(method.clone())
            .uri(format!(
                "https://api.github.com/repos/{repository}/actions/oidc/customization/sub"
            ))
            .header("Authorization", token.authorization())
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header(
                "User-Agent",
                concat!("carina-provider-github/", env!("CARGO_PKG_VERSION")),
            );
        let body = if method == Method::PUT {
            request = request.header("Content-Type", "application/json");
            let body = match keys {
                Some(keys) => json!({"use_default": false, "include_claim_keys": keys}),
                None => json!({"use_default": true}),
            };
            serde_json::to_vec(&body).map_err(|_| {
                error(
                    ProviderErrorKind::Internal,
                    "failed to encode OIDC subject request",
                )
            })?
        } else {
            Vec::new()
        };
        let request = request.body(body).map_err(|_| {
            error(
                ProviderErrorKind::Internal,
                "failed to build GitHub HTTP request",
            )
        })?;
        let response = self.api.send(request).map_err(|failure| ApiFailure {
            uncertain: failure == TransportError::MaybeSent,
            error: ProviderError {
                operation: Some(operation.clone()),
                ..error(
                    ProviderErrorKind::ApiError,
                    match failure {
                        TransportError::NotSent => {
                            "GitHub HTTP transport failed before sending the request"
                        }
                        TransportError::MaybeSent => {
                            "GitHub HTTP transport failed; request outcome is unknown"
                        }
                    },
                )
            },
        })?;
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }

        let parsed: Option<serde_json::Value> = serde_json::from_slice(response.body()).ok();
        let mut message = match parsed
            .as_ref()
            .and_then(|body| body.get("message"))
            .and_then(serde_json::Value::as_str)
        {
            Some(message) => format!(
                "GitHub API returned HTTP {status}: {}",
                token.redact(message)
            ),
            None => format!(
                "GitHub API returned HTTP {status} with no error message in the body ({} bytes)",
                response.body().len()
            ),
        };
        let kind = match status.as_u16() {
            404 => {
                // The CLI converts NotFound errors to absent state, but GitHub's
                // 404 also covers missing repository access or request permissions.
                if !message.ends_with(['.', '!', '?']) {
                    message.push('.');
                }
                // ponytail: Non-repository endpoints need their own 404 wording.
                message.push_str(" The repository was not found, or the token cannot access it.");
                if matches!(state_presence, StatePresence::InState) {
                    message.push_str(
                        " If the repository really was deleted, remove this resource from state with a \
                         `removed { from = ... }` block.",
                    );
                }
                ProviderErrorKind::ApiError
            }
            400 | 422 => ProviderErrorKind::InvalidInput,
            408 | 504 => ProviderErrorKind::Timeout,
            _ => ProviderErrorKind::ApiError,
        };
        Err(ApiFailure {
            // A redirect is deliberately not followed and also is not proof
            // that the mutation was rejected. Only definite 4xx is untracked.
            uncertain: !status.is_client_error(),
            error: ProviderError {
                operation: Some(operation),
                status: Some(status.as_u16()),
                request_id: response
                    .headers()
                    .get("x-github-request-id")
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_owned),
                ..error(kind, message)
            },
        })
    }

    fn read_subject(
        &self,
        id: &ResourceId,
        identifier: &AppliedIdentifier,
    ) -> Result<ReadOutcome, ProviderError> {
        let response = self
            .request(
                Method::GET,
                identifier.as_str(),
                None,
                StatePresence::InState,
            )
            .map_err(|failure| failure.error)?;
        let subject: OidcSubject = serde_json::from_slice(response.body()).map_err(|_| {
            error(
                ProviderErrorKind::ApiError,
                "GitHub returned an invalid OIDC subject response",
            )
        })?;
        Ok(ReadOutcome::Found {
            state: AppliedState {
                id: id.clone(),
                identifier: identifier.clone(),
                attributes: attributes(
                    identifier.as_str(),
                    (!subject.use_default).then_some(subject.include_claim_keys),
                ),
            },
        })
    }
}

impl<A: GithubApi> CarinaProvider for GithubProvider<A> {
    type CreateToken = Never;
    type UpdateToken = Never;
    type DeleteToken = Never;

    fn info(&self) -> ProviderInfo {
        ProviderInfo {
            name: "github".into(),
            display_name: "GitHub".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            capabilities: self.capabilities(),
        }
    }

    fn capabilities(&self) -> Vec<String> {
        vec!["normalize_desired".into()]
    }

    fn schemas(&self) -> Vec<ValidatedResourceSchema> {
        vec![schema::oidc_subject()]
    }

    fn reference_forms(&self) -> ReferenceForms {
        ReferenceForms::new()
    }

    fn provider_config_attribute_types(&self) -> ValidatedConfigSchema {
        ValidatedConfigSchema::default()
    }

    fn validate_config(&self, _attrs: &BTreeMap<String, Value>) -> Result<(), String> {
        Ok(())
    }

    fn initialize(&mut self, _attrs: &BTreeMap<String, Value>) -> Result<ReferenceScope, String> {
        self.token = None;
        let token = std::env::var("GITHUB_TOKEN").map_err(|error| match error {
            std::env::VarError::NotPresent => {
                "GitHub provider configuration: GITHUB_TOKEN must be set and non-empty".to_string()
            }
            std::env::VarError::NotUnicode(_) => {
                "GitHub provider configuration: GITHUB_TOKEN is not valid Unicode".to_string()
            }
        })?;
        let token = GithubToken::new(token)?;
        self.token = Some(token);
        Ok(ReferenceScope::Unavailable)
    }

    fn normalize_desired(
        &self,
        resources: Vec<DesiredResource>,
    ) -> Result<Vec<NormalizedResource>, ProviderError> {
        resources
            .into_iter()
            .map(|mut resource| {
                if let Some(value) = normalized_use_default(
                    &resource.id().resource_type,
                    resource.attribute("claim_keys"),
                    resource.attribute("use_default"),
                ) {
                    resource
                        .insert_absent("use_default", value)
                        .map_err(|failure| {
                            error(
                                ProviderErrorKind::Internal,
                                format!("failed to normalize OIDC subject: {failure}"),
                            )
                        })?;
                }
                Ok(resource.finish())
            })
            .collect()
    }

    fn read(
        &self,
        id: &ResourceId,
        identifier: &AppliedIdentifier,
        _request: ReadRequest,
    ) -> Result<ReadOutcome, ProviderError> {
        self.read_subject(id, identifier)
    }

    fn read_data_source(&self, _resource: &Resource) -> Result<State, ProviderError> {
        Err(error(
            ProviderErrorKind::InvalidInput,
            "github has no data sources",
        ))
    }

    fn create(
        &self,
        id: &ResourceId,
        request: CreateRequest,
        attempt: CreateAttempt,
    ) -> Result<CreateStep<Never>, CreateVerdict<Never>> {
        let desired = || {
            let repository = match request.resource.attributes.get("repository") {
                Some(Value::String(repository)) => repository,
                _ => {
                    return Err(error(
                        ProviderErrorKind::InvalidInput,
                        "repository is required and must be an owner/repo string",
                    ));
                }
            };
            let identifier = AppliedIdentifier::new(repository.clone()).map_err(|_| {
                error(
                    ProviderErrorKind::InvalidInput,
                    "repository is required and must be an owner/repo string",
                )
            })?;
            let keys = request
                .resource
                .attributes
                .get("claim_keys")
                .map(claim_keys)
                .transpose()?;
            Ok((identifier, keys))
        };
        let (identifier, keys) = match desired() {
            Ok(desired) => desired,
            Err(error) => return Err(attempt.untracked(error)),
        };
        match self.request(
            Method::PUT,
            identifier.as_str(),
            keys.as_deref(),
            StatePresence::NotInState,
        ) {
            Ok(_) => {
                let attributes = attributes(identifier.as_str(), keys);
                Ok(attempt
                    .issued(identifier, Sent::AllRequestAttributes)
                    .succeeded(attributes))
            }
            Err(failure) if !failure.uncertain => Err(attempt.untracked(failure.error)),
            Err(failure) => {
                // PUT is idempotent and the setting's identifier is known
                // before transmission. Never cannot carry a continuation, so
                // retain that identifier as a tracked failure and let the next
                // plan refresh it. Do not claim unconfirmed claim_keys as sent
                // state; only the repository identity is certain.
                let issued = attempt.issued(identifier, Sent::only(["repository"]));
                let observation = match self.read_subject(id, issued.identifier()) {
                    Ok(outcome) => Observation::Read(outcome),
                    Err(error) => Observation::ReadFailed(error),
                };
                Err(issued.failed(observation, failure.error))
            }
        }
    }

    fn poll_create(
        &self,
        _id: &ResourceId,
        _request: CreateRequest,
        poll: Poll<Never>,
        _poll_request: PollRequest,
    ) -> Result<CreateStep<Never>, PollVerdict<Never>> {
        match poll {
            Poll::Unissued(_, token) | Poll::Issued(_, token) => match token {},
        }
    }

    fn resume_create(
        &self,
        _id: &ResourceId,
        _request: CreateRequest,
        resume: Resume<Never>,
        _poll_request: PollRequest,
    ) -> Result<ResumeStep<Never>, ResumeVerdict<Never>> {
        match resume {
            Resume::Unissued(_, token) | Resume::Issued(_, token) => match token {},
        }
    }

    fn update(
        &self,
        id: &ResourceId,
        identifier: &AppliedIdentifier,
        request: UpdateRequest,
    ) -> Result<UpdateOutcome<Never>, ProviderError> {
        // Validate the entire patch before transmitting any mutation.
        for op in &request.patch.ops {
            if !matches!(op.key(), "claim_keys" | "use_default") {
                return Err(error(
                    ProviderErrorKind::InvalidInput,
                    "only claim_keys and use_default can be updated; repository is create-only",
                ));
            }
        }
        let mut state = request.state_after_patch();
        if !request.patch.ops.is_empty() {
            // A default reset can leave unauthored claim_keys in the patched state.
            let use_default = match state.attributes.get("use_default") {
                Some(PersistedValue::Bool(use_default)) => *use_default,
                None => !state.attributes.contains_key("claim_keys"),
                _ => {
                    return Err(error(
                        ProviderErrorKind::InvalidInput,
                        "use_default must be a boolean",
                    ));
                }
            };
            let keys = if use_default {
                None
            } else {
                let value = state.attributes.get("claim_keys").ok_or_else(|| {
                    error(
                        ProviderErrorKind::InvalidInput,
                        "claim_keys is required when use_default is false",
                    )
                })?;
                let value = value.to_value().map_err(|failure| {
                    error(ProviderErrorKind::InvalidInput, failure.to_string())
                })?;
                Some(claim_keys(&value)?)
            };
            self.request(
                Method::PUT,
                identifier.as_str(),
                keys.as_deref(),
                StatePresence::InState,
            )
            .map_err(|failure| failure.error)?;
            state
                .attributes
                .insert("use_default".into(), Value::Bool(use_default).into());
            match keys {
                Some(keys) => {
                    state.attributes.insert(
                        "claim_keys".into(),
                        Value::List(keys.into_iter().map(Value::String).collect()).into(),
                    );
                }
                None => {
                    state.attributes.remove("claim_keys");
                }
            }
        }
        Ok(UpdateOutcome::Done(
            carina_plugin_sdk::protocol::UpdateOutcome::Success {
                state: AppliedPersistedState {
                    id: id.clone(),
                    identifier: identifier.clone(),
                    attributes: state.attributes,
                },
            },
        ))
    }

    fn poll_update(
        &self,
        _id: &ResourceId,
        _identifier: &AppliedIdentifier,
        _request: UpdateRequest,
        pending: Pending<Never>,
        _poll_request: PollRequest,
    ) -> Result<UpdateOutcome<Never>, ProviderError> {
        match pending.token {}
    }

    fn delete(
        &self,
        _id: &ResourceId,
        identifier: &AppliedIdentifier,
        _request: DeleteRequest,
    ) -> Result<DeleteOutcome<Never>, ProviderError> {
        self.request(
            Method::PUT,
            identifier.as_str(),
            None,
            StatePresence::InState,
        )
        .map_err(|failure| failure.error)?;
        Ok(DeleteOutcome::Deleted)
    }

    fn poll_delete(
        &self,
        _id: &ResourceId,
        _identifier: &AppliedIdentifier,
        _request: DeleteRequest,
        pending: PendingDelete<Never>,
    ) -> Result<DeleteOutcome<Never>, ProviderError> {
        match pending.token {}
    }

    fn required_permissions(
        &self,
        _id: &ResourceId,
        _op: PlanOp,
    ) -> Result<Vec<String>, ProviderError> {
        Ok(vec![])
    }
}

#[cfg(test)]
mod tests;
