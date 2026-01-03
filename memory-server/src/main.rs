//! Memory Server - Application Entry Point
//!
//! This module handles the startup flow:
//! - Configuration loading
//! - Database connection pool initialization
//! - Database migration execution
//! - Qdrant client initialization
//! - HTTP server startup

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::http::header::HeaderName;
use memory_server::api::{create_router, health::init_start_time, AppState};
use memory_server::config::AppConfig;
use memory_server::repository::{
    AuditRepository, ConfigRepository, EventRepository, LlmProviderRepository, MemoryRepository,
    QdrantRepository,
};
use memory_server::service::{ConfigCenter, LifecycleManager, MemoryGuard, RetrievalEngine};
use sqlx::postgres::PgPoolOptions;
use tower_http::cors::{Any, CorsLayer};
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;
use tracing::{error, info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "memory_server=info,tower_http=debug,sqlx=warn".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Initialize start time for uptime tracking
    init_start_time();

    // Load configuration
    let config = AppConfig::load().map_err(|e| {
        error!(error = %e, "Failed to load configuration");
        anyhow::anyhow!("Configuration error: {}", e)
    })?;

    info!(
        host = %config.server.host,
        port = %config.server.port,
        "Starting Memory Server"
    );

    // Initialize database connection pool
    info!(
        max_connections = config.database.max_connections,
        min_connections = config.database.min_connections,
        "Connecting to PostgreSQL"
    );

    let pool = PgPoolOptions::new()
        .max_connections(config.database.max_connections)
        .min_connections(config.database.min_connections)
        .acquire_timeout(Duration::from_secs(30))
        .connect(&config.database.url)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to connect to PostgreSQL");
            anyhow::anyhow!("Database connection error: {}", e)
        })?;

    info!("PostgreSQL connection pool established");

    // Run database migrations (if enabled)
    if config.database.run_migrations {
        info!("Running database migrations");
        run_migrations(&pool).await?;
        info!("Database migrations completed");
    } else {
        info!("Database migrations skipped (run_migrations=false)");
    }

    // Initialize Qdrant client
    info!(
        url = %config.qdrant.url,
        collection = %config.qdrant.collection_name,
        "Initializing Qdrant client"
    );

    let qdrant_repo = QdrantRepository::new(&config.qdrant.url)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to initialize Qdrant client");
            anyhow::anyhow!("Qdrant initialization error: {}", e)
        })?;

    info!("Qdrant client initialized");

    // Create repositories
    let memory_repo = MemoryRepository::new(pool.clone());
    let event_repo = EventRepository::new(pool.clone());
    let audit_repo = AuditRepository::new(pool.clone());
    let config_repo = ConfigRepository::new(pool.clone());
    let llm_repo = LlmProviderRepository::new(pool.clone());

    // Create services
    let memory_guard = MemoryGuard::new_basic(
        memory_repo.clone(),
        event_repo.clone(),
        audit_repo.clone(),
        config.audit.enabled,
    );

    let retrieval_engine = RetrievalEngine::new(
        memory_repo.clone(),
        event_repo.clone(),
        audit_repo.clone(),
        config.retrieval.clone(),
        config.audit.enabled,
    );

    let config_center = ConfigCenter::with_full_support(config_repo.clone(), llm_repo, qdrant_repo);

    // Initialize config center (load providers from database)
    if let Err(e) = config_center.initialize().await {
        warn!(error = %e, "Failed to initialize config center - providers may not be loaded");
    }

    let lifecycle_manager = LifecycleManager::new(
        memory_repo.clone(),
        audit_repo.clone(),
        config.lifecycle.clone(),
        config.audit.enabled,
    );

    // Create application state
    let app_state = AppState {
        memory_guard: Arc::new(memory_guard),
        retrieval_engine,
        config_center,
        lifecycle_manager,
    };

    // Create router with middleware
    let x_request_id = HeaderName::from_static("x-request-id");
    let app = create_router(app_state)
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
        .layer(TraceLayer::new_for_http())
        .layer(TimeoutLayer::new(Duration::from_secs(300)))
        .layer(SetRequestIdLayer::new(
            x_request_id.clone(),
            MakeRequestUuid,
        ))
        .layer(PropagateRequestIdLayer::new(x_request_id));

    // Create socket address
    let addr: SocketAddr = format!("{}:{}", config.server.host, config.server.port)
        .parse()
        .map_err(|e| {
            error!(error = %e, "Invalid server address");
            anyhow::anyhow!("Invalid server address: {}", e)
        })?;

    info!(address = %addr, "HTTP server starting");

    // Start HTTP server
    let listener = tokio::net::TcpListener::bind(addr).await.map_err(|e| {
        error!(error = %e, "Failed to bind to address");
        anyhow::anyhow!("Failed to bind to {}: {}", addr, e)
    })?;

    info!(address = %addr, "Memory Server is ready to accept connections");

    axum::serve(listener, app).await.map_err(|e| {
        error!(error = %e, "Server error");
        anyhow::anyhow!("Server error: {}", e)
    })?;

    Ok(())
}

/// Run database migrations
///
/// Executes all pending migrations from the migrations directory.
/// This is called automatically on startup to ensure the database schema is up to date.
async fn run_migrations(pool: &sqlx::PgPool) -> anyhow::Result<()> {
    // Use sqlx's built-in migration support
    sqlx::migrate!("./migrations")
        .run(pool)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to run database migrations");
            anyhow::anyhow!("Migration error: {}", e)
        })?;

    Ok(())
}
