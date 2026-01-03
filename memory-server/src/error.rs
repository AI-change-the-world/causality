//! Error handling module for Memory Server
//!
//! Defines error types, error codes, and HTTP response conversion.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;
use thiserror::Error;
use utoipa::ToSchema;
use uuid::Uuid;

/// Error codes for machine-readable error identification
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    /// Request parameter validation failed
    ValidationError,
    /// Invalid update mode
    InvalidUpdateMode,
    /// Memory ID not found
    MemoryNotFound,
    /// Event ID not found
    EventNotFound,
    /// Embedding provider not found
    ProviderNotFound,
    /// Specified provider is disabled
    ProviderDisabled,
    /// No default provider configured
    NoDefaultProvider,
    /// Embedding generation failed after retries
    EmbeddingFailed,
    /// Provider rate limit exceeded
    RateLimitExceeded,
    /// PostgreSQL operation failed
    DatabaseError,
    /// Qdrant operation failed
    VectorDbError,
    /// Configuration loading/validation failed
    ConfigError,
    /// Internal server error
    InternalError,
    /// Event content is too short for processing
    EventContentTooShort,
    /// No valid memories could be extracted from event
    NoMemoriesExtracted,
    /// Query enhancement failed
    QueryEnhancementFailed,
}

impl ErrorCode {
    /// Get the HTTP status code for this error
    pub fn status_code(&self) -> StatusCode {
        match self {
            ErrorCode::ValidationError
            | ErrorCode::InvalidUpdateMode
            | ErrorCode::ProviderDisabled
            | ErrorCode::EventContentTooShort => StatusCode::BAD_REQUEST,

            ErrorCode::MemoryNotFound | ErrorCode::ProviderNotFound | ErrorCode::EventNotFound => {
                StatusCode::NOT_FOUND
            }

            ErrorCode::RateLimitExceeded => StatusCode::TOO_MANY_REQUESTS,

            ErrorCode::NoMemoriesExtracted => StatusCode::UNPROCESSABLE_ENTITY,

            ErrorCode::NoDefaultProvider
            | ErrorCode::EmbeddingFailed
            | ErrorCode::DatabaseError
            | ErrorCode::VectorDbError
            | ErrorCode::ConfigError
            | ErrorCode::QueryEnhancementFailed
            | ErrorCode::InternalError => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

/// Standard error response format
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ErrorResponse {
    /// Machine-readable error code
    pub code: ErrorCode,
    /// Human-readable error message
    pub message: String,
    /// Additional error details (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
    /// Request ID for tracing
    pub request_id: String,
}

impl ErrorResponse {
    /// Create a new error response
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: None,
            request_id: Uuid::new_v4().to_string(),
        }
    }

    /// Create a new error response with details
    pub fn with_details(
        code: ErrorCode,
        message: impl Into<String>,
        details: serde_json::Value,
    ) -> Self {
        Self {
            code,
            message: message.into(),
            details: Some(details),
            request_id: Uuid::new_v4().to_string(),
        }
    }

    /// Set a specific request ID
    pub fn with_request_id(mut self, request_id: impl Into<String>) -> Self {
        self.request_id = request_id.into();
        self
    }
}

/// Application error type
#[derive(Debug, Error)]
pub enum AppError {
    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Invalid update mode: {0}")]
    InvalidUpdateMode(String),

    #[error("Memory not found: {0}")]
    MemoryNotFound(Uuid),

    #[error("Event not found: {0}")]
    EventNotFound(Uuid),

    #[error("Provider not found: {0}")]
    ProviderNotFound(String),

    #[error("Provider disabled: {0}")]
    ProviderDisabled(String),

    #[error("No default embedding provider configured")]
    NoDefaultProvider,

    #[error("Embedding failed after retries: {0}")]
    EmbeddingFailed(String),

    #[error("Rate limit exceeded for provider: {0}")]
    RateLimitExceeded(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Vector database error: {0}")]
    VectorDb(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Event content too short: minimum {min} characters required, got {actual}")]
    EventContentTooShort { min: usize, actual: usize },

    #[error("No valid memories could be extracted from event")]
    NoMemoriesExtracted,

    #[error("Query enhancement failed: {0}")]
    QueryEnhancementFailed(String),
}

impl AppError {
    /// Get the error code for this error
    pub fn error_code(&self) -> ErrorCode {
        match self {
            AppError::Validation(_) => ErrorCode::ValidationError,
            AppError::InvalidUpdateMode(_) => ErrorCode::InvalidUpdateMode,
            AppError::MemoryNotFound(_) => ErrorCode::MemoryNotFound,
            AppError::EventNotFound(_) => ErrorCode::EventNotFound,
            AppError::ProviderNotFound(_) => ErrorCode::ProviderNotFound,
            AppError::ProviderDisabled(_) => ErrorCode::ProviderDisabled,
            AppError::NoDefaultProvider => ErrorCode::NoDefaultProvider,
            AppError::EmbeddingFailed(_) => ErrorCode::EmbeddingFailed,
            AppError::RateLimitExceeded(_) => ErrorCode::RateLimitExceeded,
            AppError::Database(_) => ErrorCode::DatabaseError,
            AppError::VectorDb(_) => ErrorCode::VectorDbError,
            AppError::Config(_) => ErrorCode::ConfigError,
            AppError::Internal(_) => ErrorCode::InternalError,
            AppError::EventContentTooShort { .. } => ErrorCode::EventContentTooShort,
            AppError::NoMemoriesExtracted => ErrorCode::NoMemoriesExtracted,
            AppError::QueryEnhancementFailed(_) => ErrorCode::QueryEnhancementFailed,
        }
    }

    /// Convert to ErrorResponse
    pub fn to_response(&self) -> ErrorResponse {
        ErrorResponse::new(self.error_code(), self.to_string())
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let error_code = self.error_code();
        let status = error_code.status_code();
        let response = self.to_response();

        tracing::error!(
            error_code = ?error_code,
            message = %response.message,
            request_id = %response.request_id,
            "Request failed"
        );

        (status, Json(response)).into_response()
    }
}

/// Result type alias for AppError
pub type AppResult<T> = Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_code_status_codes() {
        assert_eq!(
            ErrorCode::ValidationError.status_code(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            ErrorCode::MemoryNotFound.status_code(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            ErrorCode::RateLimitExceeded.status_code(),
            StatusCode::TOO_MANY_REQUESTS
        );
        assert_eq!(
            ErrorCode::DatabaseError.status_code(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[test]
    fn test_event_processing_error_codes() {
        // EventContentTooShort should return BAD_REQUEST (400)
        assert_eq!(
            ErrorCode::EventContentTooShort.status_code(),
            StatusCode::BAD_REQUEST
        );
        // NoMemoriesExtracted should return UNPROCESSABLE_ENTITY (422)
        assert_eq!(
            ErrorCode::NoMemoriesExtracted.status_code(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
        // QueryEnhancementFailed should return INTERNAL_SERVER_ERROR (500)
        assert_eq!(
            ErrorCode::QueryEnhancementFailed.status_code(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[test]
    fn test_error_response_creation() {
        let response = ErrorResponse::new(ErrorCode::ValidationError, "Invalid input");
        assert_eq!(response.code, ErrorCode::ValidationError);
        assert_eq!(response.message, "Invalid input");
        assert!(response.details.is_none());
        assert!(!response.request_id.is_empty());
    }

    #[test]
    fn test_error_response_with_details() {
        let details = serde_json::json!({"field": "category", "value": "invalid"});
        let response = ErrorResponse::with_details(
            ErrorCode::ValidationError,
            "Invalid category format",
            details.clone(),
        );
        assert_eq!(response.code, ErrorCode::ValidationError);
        assert_eq!(response.details, Some(details));
    }

    #[test]
    fn test_app_error_to_error_code() {
        assert_eq!(
            AppError::Validation("test".to_string()).error_code(),
            ErrorCode::ValidationError
        );
        assert_eq!(
            AppError::MemoryNotFound(Uuid::new_v4()).error_code(),
            ErrorCode::MemoryNotFound
        );
        assert_eq!(
            AppError::InvalidUpdateMode("test".to_string()).error_code(),
            ErrorCode::InvalidUpdateMode
        );
    }

    #[test]
    fn test_event_processing_app_errors() {
        // Test EventContentTooShort
        let err = AppError::EventContentTooShort { min: 10, actual: 5 };
        assert_eq!(err.error_code(), ErrorCode::EventContentTooShort);
        assert_eq!(
            err.to_string(),
            "Event content too short: minimum 10 characters required, got 5"
        );

        // Test NoMemoriesExtracted
        let err = AppError::NoMemoriesExtracted;
        assert_eq!(err.error_code(), ErrorCode::NoMemoriesExtracted);
        assert_eq!(
            err.to_string(),
            "No valid memories could be extracted from event"
        );

        // Test QueryEnhancementFailed
        let err = AppError::QueryEnhancementFailed("LLM timeout".to_string());
        assert_eq!(err.error_code(), ErrorCode::QueryEnhancementFailed);
        assert_eq!(err.to_string(), "Query enhancement failed: LLM timeout");
    }
}
