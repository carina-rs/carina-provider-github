use super::*;
use crate::api::{GithubApi, GithubToken, TransportError};
use carina_plugin_sdk::core::parser::ProviderCustomTypes;
use carina_plugin_sdk::core::resource::{
    ConcreteValue, DeferredValue, Resource as CoreResource, Value as CoreValue,
};
use carina_plugin_sdk::core::schema::{
    CompiledAttributeType, IdentityMode, SchemaKind, SchemaRegistry, TypeError, TypeInSchema,
};
use carina_plugin_sdk::protocol::types::{
    CreateStep as WireCreateStep, CreateVerdict as WireCreateVerdict,
};
use carina_plugin_sdk::{NativeDesiredNormalization, ProviderSchemaCache, dispatch_create};
use http::{Request, Response};
use serde_json::json;
use std::cell::RefCell;
use std::collections::VecDeque;

const TEST_TOKEN: &str = "github_test_secret_never_print";
const REPOSITORY: &str = "carina-test/oidc-subject";

#[derive(Default)]
struct FakeApi {
    requests: RefCell<Vec<Request<Vec<u8>>>>,
    responses: RefCell<VecDeque<Result<Response<Vec<u8>>, TransportError>>>,
}

impl GithubApi for FakeApi {
    fn send(&self, request: Request<Vec<u8>>) -> Result<Response<Vec<u8>>, TransportError> {
        assert_eq!(request.uri().scheme_str(), Some("https"));
        assert_eq!(request.uri().host(), Some("api.github.com"));
        assert_eq!(
            request.uri().path(),
            format!("/repos/{REPOSITORY}/actions/oidc/customization/sub")
        );
        assert_eq!(
            request.headers()["authorization"],
            format!("Bearer {TEST_TOKEN}")
        );
        assert!(request.headers()["authorization"].is_sensitive());
        assert_eq!(request.headers()["accept"], "application/vnd.github+json");
        assert_eq!(request.headers()["x-github-api-version"], "2022-11-28");
        assert!(!request.headers()["user-agent"].is_empty());
        assert!(!format!("{request:?}").contains(TEST_TOKEN));
        if !request.body().is_empty() {
            let body: serde_json::Value = serde_json::from_slice(request.body()).unwrap();
            assert!(body.get("use_immutable_subject").is_none());
            assert!(body.get("sub_claim_prefix").is_none());
            assert_eq!(request.headers()["content-type"], "application/json");
        }
        self.requests.borrow_mut().push(request);
        self.responses
            .borrow_mut()
            .pop_front()
            .expect("unexpected HTTP request")
    }
}

fn response(status: u16, body: serde_json::Value) -> Result<Response<Vec<u8>>, TransportError> {
    Ok(Response::builder()
        .status(status)
        .body(serde_json::to_vec(&body).unwrap())
        .unwrap())
}

fn provider(
    responses: impl IntoIterator<Item = Result<Response<Vec<u8>>, TransportError>>,
) -> GithubProvider<FakeApi> {
    GithubProvider {
        api: FakeApi {
            responses: RefCell::new(responses.into_iter().collect()),
            ..Default::default()
        },
        token: Some(GithubToken::new(TEST_TOKEN.into()).unwrap()),
    }
}

fn id() -> ResourceId {
    ResourceId {
        provider: "github".into(),
        resource_type: "actions.OidcSubject".into(),
        identity: "subject".into(),
    }
}

fn identifier() -> AppliedIdentifier {
    AppliedIdentifier::new(REPOSITORY).unwrap()
}

fn keys(values: &[&str]) -> Value {
    Value::List(
        values
            .iter()
            .map(|key| Value::String((*key).into()))
            .collect(),
    )
}

fn attributes(claim_keys: Option<&[&str]>) -> BTreeMap<String, Value> {
    let mut attrs = BTreeMap::from([("repository".into(), Value::String(REPOSITORY.into()))]);
    if let Some(claim_keys) = claim_keys {
        attrs.insert("claim_keys".into(), keys(claim_keys));
    }
    attrs
}

fn state_attributes(claim_keys: Option<&[&str]>) -> BTreeMap<String, Value> {
    let mut attrs = attributes(claim_keys);
    attrs.insert("use_default".into(), Value::Bool(claim_keys.is_none()));
    attrs
}

fn create_request(claim_keys: Option<&[&str]>) -> CreateRequest {
    CreateRequest {
        resource: Resource {
            id: id(),
            attributes: attributes(claim_keys),
            directives: Default::default(),
        },
    }
}

fn assert_body(provider: &GithubProvider<FakeApi>, index: usize, expected: serde_json::Value) {
    let requests = provider.api.requests.borrow();
    assert_eq!(requests[index].method(), "PUT");
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(requests[index].body()).unwrap(),
        expected
    );
}

// Exercise the shared DesiredView normalizer with real opaque deferred leaves
// through the native adapter; DesiredResource is constructed at the guest boundary.
fn normalize_desired_resource(
    resource_type: &str,
    claim_keys: Option<CoreValue>,
    use_default: Option<CoreValue>,
) -> CoreResource {
    let mut resource = CoreResource::with_provider("github", resource_type, "subject", None);
    if let Some(claim_keys) = claim_keys {
        resource = resource.with_attribute("claim_keys", claim_keys);
    }
    if let Some(use_default) = use_default {
        resource = resource.with_attribute("use_default", use_default);
    }
    let mut desired = NativeDesiredNormalization::from_core(&mut resource).unwrap();
    normalized_use_default(&mut desired.view()).unwrap();
    desired.finish().unwrap();
    resource
}

#[test]
fn normalize_desired_inserts_use_default_for_absent_known_and_deferred_keys() {
    let known = CoreValue::Concrete(ConcreteValue::List(vec![CoreValue::Concrete(
        ConcreteValue::String("repo".into()),
    )]));
    for (claim_keys, expected) in [
        (None, true),
        (Some(known.clone()), false),
        (
            Some(CoreValue::Deferred(DeferredValue::Secret(Box::new(known)))),
            false,
        ),
    ] {
        let expected_keys = claim_keys.clone();
        let resource = normalize_desired_resource("actions.OidcSubject", claim_keys, None);
        assert_eq!(
            resource.attributes["use_default"],
            CoreValue::Concrete(ConcreteValue::Bool(expected)),
        );
        assert_eq!(
            resource.attributes.get("claim_keys"),
            expected_keys.as_ref()
        );
    }
}

#[test]
fn normalize_desired_keeps_existing_use_default() {
    let known = CoreValue::Concrete(ConcreteValue::List(vec![CoreValue::Concrete(
        ConcreteValue::String("repo".into()),
    )]));
    for existing in [true, false] {
        for claim_keys in [
            None,
            Some(known.clone()),
            Some(CoreValue::Deferred(DeferredValue::Secret(Box::new(
                known.clone(),
            )))),
        ] {
            let existing = CoreValue::Concrete(ConcreteValue::Bool(existing));
            let resource = normalize_desired_resource(
                "actions.OidcSubject",
                claim_keys,
                Some(existing.clone()),
            );
            assert_eq!(resource.attributes["use_default"], existing);
        }
    }
}

#[test]
fn normalize_desired_preserves_deferred_use_default() {
    let existing = CoreValue::Deferred(DeferredValue::Secret(Box::new(CoreValue::Concrete(
        ConcreteValue::Bool(true),
    ))));
    let resource = normalize_desired_resource("actions.OidcSubject", None, Some(existing.clone()));
    assert_eq!(resource.attributes["use_default"], existing);
}

#[test]
fn normalize_desired_leaves_other_resource_types_untouched() {
    let resource = normalize_desired_resource("actions.Other", None, None);
    assert!(resource.attributes.is_empty());
}

#[test]
fn create_default() {
    let provider = provider([response(201, json!({}))]);
    let step = dispatch_create(&provider, &id(), create_request(None)).unwrap();
    let WireCreateStep::Success {
        identifier,
        attributes: actual,
    } = step
    else {
        panic!("expected successful create");
    };
    assert_eq!(identifier.as_str(), REPOSITORY);
    assert_eq!(actual, state_attributes(None));
    assert_body(&provider, 0, json!({"use_default": true}));
}

#[test]
fn create_custom_preserves_key_order() {
    let provider = provider([response(201, json!({}))]);
    let desired = ["job_workflow_ref", "repo", "context"];
    let step = dispatch_create(&provider, &id(), create_request(Some(&desired))).unwrap();
    let WireCreateStep::Success {
        identifier,
        attributes: actual,
    } = step
    else {
        panic!("expected successful create");
    };
    assert_eq!(identifier.as_str(), REPOSITORY);
    assert_eq!(actual, state_attributes(Some(&desired)));
    assert_body(
        &provider,
        0,
        json!({"use_default": false, "include_claim_keys": desired}),
    );
}

#[test]
fn create_accepts_string_list_wire_values() {
    let provider = provider([response(201, json!({}))]);
    let mut request = create_request(None);
    request
        .resource
        .attributes
        .insert("claim_keys".into(), Value::StringList(vec!["repo".into()]));
    dispatch_create(&provider, &id(), request).unwrap();
    assert_body(
        &provider,
        0,
        json!({"use_default": false, "include_claim_keys": ["repo"]}),
    );
}

#[test]
fn create_rejects_empty_claim_keys_before_any_request() {
    for value in [keys(&[]), Value::StringList(vec![])] {
        let provider = provider([response(201, json!({}))]);
        let mut request = create_request(None);
        request
            .resource
            .attributes
            .insert("claim_keys".into(), value);
        let verdict = dispatch_create(&provider, &id(), request).unwrap_err();
        let WireCreateVerdict::NotCreated(error) = *verdict else {
            panic!("expected NotCreated");
        };
        assert_eq!(error.kind, ProviderErrorKind::InvalidInput);
        assert!(provider.api.requests.borrow().is_empty());
    }
}

#[test]
fn create_derives_use_default_only_from_claim_keys() {
    for claim_keys in [None, Some(["repo"].as_slice())] {
        let provider = provider([response(201, json!({}))]);
        let mut request = create_request(claim_keys);
        request
            .resource
            .attributes
            .insert("use_default".into(), Value::Bool(claim_keys.is_some()));
        let WireCreateStep::Success { attributes, .. } =
            dispatch_create(&provider, &id(), request).unwrap()
        else {
            panic!("expected successful create");
        };
        assert_eq!(attributes, state_attributes(claim_keys));
        let body = match claim_keys {
            Some(keys) => json!({"use_default": false, "include_claim_keys": keys}),
            None => json!({"use_default": true}),
        };
        assert_body(&provider, 0, body);
    }
}

#[test]
fn read_default_omits_keys_even_if_github_returns_them() {
    let provider = provider([response(
        200,
        json!({"use_default": true, "include_claim_keys": ["repo"]}),
    )]);
    let ReadOutcome::Found { state } = provider.read(&id(), &identifier(), ReadRequest).unwrap()
    else {
        panic!("expected found");
    };
    assert_eq!(state.id, id());
    assert_eq!(state.identifier.as_str(), REPOSITORY);
    assert_eq!(state.attributes, state_attributes(None));
    assert_eq!(provider.api.requests.borrow()[0].method(), "GET");
    assert!(provider.api.requests.borrow()[0].body().is_empty());
}

#[test]
fn read_custom_preserves_key_order() {
    let desired = ["job_workflow_ref", "repo", "context"];
    let provider = provider([response(
        200,
        json!({"use_default": false, "include_claim_keys": desired, "use_immutable_subject": true}),
    )]);
    let ReadOutcome::Found { state } = provider.read(&id(), &identifier(), ReadRequest).unwrap()
    else {
        panic!("expected found");
    };
    assert_eq!(state.attributes, state_attributes(Some(&desired)));
}

#[test]
fn read_org_template_returns_empty_keys() {
    for body in [
        json!({"use_default": false}),
        json!({"use_default": false, "include_claim_keys": []}),
    ] {
        let provider = provider([response(200, body)]);
        let ReadOutcome::Found { state } =
            provider.read(&id(), &identifier(), ReadRequest).unwrap()
        else {
            panic!("expected found");
        };
        assert_eq!(state.attributes, state_attributes(Some(&[])));
    }
}

#[test]
fn read_404_is_an_error() {
    for (body, expected_message) in [
        (
            json!({"message": "Not Found"}),
            "GitHub API returned HTTP 404 Not Found: Not Found. \
             The repository was not found, or the token cannot access it. \
             If the repository really was deleted, remove this resource from state with a \
             `removed { from = ... }` block.",
        ),
        (
            json!({}),
            "GitHub API returned HTTP 404 Not Found with no error message in the body (2 bytes). \
             The repository was not found, or the token cannot access it. \
             If the repository really was deleted, remove this resource from state with a \
             `removed { from = ... }` block.",
        ),
        (
            json!({"message": "Not Found."}),
            "GitHub API returned HTTP 404 Not Found: Not Found. \
             The repository was not found, or the token cannot access it. \
             If the repository really was deleted, remove this resource from state with a \
             `removed { from = ... }` block.",
        ),
        (
            json!({"message": "Not Found!"}),
            "GitHub API returned HTTP 404 Not Found: Not Found! \
             The repository was not found, or the token cannot access it. \
             If the repository really was deleted, remove this resource from state with a \
             `removed { from = ... }` block.",
        ),
        (
            json!({"message": "Not Found?"}),
            "GitHub API returned HTTP 404 Not Found: Not Found? \
             The repository was not found, or the token cannot access it. \
             If the repository really was deleted, remove this resource from state with a \
             `removed { from = ... }` block.",
        ),
    ] {
        let provider = provider([response(404, body)]);
        let error = provider
            .read(&id(), &identifier(), ReadRequest)
            .unwrap_err();
        assert_eq!(error.kind, ProviderErrorKind::ApiError);
        assert_eq!(error.status, Some(404));
        assert_eq!(error.operation.as_deref(), Some("actions.OidcSubject.GET"));
        assert_eq!(error.message, expected_message);
    }
}

#[test]
fn invalid_read_response_is_an_error() {
    for body in [
        json!({}),
        json!({"use_default": "false"}),
        json!({"use_default": false, "include_claim_keys": 42}),
    ] {
        let provider = provider([response(200, body)]);
        assert!(provider.read(&id(), &identifier(), ReadRequest).is_err());
    }
}

fn update_request(ops: Vec<PatchOp>) -> UpdateRequest {
    UpdateRequest {
        from: PersistedState {
            id: id(),
            identifier: Some(REPOSITORY.into()),
            attributes: state_attributes(Some(&["repo", "context"]))
                .into_iter()
                .map(|(key, value)| (key, value.into()))
                .collect(),
            exists: true,
        },
        patch: UpdatePatch { ops },
    }
}

#[test]
fn update_custom_to_custom_uses_patch() {
    let provider = provider([response(204, json!(null))]);
    let desired = ["repo", "job_workflow_ref"];
    let outcome = provider
        .update(
            &id(),
            &identifier(),
            update_request(vec![PatchOp::Replace {
                key: "claim_keys".into(),
                value: keys(&desired),
            }]),
        )
        .unwrap();
    let UpdateOutcome::Done(carina_plugin_sdk::protocol::UpdateOutcome::Success { state }) =
        outcome
    else {
        panic!("expected successful update");
    };
    assert_eq!(state.identifier.as_str(), REPOSITORY);
    assert_eq!(state.attributes["claim_keys"], keys(&desired).into());
    assert_eq!(state.attributes["use_default"], PersistedValue::Bool(false));
    assert_body(
        &provider,
        0,
        json!({"use_default": false, "include_claim_keys": desired}),
    );
    assert_eq!(provider.api.requests.borrow().len(), 1);
}

#[test]
fn update_removing_claim_keys_without_use_default_resets_to_default() {
    let provider = provider([response(204, json!(null))]);
    let mut request = update_request(vec![PatchOp::Remove {
        key: "claim_keys".into(),
    }]);
    request.from.attributes.remove("use_default");
    let outcome = provider.update(&id(), &identifier(), request).unwrap();
    let UpdateOutcome::Done(carina_plugin_sdk::protocol::UpdateOutcome::Success { state }) =
        outcome
    else {
        panic!("expected successful update");
    };
    assert!(!state.attributes.contains_key("claim_keys"));
    assert_eq!(state.attributes["use_default"], PersistedValue::Bool(true));
    assert_body(&provider, 0, json!({"use_default": true}));
}

#[test]
fn update_custom_to_default_accepts_use_default_patch_in_either_order() {
    let ops = vec![
        PatchOp::Remove {
            key: "claim_keys".into(),
        },
        PatchOp::Replace {
            key: "use_default".into(),
            value: Value::Bool(true),
        },
    ];
    for ops in [ops.clone(), ops.into_iter().rev().collect()] {
        let provider = provider([response(204, json!(null))]);
        let outcome = provider
            .update(&id(), &identifier(), update_request(ops))
            .unwrap();
        let UpdateOutcome::Done(carina_plugin_sdk::protocol::UpdateOutcome::Success { state }) =
            outcome
        else {
            panic!("expected successful update");
        };
        assert!(!state.attributes.contains_key("claim_keys"));
        assert_eq!(state.attributes["use_default"], PersistedValue::Bool(true));
        assert_body(&provider, 0, json!({"use_default": true}));
        assert_eq!(provider.api.requests.borrow().len(), 1);
    }
}

#[test]
fn update_use_default_only_uses_patched_state() {
    for claim_keys in [None, Some(["repo"].as_slice()), Some([].as_slice())] {
        for op in [
            PatchOp::Add {
                key: "use_default".into(),
                value: Value::Bool(claim_keys.is_none()),
            },
            PatchOp::Replace {
                key: "use_default".into(),
                value: Value::Bool(claim_keys.is_none()),
            },
            PatchOp::Reassert {
                key: "use_default".into(),
                value: Value::Bool(claim_keys.is_none()),
            },
            PatchOp::Remove {
                key: "use_default".into(),
            },
        ] {
            let provider = provider([response(204, json!(null))]);
            let mut request = update_request(vec![op]);
            request.from.attributes = attributes(claim_keys)
                .into_iter()
                .map(|(key, value)| (key, value.into()))
                .collect();
            let outcome = provider.update(&id(), &identifier(), request);
            if claim_keys.is_some_and(|keys| keys.is_empty()) {
                assert_eq!(outcome.unwrap_err().kind, ProviderErrorKind::InvalidInput);
                assert!(provider.api.requests.borrow().is_empty());
                continue;
            }
            let UpdateOutcome::Done(carina_plugin_sdk::protocol::UpdateOutcome::Success { state }) =
                outcome.unwrap()
            else {
                panic!("expected successful update");
            };
            let expected = state_attributes(claim_keys)
                .into_iter()
                .map(|(key, value)| (key, value.into()))
                .collect();
            assert_eq!(state.attributes, expected);
            let body = match claim_keys {
                Some(keys) => json!({"use_default": false, "include_claim_keys": keys}),
                None => json!({"use_default": true}),
            };
            assert_body(&provider, 0, body);
            assert_eq!(provider.api.requests.borrow().len(), 1);
        }
    }
}

#[test]
fn update_use_default_only_repairs_default_subject_drift() {
    let provider = provider([response(204, json!(null))]);
    let mut request = update_request(vec![PatchOp::Replace {
        key: "use_default".into(),
        value: Value::Bool(true),
    }]);
    request
        .from
        .attributes
        .insert("claim_keys".into(), keys(&["repo"]).into());
    let outcome = provider.update(&id(), &identifier(), request).unwrap();
    let UpdateOutcome::Done(carina_plugin_sdk::protocol::UpdateOutcome::Success { state }) =
        outcome
    else {
        panic!("expected successful update");
    };
    assert_body(&provider, 0, json!({"use_default": true}));
    assert_eq!(state.attributes["use_default"], PersistedValue::Bool(true));
    assert!(!state.attributes.contains_key("claim_keys"));
    assert_eq!(provider.api.requests.borrow().len(), 1);
}

#[test]
fn update_use_default_false_without_claim_keys_errors_before_sending() {
    let mut missing_keys = update_request(vec![PatchOp::Replace {
        key: "use_default".into(),
        value: Value::Bool(false),
    }]);
    missing_keys.from.attributes.remove("claim_keys");
    for request in [
        missing_keys,
        update_request(vec![PatchOp::Remove {
            key: "claim_keys".into(),
        }]),
    ] {
        let provider = provider([response(204, json!(null))]);
        let error = provider.update(&id(), &identifier(), request).unwrap_err();
        assert_eq!(error.kind, ProviderErrorKind::InvalidInput);
        assert!(error.message.contains("claim_keys"));
        assert!(provider.api.requests.borrow().is_empty());
    }
}

#[test]
fn update_add_and_reassert_keys() {
    for op in [
        PatchOp::Add {
            key: "claim_keys".into(),
            value: keys(&["repo"]),
        },
        PatchOp::Reassert {
            key: "claim_keys".into(),
            value: keys(&["repo"]),
        },
    ] {
        let provider = provider([response(204, json!(null))]);
        provider
            .update(&id(), &identifier(), update_request(vec![op]))
            .unwrap();
        assert_body(
            &provider,
            0,
            json!({"use_default": false, "include_claim_keys": ["repo"]}),
        );
    }
}

#[test]
fn update_validates_entire_patch_before_transmitting() {
    let provider = provider([]);
    let request = update_request(vec![
        PatchOp::Replace {
            key: "claim_keys".into(),
            value: keys(&["repo"]),
        },
        PatchOp::Replace {
            key: "repository".into(),
            value: Value::String("another/repo".into()),
        },
    ]);
    assert!(provider.update(&id(), &identifier(), request).is_err());
    assert!(provider.api.requests.borrow().is_empty());
}

#[test]
fn update_empty_patch_does_not_send_request() {
    let provider = provider([]);
    provider
        .update(&id(), &identifier(), update_request(vec![]))
        .unwrap();
    assert!(provider.api.requests.borrow().is_empty());
}

#[test]
fn update_failures_return_error_without_retry_or_read_back() {
    for failure in [
        response(500, json!({"message": "Internal Server Error"})),
        response(404, json!({"message": "Not Found"})),
        Err(TransportError::MaybeSent),
    ] {
        let expected_status = failure
            .as_ref()
            .ok()
            .map(|response| response.status().as_u16());
        let provider = provider([failure]);
        let error = provider
            .update(
                &id(),
                &identifier(),
                update_request(vec![PatchOp::Replace {
                    key: "claim_keys".into(),
                    value: keys(&["repo"]),
                }]),
            )
            .unwrap_err();
        assert_eq!(error.kind, ProviderErrorKind::ApiError);
        assert_eq!(error.status, expected_status);
        if expected_status == Some(404) {
            assert_eq!(
                error.message,
                "GitHub API returned HTTP 404 Not Found: Not Found. \
                 The repository was not found, or the token cannot access it. \
                 If the repository really was deleted, remove this resource from state with a \
                 `removed { from = ... }` block."
            );
        }
        assert_eq!(provider.api.requests.borrow().len(), 1);
        assert_body(
            &provider,
            0,
            json!({"use_default": false, "include_claim_keys": ["repo"]}),
        );
    }
}

#[test]
fn delete_resets_to_default() {
    let provider = provider([response(204, json!(null))]);
    assert!(matches!(
        provider.delete(&id(), &identifier(), DeleteRequest::default()),
        Ok(DeleteOutcome::Deleted)
    ));
    assert_body(&provider, 0, json!({"use_default": true}));
}

#[test]
fn delete_404_is_an_error() {
    for (body, expected_message) in [
        (
            json!({"message": "Not Found"}),
            "GitHub API returned HTTP 404 Not Found: Not Found. \
             The repository was not found, or the token cannot access it. \
             If the repository really was deleted, remove this resource from state with a \
             `removed { from = ... }` block.",
        ),
        (
            json!({}),
            "GitHub API returned HTTP 404 Not Found with no error message in the body (2 bytes). \
             The repository was not found, or the token cannot access it. \
             If the repository really was deleted, remove this resource from state with a \
             `removed { from = ... }` block.",
        ),
    ] {
        let provider = provider([response(404, body)]);
        let error = provider
            .delete(&id(), &identifier(), DeleteRequest::default())
            .unwrap_err();
        assert_eq!(error.kind, ProviderErrorKind::ApiError);
        assert_eq!(error.status, Some(404));
        assert_eq!(error.operation.as_deref(), Some("actions.OidcSubject.PUT"));
        assert_eq!(error.message, expected_message);
        assert_body(&provider, 0, json!({"use_default": true}));
    }
}

#[test]
fn delete_failures_return_error() {
    for failure in [
        response(500, json!({"message": "Internal Server Error"})),
        response(403, json!({"message": "Forbidden"})),
        Err(TransportError::MaybeSent),
    ] {
        let provider = provider([failure]);
        let error = provider
            .delete(&id(), &identifier(), DeleteRequest::default())
            .unwrap_err();
        assert_eq!(error.kind, ProviderErrorKind::ApiError);
        assert_eq!(provider.api.requests.borrow().len(), 1);
        assert_body(&provider, 0, json!({"use_default": true}));
    }
}

#[test]
fn definite_4xx_create_failures_are_not_created_with_github_message() {
    for status in [400, 401, 403, 404, 409, 422, 429] {
        let provider = provider([response(
            status,
            json!({"message": "GitHub rejected this request"}),
        )]);
        let verdict = dispatch_create(&provider, &id(), create_request(None)).unwrap_err();
        let WireCreateVerdict::NotCreated(error) = *verdict else {
            panic!("expected NotCreated");
        };
        assert_eq!(error.status, Some(status));
        assert!(error.message.contains("GitHub rejected this request"));
        assert_eq!(
            error.kind,
            if status == 400 || status == 422 {
                ProviderErrorKind::InvalidInput
            } else {
                ProviderErrorKind::ApiError
            }
        );
        assert_eq!(provider.api.requests.borrow().len(), 1);
    }
}

#[test]
fn not_created_create_404_message_is_exact() {
    for (body, expected_message) in [
        (
            json!({"message": "Not Found"}),
            "GitHub API returned HTTP 404 Not Found: Not Found. \
             The repository was not found, or the token cannot access it.",
        ),
        (
            json!({}),
            "GitHub API returned HTTP 404 Not Found with no error message in the body (2 bytes). \
             The repository was not found, or the token cannot access it.",
        ),
    ] {
        let provider = provider([response(404, body)]);
        let verdict = dispatch_create(&provider, &id(), create_request(None)).unwrap_err();
        let WireCreateVerdict::NotCreated(error) = *verdict else {
            panic!("expected NotCreated");
        };
        assert_eq!(error.kind, ProviderErrorKind::ApiError);
        assert_eq!(error.status, Some(404));
        assert_eq!(error.operation.as_deref(), Some("actions.OidcSubject.PUT"));
        assert_eq!(error.message, expected_message);
        assert_eq!(provider.api.requests.borrow().len(), 1);
    }
}

#[test]
fn pre_send_create_failure_is_not_created() {
    let provider = provider([Err(TransportError::NotSent)]);
    assert!(matches!(
        *dispatch_create(&provider, &id(), create_request(None)).unwrap_err(),
        WireCreateVerdict::NotCreated(_)
    ));
    assert_eq!(provider.api.requests.borrow().len(), 1);
}

#[test]
fn uncertain_create_preserves_identifier_and_observed_state() {
    for failure in [
        Err(TransportError::MaybeSent),
        response(301, json!({"message": "Moved Permanently"})),
        response(307, json!({"message": "Temporary Redirect"})),
        response(500, json!({"message": "Internal Server Error"})),
        response(503, json!({"message": "Unavailable"})),
    ] {
        let provider = provider([failure, response(200, json!({"use_default": true}))]);
        let mut request = create_request(Some(&["repo"]));
        request
            .resource
            .attributes
            .insert("use_default".into(), Value::Bool(false));
        let verdict = dispatch_create(&provider, &id(), request).unwrap_err();
        let WireCreateVerdict::IdentifierReported(failure) = *verdict else {
            panic!("uncertain create must retain its known identifier");
        };
        assert_eq!(failure.identifier.as_str(), REPOSITORY);
        assert_eq!(failure.sent, Sent::only(["repository"]));
        let Observation::Read(ReadOutcome::Found { state }) = failure.observation else {
            panic!("expected read-back observation");
        };
        assert_eq!(state.attributes, state_attributes(None));
        assert_eq!(provider.api.requests.borrow().len(), 2);
        assert_eq!(provider.api.requests.borrow()[1].method(), "GET");
    }
}

#[test]
fn uncertain_create_keeps_identifier_when_read_back_fails() {
    for (read_back, expected_status) in [
        (Err(TransportError::MaybeSent), None),
        (response(502, json!({"message": "Bad Gateway"})), Some(502)),
        (response(404, json!({"message": "Not Found"})), Some(404)),
    ] {
        let provider = provider([Err(TransportError::MaybeSent), read_back]);
        let verdict = dispatch_create(&provider, &id(), create_request(None)).unwrap_err();
        let WireCreateVerdict::IdentifierReported(failure) = *verdict else {
            panic!("expected IdentifierReported");
        };
        assert_eq!(failure.identifier.as_str(), REPOSITORY);
        assert_eq!(failure.sent, Sent::only(["repository"]));
        let Observation::ReadFailed(error) = failure.observation else {
            panic!("expected read-back failure");
        };
        assert_eq!(error.status, expected_status);
        if expected_status == Some(404) {
            assert_eq!(error.kind, ProviderErrorKind::ApiError);
            assert_eq!(
                error.message,
                "GitHub API returned HTTP 404 Not Found: Not Found. \
                 The repository was not found, or the token cannot access it. \
                 If the repository really was deleted, remove this resource from state with a \
                 `removed { from = ... }` block."
            );
        }
    }
}

#[test]
fn invalid_create_is_not_created_before_any_request() {
    for (key, value) in [
        ("repository", Value::String(String::new())),
        ("repository", Value::String("owner/.".into())),
        ("repository", Value::String("owner/..".into())),
        ("repository", Value::String("bad/repo/extra".into())),
        ("claim_keys", keys(&["unknown_claim"])),
        ("claim_keys", Value::Int(1)),
    ] {
        let provider = provider([]);
        let mut request = create_request(None);
        request.resource.attributes.insert(key.into(), value);
        assert!(matches!(
            *dispatch_create(&provider, &id(), request).unwrap_err(),
            WireCreateVerdict::NotCreated(_)
        ));
        assert!(provider.api.requests.borrow().is_empty());
    }
}

#[test]
fn token_is_redacted_in_debug_and_errors() {
    let token = GithubToken::new(TEST_TOKEN.into()).unwrap();
    assert!(!format!("{token:?}").contains(TEST_TOKEN));
    let request_id = "ABCD:0123:4567:89AB:CDEF";
    for (status, expected_message) in [
        (
            401,
            "GitHub API returned HTTP 401 Unauthorized: rejected Bearer [REDACTED]",
        ),
        (
            404,
            "GitHub API returned HTTP 404 Not Found: rejected Bearer [REDACTED]. \
             The repository was not found, or the token cannot access it. \
             If the repository really was deleted, remove this resource from state with a \
             `removed { from = ... }` block.",
        ),
        (
            500,
            "GitHub API returned HTTP 500 Internal Server Error: rejected Bearer [REDACTED]",
        ),
    ] {
        let echoed = format!("rejected Bearer {TEST_TOKEN}");
        let response = Ok(Response::builder()
            .status(status)
            .header("x-github-request-id", request_id)
            .body(serde_json::to_vec(&json!({"message": echoed})).unwrap())
            .unwrap());
        let provider = provider([response]);
        assert!(!format!("{provider:?}").contains(TEST_TOKEN));
        let error = provider
            .read(&id(), &identifier(), ReadRequest)
            .unwrap_err();
        assert_eq!(error.kind, ProviderErrorKind::ApiError);
        assert_eq!(error.status, Some(status));
        assert_eq!(error.operation.as_deref(), Some("actions.OidcSubject.GET"));
        assert!(!format!("{error:?}").contains(TEST_TOKEN));
        assert!(!serde_json::to_string(&error).unwrap().contains(TEST_TOKEN));
        assert_eq!(error.message, expected_message);
        assert_eq!(error.request_id.as_deref(), Some(request_id));
    }
    let body = format!("upstream error {TEST_TOKEN}").into_bytes();
    let body_len = body.len();
    let provider = provider([Ok(Response::builder().status(502).body(body).unwrap())]);
    let error = provider
        .read(&id(), &identifier(), ReadRequest)
        .unwrap_err();
    assert!(!format!("{error:?}").contains(TEST_TOKEN));
    assert!(!serde_json::to_string(&error).unwrap().contains(TEST_TOKEN));
    assert_eq!(
        error.message,
        format!(
            "GitHub API returned HTTP 502 Bad Gateway with no error message in the body ({body_len} bytes)"
        )
    );
}

#[test]
fn large_html_error_body_is_not_included_in_message() {
    let body = format!(
        "<html><body><h1>502 Bad Gateway</h1>{}</body></html>",
        "upstream proxy error • ".repeat(2048)
    )
    .into_bytes();
    let body_len = body.len();
    let provider = provider([Ok(Response::builder().status(502).body(body).unwrap())]);
    let error = provider
        .read(&id(), &identifier(), ReadRequest)
        .unwrap_err();
    assert_eq!(
        error.message,
        format!(
            "GitHub API returned HTTP 502 Bad Gateway with no error message in the body ({body_len} bytes)"
        )
    );
}

#[test]
fn json_error_bodies_without_a_string_message_are_summarized() {
    for body in [json!({}), json!({"message": 502}), json!({"message": null})] {
        let body_len = serde_json::to_vec(&body).unwrap().len();
        let provider = provider([response(502, body)]);
        let error = provider
            .read(&id(), &identifier(), ReadRequest)
            .unwrap_err();
        assert_eq!(
            error.message,
            format!(
                "GitHub API returned HTTP 502 Bad Gateway with no error message in the body ({body_len} bytes)"
            )
        );
    }
}

// Each environment test re-executes only itself, so Rust 2024 tests never
// mutate the process environment while other tests are running.
fn initialization_in_child(test: &str, token: Option<&std::ffi::OsStr>) -> bool {
    if std::env::var_os("CARINA_GITHUB_INITIALIZATION_CHILD").is_some() {
        return true;
    }
    let mut child = std::process::Command::new(std::env::current_exe().unwrap());
    child
        .args(["--exact", test])
        .env("CARINA_GITHUB_INITIALIZATION_CHILD", "1")
        .env_remove("GITHUB_TOKEN");
    if let Some(token) = token {
        child.env("GITHUB_TOKEN", token);
    }
    let output = child.output().unwrap();
    assert!(
        output.status.success(),
        "initialization subprocess failed: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    false
}

#[test]
fn missing_github_token_fails_before_any_request() {
    if !initialization_in_child("tests::missing_github_token_fails_before_any_request", None) {
        return;
    }
    assert_initialization_failure();
}

#[test]
fn empty_github_token_fails_before_any_request() {
    if !initialization_in_child(
        "tests::empty_github_token_fails_before_any_request",
        Some(std::ffi::OsStr::new("")),
    ) {
        return;
    }
    assert_initialization_failure();
}

fn assert_initialization_failure() {
    let mut provider = GithubProvider::<FakeApi>::default();
    let message = provider.initialize(&BTreeMap::new()).unwrap_err();
    assert!(message.contains("GITHUB_TOKEN"));
    assert!(message.contains("configuration"));
    assert!(message.contains("must be set and non-empty"));
    assert!(dispatch_create(&provider, &id(), create_request(None)).is_err());
    assert!(provider.api.requests.borrow().is_empty());
}

#[cfg(unix)]
#[test]
fn non_unicode_github_token_is_a_redacted_configuration_error() {
    use std::os::unix::ffi::OsStrExt;

    let token = std::ffi::OsStr::from_bytes(b"secret-token-\xff");
    if !initialization_in_child(
        "tests::non_unicode_github_token_is_a_redacted_configuration_error",
        Some(token),
    ) {
        return;
    }
    let mut provider = provider([]);
    let message = provider.initialize(&BTreeMap::new()).unwrap_err();
    assert_eq!(
        message,
        "GitHub provider configuration: GITHUB_TOKEN is not valid Unicode"
    );
    assert!(!message.contains("secret-token"));
    assert!(provider.token.is_none());
    assert!(dispatch_create(&provider, &id(), create_request(None)).is_err());
    assert!(provider.api.requests.borrow().is_empty());
}

#[test]
fn initialize_reads_token_from_environment() {
    if !initialization_in_child(
        "tests::initialize_reads_token_from_environment",
        Some(std::ffi::OsStr::new(TEST_TOKEN)),
    ) {
        return;
    }
    let mut provider = provider([response(201, json!({}))]);
    provider.token = None;
    assert!(matches!(
        provider.initialize(&BTreeMap::new()).unwrap(),
        ReferenceScope::Unavailable
    ));
    assert!(provider.api.requests.borrow().is_empty());
    dispatch_create(&provider, &id(), create_request(None)).unwrap();
}

#[test]
fn invalid_token_header_is_a_redacted_configuration_error() {
    let secret = "secret\ninvalid-header";
    let message = GithubToken::new(secret.into()).unwrap_err();
    assert!(message.contains("GITHUB_TOKEN"));
    assert!(!message.contains(secret));
}

fn core_keys(keys: &[&str]) -> CoreValue {
    CoreValue::Concrete(ConcreteValue::List(
        keys.iter()
            .map(|key| CoreValue::Concrete(ConcreteValue::enum_identifier(*key)))
            .collect(),
    ))
}

#[test]
fn repository_validation_handles_dot_path_segments() {
    for text in ["owner/.", "owner/.."] {
        assert!(validate_repository(text).is_err(), "accepted {text}");
    }
    for text in ["owner/.github", "owner/..x", "owner/..."] {
        validate_repository(text).unwrap();
    }
}

#[test]
fn schema_repository_is_required_create_only_identity_and_refined() {
    let schema = GithubProvider::<FakeApi>::default().schemas().remove(0);
    assert_eq!(schema.as_schema().resource_type(), "actions.OidcSubject");
    let repository = schema.effective_attribute("repository").unwrap();
    assert!(repository.is_required());
    assert!(repository.is_create_only());
    assert_eq!(repository.identity_mode(), IdentityMode::Required);
    for text in [
        "missing-slash",
        "owner/",
        "/repo",
        "owner/repo/extra",
        "owner_/repo",
        "owner/a?b",
        "owner/a b",
        "https://github.com/owner/repo",
        "owner/repo\n",
        "owner/.",
        "owner/..",
    ] {
        let value: CoreValue = CoreValue::Concrete(ConcreteValue::String(text.into()));
        assert!(
            repository.validate(&value).is_err(),
            "accepted malformed repository"
        );
    }
    for text in [
        REPOSITORY,
        "Owner-1/repo.name_1-2",
        "owner/.github",
        "owner/..x",
        "owner/...",
    ] {
        let value: CoreValue = CoreValue::Concrete(ConcreteValue::String(text.into()));
        repository.validate(&value).unwrap();
    }
}

#[test]
fn schema_use_default_is_provider_populated_bool() {
    let provider = GithubProvider::<FakeApi>::default();
    let cache = ProviderSchemaCache::from_provider(&provider).unwrap();
    let mut registry = SchemaRegistry::new();
    registry
        .insert_provider_witness("github", cache.witness())
        .unwrap();
    let schema = registry
        .get("github", "actions.OidcSubject", SchemaKind::Resource)
        .unwrap();
    let use_default = schema.schema().effective_attribute("use_default").unwrap();
    assert!(use_default.effective_input_mode().is_provider_populated());
    assert!(!use_default.is_create_only());
    let catalog = ProviderCustomTypes::default();
    let sink = schema.assignment_sink(use_default.field_type(), &catalog);
    for (source, assignable) in [
        (CompiledAttributeType::bool(), true),
        (CompiledAttributeType::string(), false),
    ] {
        assert_eq!(
            TypeInSchema::schemaless(&source)
                .assignment_to(sink)
                .require_loaded()
                .unwrap(),
            assignable
        );
    }
    let wire: serde_json::Value =
        serde_json::from_str(&carina_plugin_sdk::provider_schema_to_json(cache.witness()).unwrap())
            .unwrap();
    assert_eq!(
        wire["resources"][0]["attributes"]["use_default"]["attr_type"],
        json!({"type": "Bool"})
    );
}

#[test]
fn schema_rejects_authored_use_default() {
    let provider = GithubProvider::<FakeApi>::default();
    let cache = ProviderSchemaCache::from_provider(&provider).unwrap();
    let mut registry = SchemaRegistry::new();
    registry
        .insert_provider_witness("github", cache.witness())
        .unwrap();
    let schema = registry
        .get("github", "actions.OidcSubject", SchemaKind::Resource)
        .unwrap();
    let resolver =
        |_: carina_plugin_sdk::core::parser::CustomTypeValidationTarget<'_>, _: &CoreValue| Ok(());
    let mut attrs: std::collections::HashMap<String, CoreValue> =
        std::collections::HashMap::from([(
            "repository".into(),
            CoreValue::Concrete(ConcreteValue::String(REPOSITORY.into())),
        )]);
    schema
        .validate_with_resolver(&attrs, &|_| false, &resolver)
        .unwrap();
    for value in [true, false] {
        attrs.insert(
            "use_default".into(),
            CoreValue::Concrete(ConcreteValue::Bool(value)),
        );
        let errors = schema
            .validate_with_resolver(&attrs, &|_| false, &resolver)
            .unwrap_err();
        assert!(errors.iter().any(
            |error| matches!(error, TypeError::ReadOnlyAttribute { name } if name == "use_default")
        ));
    }
}

#[test]
fn provider_advertises_desired_normalization() {
    let provider = GithubProvider::<FakeApi>::default();
    assert_eq!(provider.capabilities(), ["normalize_desired"]);
    assert_eq!(provider.info().capabilities, ["normalize_desired"]);
}

#[test]
fn schema_rejects_empty_keys() {
    let schema = GithubProvider::<FakeApi>::default().schemas().remove(0);
    assert!(
        schema
            .effective_attribute("claim_keys")
            .unwrap()
            .validate(&core_keys(&[]))
            .is_err()
    );
}

#[test]
fn schema_rejects_bare_and_qualified_duplicates() {
    let schema = GithubProvider::<FakeApi>::default().schemas().remove(0);
    let keys = schema.effective_attribute("claim_keys").unwrap();
    for duplicate in ["repo", "github.actions.OidcSubject.ClaimKey.repo"] {
        assert!(matches!(
            keys.validate(&core_keys(&["repo", duplicate])),
            Err(TypeError::DuplicateListElement { .. })
        ));
    }
}

#[test]
fn schema_accepts_bare_and_qualified_enum_spellings_without_aliases() {
    let schema = GithubProvider::<FakeApi>::default().schemas().remove(0);
    let keys = schema.effective_attribute("claim_keys").unwrap();
    assert!(!keys.is_required());
    assert!(!keys.is_create_only());
    keys.validate(&core_keys(&["repo", "context", "job_workflow_ref"]))
        .unwrap();
    keys.validate(&core_keys(&[
        "github.actions.OidcSubject.ClaimKey.repo",
        "github.actions.OidcSubject.ClaimKey.context",
    ]))
    .unwrap();
    for unsupported in [
        "sub",
        "aud",
        "exp",
        "iat",
        "iss",
        "jti",
        "nbf",
        "repo_property_team",
        "jobWorkflowRef",
        "unknown_claim",
    ] {
        assert!(
            keys.validate(&core_keys(&[unsupported])).is_err(),
            "accepted {unsupported}"
        );
    }
}

#[test]
fn schema_accepts_exact_design_claim_keys() {
    let expected = [
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
    let provider = GithubProvider::<FakeApi>::default();
    let schema = provider.schemas().remove(0);
    schema
        .effective_attribute("claim_keys")
        .unwrap()
        .validate(&core_keys(&expected))
        .unwrap();
    let cache = ProviderSchemaCache::from_provider(&provider).unwrap();
    let wire: carina_plugin_sdk::protocol::types::ProviderSchemaEnvelope =
        serde_json::from_str(&carina_plugin_sdk::provider_schema_to_json(cache.witness()).unwrap())
            .unwrap();
    let carina_plugin_sdk::protocol::types::AttributeType::List {
        element_type,
        ordered,
        length,
        elements,
        ..
    } = &wire.resources[0].attributes["claim_keys"].attr_type
    else {
        panic!("expected list");
    };
    assert!(*ordered);
    assert_eq!(*length, Some((Some(1), None)));
    assert_eq!(
        *elements,
        carina_plugin_sdk::protocol::types::ListElements::Unique
    );
    let carina_plugin_sdk::protocol::types::AttributeType::StringEnum {
        values,
        name,
        namespace,
        dsl_aliases,
    } = element_type.as_ref()
    else {
        panic!("expected closed enum");
    };
    assert_eq!(values, &expected);
    assert_eq!(name, "ClaimKey");
    assert_eq!(namespace.as_deref(), Some("github.actions.OidcSubject"));
    assert!(dsl_aliases.is_empty());
}

#[test]
fn provider_assignability_edges_match_native_witness_and_wire_schema() {
    let provider = GithubProvider::<HttpGithubApi>::default();
    let edges = provider.assignability_edges();
    assert!(edges.is_empty());

    let cache = ProviderSchemaCache::from_provider(&provider).unwrap();
    assert_eq!(cache.witness().assignability_edges(), edges.as_slice());

    let wire: carina_plugin_sdk::protocol::types::ProviderSchemaEnvelope =
        serde_json::from_str(&carina_plugin_sdk::provider_schema_to_json(cache.witness()).unwrap())
            .unwrap();
    assert!(wire.assignability_edges.is_empty());
}

#[test]
fn provider_schema_is_admitted_and_protected_with_empty_config() {
    let provider = GithubProvider::<FakeApi>::default();
    assert_eq!(provider.info().name, "github");
    let cache = ProviderSchemaCache::from_provider(&provider).unwrap();
    let wire: serde_json::Value =
        serde_json::from_str(&carina_plugin_sdk::provider_schema_to_json(cache.witness()).unwrap())
            .unwrap();
    assert_eq!(wire["resources"][0]["default_prevent_destroy"], true);
    assert_eq!(
        wire["resources"][0]["attributes"]["claim_keys"]["attr_type"]["ordered"],
        true
    );
    assert_eq!(
        carina_plugin_sdk::provider_config_schema_to_json(cache.config()).unwrap(),
        "{}"
    );
}

#[test]
fn egress_pattern_is_exact_and_parseable() {
    let pattern = carina_plugin_sdk::protocol::HostPattern::parse("api.github.com").unwrap();
    assert!(pattern.matches("api.github.com"));
    assert!(!pattern.matches("uploads.github.com"));
    assert!(!pattern.matches("evil.api.github.com"));
    let entrypoint = include_str!("main.rs");
    assert!(entrypoint.contains("egress: [\"api.github.com\"]"));
}
