//! The HTTP seam: native tests supply a fake and WASM uses the SDK transport.

use carina_plugin_sdk::protected_material::RegisteredRequest;
use http::Response;

/// Whether a transport failure could have transmitted the request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransportError {
    NotSent,
    MaybeSent,
}

pub trait GithubApi {
    fn send(&self, request: RegisteredRequest) -> Result<Response<Vec<u8>>, TransportError>;
}

#[derive(Default)]
pub struct HttpGithubApi;

impl GithubApi for HttpGithubApi {
    fn send(&self, request: RegisteredRequest) -> Result<Response<Vec<u8>>, TransportError> {
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
