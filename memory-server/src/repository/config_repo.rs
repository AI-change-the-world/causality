//! Config Repository implementation
//!
//! This module is kept for potential future configuration storage needs.
//! Provider management has been moved to config.yaml.

use sqlx::PgPool;

/// Repository for configuration operations
#[derive(Clone)]
pub struct ConfigRepository {
    #[allow(dead_code)]
    pool: PgPool,
}

impl ConfigRepository {
    /// Create a new ConfigRepository
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_config_repository_creation() {
        // This is a placeholder test - actual database tests would require a test database
        // The ConfigRepository is now minimal as provider management moved to config.yaml
    }
}
