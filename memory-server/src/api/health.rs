//! Health & Metrics API handlers
//!
//! Implements:
//! - GET /health - Health check endpoint
//! - GET /metrics - Prometheus metrics endpoint

use axum::{extract::State, http::StatusCode, response::IntoResponse, routing::get, Json, Router};
use serde::Serialize;
use std::time::Instant;
use utoipa::ToSchema;

use crate::api::AppState;

/// Health status enum
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum HealthStatus {
    /// All components are healthy
    Healthy,
    /// Some components are degraded but service is operational
    Degraded,
    /// Service is unhealthy
    Unhealthy,
}

/// Component health information
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ComponentHealth {
    /// Component status
    pub status: HealthStatus,
    /// Latency in milliseconds (if available)
    pub latency_ms: Option<u64>,
    /// Error message (if unhealthy)
    pub error: Option<String>,
}

impl ComponentHealth {
    /// Create a healthy component status
    pub fn healthy(latency_ms: u64) -> Self {
        ComponentHealth {
            status: HealthStatus::Healthy,
            latency_ms: Some(latency_ms),
            error: None,
        }
    }

    /// Create an unhealthy component status
    pub fn unhealthy(error: impl Into<String>) -> Self {
        ComponentHealth {
            status: HealthStatus::Unhealthy,
            latency_ms: None,
            error: Some(error.into()),
        }
    }

    /// Create a degraded component status
    pub fn degraded(latency_ms: u64, error: impl Into<String>) -> Self {
        ComponentHealth {
            status: HealthStatus::Degraded,
            latency_ms: Some(latency_ms),
            error: Some(error.into()),
        }
    }
}

/// Health check response
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct HealthResponse {
    /// Overall service status
    pub status: HealthStatus,
    /// PostgreSQL health
    pub postgres: ComponentHealth,
    /// Qdrant health
    pub qdrant: ComponentHealth,
    /// Embedding provider health
    pub embedding: ComponentHealth,
    /// Service version
    pub version: String,
    /// Uptime in seconds
    pub uptime_seconds: u64,
}

impl HealthResponse {
    /// Compute overall status from component statuses
    pub fn compute_overall_status(&mut self) {
        let statuses = [
            self.postgres.status,
            self.qdrant.status,
            self.embedding.status,
        ];

        if statuses.iter().any(|s| *s == HealthStatus::Unhealthy) {
            // If any critical component is unhealthy, service is unhealthy
            // PostgreSQL is critical
            if self.postgres.status == HealthStatus::Unhealthy {
                self.status = HealthStatus::Unhealthy;
            } else {
                // Qdrant and embedding can be degraded
                self.status = HealthStatus::Degraded;
            }
        } else if statuses.iter().any(|s| *s == HealthStatus::Degraded) {
            self.status = HealthStatus::Degraded;
        } else {
            self.status = HealthStatus::Healthy;
        }
    }
}

/// Application start time for uptime calculation
static START_TIME: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

/// Initialize the start time
pub fn init_start_time() {
    START_TIME.get_or_init(Instant::now);
}

/// Get uptime in seconds
fn get_uptime_seconds() -> u64 {
    START_TIME
        .get()
        .map(|start| start.elapsed().as_secs())
        .unwrap_or(0)
}

/// Create health routes
pub fn health_routes() -> Router<AppState> {
    Router::new()
        .route("/health", get(health_check))
        .route("/metrics", get(metrics))
}

/// GET /health - Health check endpoint
///
/// Returns the health status of the service and its dependencies:
/// - PostgreSQL connectivity
/// - Qdrant connectivity
/// - Embedding provider availability
#[utoipa::path(
    get,
    path = "/health",
    tag = "health",
    responses(
        (status = 200, description = "Service is healthy or degraded", body = HealthResponse),
        (status = 503, description = "Service is unhealthy", body = HealthResponse)
    )
)]
pub async fn health_check(State(state): State<AppState>) -> impl IntoResponse {
    // Check PostgreSQL health
    let postgres_health = check_postgres_health(&state).await;

    // Check Qdrant health (placeholder - will be implemented with Qdrant integration)
    let qdrant_health = check_qdrant_health(&state).await;

    // Check embedding provider health
    let embedding_health = check_embedding_health(&state).await;

    let mut response = HealthResponse {
        status: HealthStatus::Healthy,
        postgres: postgres_health,
        qdrant: qdrant_health,
        embedding: embedding_health,
        version: env!("CARGO_PKG_VERSION").to_string(),
        uptime_seconds: get_uptime_seconds(),
    };

    response.compute_overall_status();

    let status_code = match response.status {
        HealthStatus::Healthy => StatusCode::OK,
        HealthStatus::Degraded => StatusCode::OK,
        HealthStatus::Unhealthy => StatusCode::SERVICE_UNAVAILABLE,
    };

    (status_code, Json(response))
}

/// Check PostgreSQL health
async fn check_postgres_health(state: &AppState) -> ComponentHealth {
    let start = Instant::now();

    // Try to perform a simple query to check connectivity
    match state.lifecycle_manager.check_database_health().await {
        Ok(_) => ComponentHealth::healthy(start.elapsed().as_millis() as u64),
        Err(e) => ComponentHealth::unhealthy(e.to_string()),
    }
}

/// Check Qdrant health (placeholder)
async fn check_qdrant_health(_state: &AppState) -> ComponentHealth {
    // TODO: Implement actual Qdrant health check when Qdrant integration is complete
    // For now, return healthy as a placeholder
    ComponentHealth::healthy(0)
}

/// Check embedding provider health
async fn check_embedding_health(_state: &AppState) -> ComponentHealth {
    let start = Instant::now();

    // TODO remove later
    ComponentHealth::healthy(start.elapsed().as_millis() as u64)
}

/// GET /metrics - Prometheus metrics endpoint
///
/// Returns metrics in Prometheus format.
#[utoipa::path(
    get,
    path = "/metrics",
    tag = "health",
    responses(
        (status = 200, description = "Prometheus metrics", content_type = "text/plain")
    )
)]
pub async fn metrics() -> impl IntoResponse {
    // TODO: Implement actual metrics collection
    // For now, return basic metrics
    let metrics = format!(
        r#"# HELP memory_server_uptime_seconds Time since server started
# TYPE memory_server_uptime_seconds gauge
memory_server_uptime_seconds {}

# HELP memory_server_info Server information
# TYPE memory_server_info gauge
memory_server_info{{version="{}"}} 1
"#,
        get_uptime_seconds(),
        env!("CARGO_PKG_VERSION"),
    );

    (
        StatusCode::OK,
        [("content-type", "text/plain; charset=utf-8")],
        metrics,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_component_health_healthy() {
        let health = ComponentHealth::healthy(50);
        assert_eq!(health.status, HealthStatus::Healthy);
        assert_eq!(health.latency_ms, Some(50));
        assert!(health.error.is_none());
    }

    #[test]
    fn test_component_health_unhealthy() {
        let health = ComponentHealth::unhealthy("Connection refused");
        assert_eq!(health.status, HealthStatus::Unhealthy);
        assert!(health.latency_ms.is_none());
        assert_eq!(health.error, Some("Connection refused".to_string()));
    }

    #[test]
    fn test_component_health_degraded() {
        let health = ComponentHealth::degraded(100, "High latency");
        assert_eq!(health.status, HealthStatus::Degraded);
        assert_eq!(health.latency_ms, Some(100));
        assert_eq!(health.error, Some("High latency".to_string()));
    }

    #[test]
    fn test_health_response_compute_overall_healthy() {
        let mut response = HealthResponse {
            status: HealthStatus::Healthy,
            postgres: ComponentHealth::healthy(10),
            qdrant: ComponentHealth::healthy(20),
            embedding: ComponentHealth::healthy(5),
            version: "0.1.0".to_string(),
            uptime_seconds: 100,
        };

        response.compute_overall_status();
        assert_eq!(response.status, HealthStatus::Healthy);
    }

    #[test]
    fn test_health_response_compute_overall_degraded() {
        let mut response = HealthResponse {
            status: HealthStatus::Healthy,
            postgres: ComponentHealth::healthy(10),
            qdrant: ComponentHealth::unhealthy("Connection failed"),
            embedding: ComponentHealth::healthy(5),
            version: "0.1.0".to_string(),
            uptime_seconds: 100,
        };

        response.compute_overall_status();
        assert_eq!(response.status, HealthStatus::Degraded);
    }

    #[test]
    fn test_health_response_compute_overall_unhealthy() {
        let mut response = HealthResponse {
            status: HealthStatus::Healthy,
            postgres: ComponentHealth::unhealthy("Connection refused"),
            qdrant: ComponentHealth::healthy(20),
            embedding: ComponentHealth::healthy(5),
            version: "0.1.0".to_string(),
            uptime_seconds: 100,
        };

        response.compute_overall_status();
        assert_eq!(response.status, HealthStatus::Unhealthy);
    }

    #[test]
    fn test_health_status_serialization() {
        assert_eq!(
            serde_json::to_string(&HealthStatus::Healthy).unwrap(),
            "\"healthy\""
        );
        assert_eq!(
            serde_json::to_string(&HealthStatus::Degraded).unwrap(),
            "\"degraded\""
        );
        assert_eq!(
            serde_json::to_string(&HealthStatus::Unhealthy).unwrap(),
            "\"unhealthy\""
        );
    }
}
