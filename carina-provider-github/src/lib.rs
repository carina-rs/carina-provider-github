//! GitHub Actions OIDC subject customization for Carina.

#![allow(clippy::result_large_err)]

pub mod api;
mod schema;

use api::{GithubApi, HttpGithubApi, TransportError};
use carina_plugin_sdk::core::schema::{
    AssignabilityEdge, ReferenceForms, ValidatedConfigSchema, ValidatedResourceSchema,
};
use carina_plugin_sdk::protected_material::{GithubTokenMaterial, RegisteredGithubToken};
use carina_plugin_sdk::types::*;
use carina_plugin_sdk::{
    CarinaProvider, CreateAttempt, CreateStep, CreateVerdict, DeleteOutcome, DesiredResource,
    DesiredScalar, DesiredValue, DesiredView, Never, NormalizedResource, Pending, PendingDelete,
    PlanOp, Poll, PollRequest, PollVerdict, ProviderDiagnostics, Resume, ResumeStep, ResumeVerdict,
    Sent, UpdateOutcome,
};
use http::{Method, Request, Response};
use serde::Deserialize;
use serde_json::json;
use std::collections::BTreeMap;
use std::fmt;

/// Return the validated resource schemas exported by the GitHub provider.
pub fn resource_schemas() -> Vec<ValidatedResourceSchema> {
    vec![schema::oidc_subject()]
}

/// Return the provider-wide directional identity edges exported by the GitHub provider.
pub fn assignability_edges() -> Vec<AssignabilityEdge> {
    Vec::new()
}

#[derive(Default)]
pub struct GithubProvider<A = HttpGithubApi> {
    api: A,
    token: Option<RegisteredGithubToken>,
    #[cfg(not(target_arch = "wasm32"))]
    diagnostics: Option<ProviderDiagnostics>,
    // RegisteredRequest intentionally exposes no plaintext request. Observe only
    // the unsigned request in native tests; the fake still receives its capability.
    #[cfg(test)]
    unsigned_requests: std::rc::Rc<std::cell::RefCell<Vec<Request<Vec<u8>>>>>,
}

impl<A> fmt::Debug for GithubProvider<A> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GithubProvider")
            .field("token", &self.token)
            .finish_non_exhaustive()
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
        Err(ProviderError::invalid_input(
            "repository must match owner/repo (letters, digits and GitHub name punctuation only)",
        )
        .for_provider("github"))
    }
}

fn claim_keys(value: &Value) -> Result<Vec<String>, ProviderError> {
    let invalid = || {
        ProviderError::invalid_input(
            "claim_keys must be a non-empty list of github.actions.OidcSubject.ClaimKey values",
        )
        .for_provider("github")
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

fn normalized_use_default(view: &mut DesiredView<'_>) -> Result<(), ProviderError> {
    if view.id().resource_type != "actions.OidcSubject" || view.attribute("use_default").is_some() {
        return Ok(());
    }
    let value = DesiredValue::Known(DesiredScalar::Bool(view.attribute("claim_keys").is_none()));
    view.insert_absent("use_default", value).map_err(|failure| {
        ProviderError::internal_caused_by("failed to normalize OIDC subject", failure)
            .for_provider("github")
    })
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
        let token = self.token.as_ref().ok_or_else(|| ProviderError::invalid_input("GitHub provider configuration: initialize with a non-empty GITHUB_TOKEN before making requests").for_provider("github"))?;
        let operation = format!("actions.OidcSubject.{method}");
        let mut request = Request::builder()
            .method(method.clone())
            .uri(format!(
                "https://api.github.com/repos/{repository}/actions/oidc/customization/sub"
            ))
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
            serde_json::to_vec(&body).map_err(|failure| {
                ProviderError::internal_caused_by("failed to encode OIDC subject request", failure)
                    .for_provider("github")
            })?
        } else {
            Vec::new()
        };
        let request = request.body(body).map_err(|failure| {
            ProviderError::internal_caused_by("failed to build GitHub HTTP request", failure)
                .for_provider("github")
        })?;
        #[cfg(test)]
        self.unsigned_requests.borrow_mut().push(request.clone());
        let request = token.authorize(request).map_err(|failure| {
            ProviderError::internal_caused_by("failed to authorize GitHub HTTP request", failure)
                .for_provider("github")
                .with_operation(operation.clone())
        })?;
        let response = self.api.send(request).map_err(|failure| ApiFailure {
            uncertain: failure == TransportError::MaybeSent,
            error: ProviderError::api_error(match failure {
                TransportError::NotSent => {
                    "GitHub HTTP transport failed before sending the request"
                }
                TransportError::MaybeSent => {
                    "GitHub HTTP transport failed; request outcome is unknown"
                }
            })
            .for_provider("github")
            .with_operation(operation.clone()),
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
            // The host binds this opaque error using the token inventory populated
            // before authorization, including echoes in messages and request IDs.
            Some(message) => format!("GitHub API returned HTTP {status}: {message}"),
            None => format!(
                "GitHub API returned HTTP {status} with no error message in the body ({} bytes)",
                response.body().len()
            ),
        };
        let failure = match status.as_u16() {
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
                ProviderError::api_error(message)
            }
            400 | 422 => ProviderError::invalid_input(message),
            408 | 504 => ProviderError::timeout(message),
            _ => ProviderError::api_error(message),
        }
        .for_provider("github")
        .with_operation(operation)
        .with_status(status.as_u16());
        let failure = match response
            .headers()
            .get("x-github-request-id")
            .and_then(|value| value.to_str().ok())
        {
            Some(request_id) => failure.with_request_id(request_id),
            None => failure,
        };
        Err(ApiFailure {
            // A redirect is deliberately not followed and also is not proof
            // that the mutation was rejected. Only definite 4xx yields NotCreated.
            uncertain: !status.is_client_error(),
            error: failure,
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
        let subject: OidcSubject = serde_json::from_slice(response.body()).map_err(|failure| {
            ProviderError::api_error_caused_by(
                "GitHub returned an invalid OIDC subject response",
                failure,
            )
            .for_provider("github")
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
        resource_schemas()
    }

    fn reference_forms(&self) -> ReferenceForms {
        ReferenceForms::new()
    }

    fn assignability_edges(&self) -> Vec<AssignabilityEdge> {
        assignability_edges()
    }

    fn provider_config_attribute_types(&self) -> ValidatedConfigSchema {
        ValidatedConfigSchema::default()
    }

    fn validate_config(&self, _attrs: &BTreeMap<String, Value>) -> Result<(), ProviderError> {
        Ok(())
    }

    fn initialize(
        &mut self,
        _attrs: &BTreeMap<String, Value>,
    ) -> Result<ReferenceScope, ProviderError> {
        self.token = None;
        let token = std::env::var("GITHUB_TOKEN").map_err(|failure| {
            ProviderError::invalid_input(match failure {
                std::env::VarError::NotPresent => {
                    "GitHub provider configuration: GITHUB_TOKEN must be set and non-empty"
                }
                std::env::VarError::NotUnicode(_) => {
                    "GitHub provider configuration: GITHUB_TOKEN is not valid Unicode"
                }
            })
            .for_provider("github")
        })?;
        if token.is_empty() {
            return Err(ProviderError::invalid_input(
                "GitHub provider configuration: GITHUB_TOKEN must be set and non-empty",
            )
            .for_provider("github"));
        }
        http::HeaderValue::from_str(&format!("Bearer {token}")).map_err(|failure| {
            ProviderError::invalid_input_caused_by(
                "GitHub provider configuration: GITHUB_TOKEN is not a valid authorization header",
                failure,
            )
            .for_provider("github")
        })?;
        #[cfg(target_arch = "wasm32")]
        let diagnostics = ProviderDiagnostics::guest();
        #[cfg(not(target_arch = "wasm32"))]
        let diagnostics = self.diagnostics.as_ref().ok_or_else(|| {
            ProviderError::internal("GitHub provider initialization requires session diagnostics")
                .for_provider("github")
        })?;
        self.token = Some(
            diagnostics
                .register_github_token(GithubTokenMaterial::new(token))
                .map_err(|failure| {
                    ProviderError::internal_caused_by(
                        "GitHub provider configuration: failed to register GITHUB_TOKEN",
                        failure,
                    )
                    .for_provider("github")
                })?,
        );
        Ok(ReferenceScope::Unavailable)
    }

    fn normalize_desired(
        &self,
        resources: Vec<DesiredResource>,
    ) -> Result<Vec<NormalizedResource>, ProviderError> {
        resources
            .into_iter()
            .map(|resource| {
                let mut desired = resource.into_normalization()?;
                normalized_use_default(&mut desired.view())?;
                Ok(desired.finish()?)
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
        Err(ProviderError::invalid_input("github has no data sources").for_provider("github"))
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
                    return Err(ProviderError::invalid_input(
                        "repository is required and must be an owner/repo string",
                    )
                    .for_provider("github"));
                }
            };
            let identifier = AppliedIdentifier::new(repository.clone()).map_err(|_| {
                ProviderError::invalid_input(
                    "repository is required and must be an owner/repo string",
                )
                .for_provider("github")
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
            Err(error) => return Err(attempt.not_created(error)),
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
            Err(failure) if !failure.uncertain => Err(attempt.not_created(failure.error)),
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
                Err(issued.identifier_reported(observation, failure.error))
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
                return Err(ProviderError::invalid_input(
                    "only claim_keys and use_default can be updated; repository is create-only",
                )
                .for_provider("github"));
            }
        }
        let mut state = request.state_after_patch();
        if !request.patch.ops.is_empty() {
            // A default reset can leave unauthored claim_keys in the patched state.
            let use_default = match state.attributes.get("use_default") {
                Some(PersistedValue::Bool(use_default)) => *use_default,
                None => !state.attributes.contains_key("claim_keys"),
                _ => {
                    return Err(
                        ProviderError::invalid_input("use_default must be a boolean")
                            .for_provider("github"),
                    );
                }
            };
            let keys = if use_default {
                None
            } else {
                let value = state.attributes.get("claim_keys").ok_or_else(|| {
                    ProviderError::invalid_input("claim_keys is required when use_default is false")
                        .for_provider("github")
                })?;
                let value = value.to_value().map_err(|failure| {
                    ProviderError::from_diagnostic(ProviderErrorKind::InvalidInput, failure)
                        .for_provider("github")
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
            carina_plugin_sdk::types::UpdateOutcome::Success {
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
