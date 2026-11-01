#[cfg(target_arch = "wasm32")]
use carina_provider_github::GithubProvider;

#[cfg(target_arch = "wasm32")]
carina_plugin_sdk::export_provider!(GithubProvider, http, egress: ["api.github.com"]);

// The host calls the component exports. Native builds exist for tests.
fn main() {}
