//! The HTTP seam: native tests supply a fake and WASM uses the SDK transport.

use http::{HeaderValue, Request, Response};
use std::fmt;

/// Whether a transport failure could have transmitted the request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransportError {
    NotSent,
    MaybeSent,
}

pub trait GithubApi {
    fn send(&self, request: Request<Vec<u8>>) -> Result<Response<Vec<u8>>, TransportError>;
}

#[derive(Default)]
pub struct HttpGithubApi;

impl GithubApi for HttpGithubApi {
    fn send(&self, request: Request<Vec<u8>>) -> Result<Response<Vec<u8>>, TransportError> {
        #[cfg(target_arch = "wasm32")]
        {
            use carina_plugin_sdk::wasi_http::{SendPhase, send_request};
            send_request(request).map_err(|error| match error.phase() {
                SendPhase::NotSent => TransportError::NotSent,
                SendPhase::MaybeSent => TransportError::MaybeSent,
            })
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            // Native builds only support fake APIs; the component owns HTTP.
            let _ = request;
            Err(TransportError::NotSent)
        }
    }
}

/// Credentials have no ordinary diagnostic or serialization representation.
pub(crate) struct GithubToken(String);

impl GithubToken {
    pub(crate) fn new(value: String) -> Result<Self, String> {
        if value.is_empty() {
            return Err(
                "GitHub provider configuration: GITHUB_TOKEN must be set and non-empty".into(),
            );
        }
        HeaderValue::from_str(&format!("Bearer {value}")).map_err(|_| {
            "GitHub provider configuration: GITHUB_TOKEN is not a valid authorization header"
                .to_string()
        })?;
        Ok(Self(value))
    }

    pub(crate) fn authorization(&self) -> HeaderValue {
        let mut header = HeaderValue::from_str(&format!("Bearer {}", self.0))
            .expect("validated GitHub token is a header value");
        header.set_sensitive(true);
        header
    }

    pub(crate) fn redact(&self, value: &str) -> String {
        value.replace(&self.0, "[REDACTED]")
    }
}

impl fmt::Debug for GithubToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("GithubToken([REDACTED])")
    }
}
