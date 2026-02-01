//! Config API handlers
//!
//! Provider management has been moved to config.yaml.
//! This module is kept for potential future configuration APIs.

use axum::Router;

use crate::api::AppState;

/// Create config routes (currently empty as providers are configured via config.yaml)
pub fn config_routes() -> Router<AppState> {
    Router::new()
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_config_routes_empty() {
        // Config routes are now empty as provider management moved to config.yaml
    }
}
