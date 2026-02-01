//! Memory Server - Application Entry Point
//!
//! This module handles the startup flow:
//! - Configuration loading
//! - Database connection pool initialization
//! - Database migration execution
//! - Qdrant client initialization
//! - Global LLM and Embedding provider initialization
//! - HTTP server startup

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::http::header::HeaderName;
use causality::api::{create_router, health::init_start_time, AppState};
use causality::config::AppConfig;
use causality::embedding::{EmbeddingProvider, LocalProvider, OpenAIProvider, ProviderType};
use causality::llm::{
    LlmProvider, LlmProviderConfig, LlmProviderType, LocalLlmProvider, OpenAILlmProvider,
};
use causality::repository::{
    AuditRepository, EventRepository, MemoryRepository, ProfileRepository, QdrantRepository,
};
use causality::service::{LifecycleManager, MemoryGuard, ProfileService, RetrievalEngine};
use sqlx::postgres::PgPoolOptions;
use tower_http::cors::{Any, CorsLayer};
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;
use tracing::{error, info};
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

    // Initialize global LLM provider from config
    info!(
        provider_type = %config.llm.provider_type,
        model = %config.llm.model,
        "Initializing global LLM provider"
    );

    let llm_provider: Arc<dyn LlmProvider> = create_llm_provider(&config)?;
    info!("Global LLM provider initialized");

    // Initialize global Embedding provider from config
    info!(
        provider_type = %config.embedding.provider_type,
        model = %config.embedding.model,
        dimension = config.embedding.dimension,
        "Initializing global Embedding provider"
    );

    let (embedding_provider, embedding_provider_name) = create_embedding_provider(&config)?;
    info!(
        provider_name = %embedding_provider_name,
        "Global Embedding provider initialized"
    );

    // Ensure Qdrant collection exists for the embedding provider
    if !qdrant_repo
        .collection_exists(&embedding_provider_name)
        .await
    {
        info!(
            collection = %embedding_provider_name,
            dimension = config.embedding.dimension,
            "Creating Qdrant collection"
        );
        qdrant_repo
            .create_collection(&embedding_provider_name, config.embedding.dimension)
            .await
            .map_err(|e| {
                error!(error = %e, "Failed to create Qdrant collection");
                anyhow::anyhow!("Qdrant collection creation error: {}", e)
            })?;
        info!("Qdrant collection created");
    } else {
        info!(
            collection = %embedding_provider_name,
            "Qdrant collection already exists"
        );
    }

    // Create repositories
    let memory_repo = MemoryRepository::new(pool.clone());
    let event_repo = EventRepository::new(pool.clone());
    let audit_repo = AuditRepository::new(pool.clone());
    let profile_repo = ProfileRepository::new(pool.clone());

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

    let lifecycle_manager = LifecycleManager::new(
        memory_repo.clone(),
        audit_repo.clone(),
        config.lifecycle.clone(),
        config.audit.enabled,
    );

    // Create ProfileService with global LLM provider
    let profile_service = ProfileService::new(profile_repo, llm_provider.clone());

    // Create application state
    let app_state = AppState {
        memory_guard: Arc::new(memory_guard),
        retrieval_engine,
        lifecycle_manager,
        profile_service,
        llm_provider,
        embedding_provider,
        embedding_provider_name,
        qdrant_repo,
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

/// Create LLM provider from configuration
fn create_llm_provider(config: &AppConfig) -> anyhow::Result<Arc<dyn LlmProvider>> {
    let llm_config = LlmProviderConfig {
        name: format!("{}-{}", config.llm.provider_type, config.llm.model),
        provider_type: config.llm.provider_type,
        endpoint: config.llm.endpoint.clone(),
        api_key: config.llm.api_key.clone(),
        model: config.llm.model.clone(),
        enabled: true,
    };

    match config.llm.provider_type {
        LlmProviderType::Openai | LlmProviderType::Azure => Ok(Arc::new(
            OpenAILlmProvider::new(llm_config).map_err(|e| {
                error!(error = %e, "Failed to create Openai LLM provider");
                anyhow::anyhow!("LLM provider initialization error: {}", e)
            })?,
        )),
        LlmProviderType::Local => Ok(Arc::new(LocalLlmProvider::new(llm_config).map_err(
            |e| {
                error!(error = %e, "Failed to create local LLM provider");
                anyhow::anyhow!("LLM provider initialization error: {}", e)
            },
        )?)),
    }
}

/// Create Embedding provider from configuration
fn create_embedding_provider(
    config: &AppConfig,
) -> anyhow::Result<(Arc<dyn EmbeddingProvider>, String)> {
    use causality::embedding::ProviderConfig;

    let provider_name = format!(
        "{}-{}",
        config.embedding.provider_type, config.embedding.model
    );

    let embedding_config = ProviderConfig {
        name: provider_name.clone(),
        provider_type: config.embedding.provider_type,
        endpoint: config.embedding.endpoint.clone(),
        api_key: config.embedding.api_key.clone(),
        model: config.embedding.model.clone(),
        dimension: config.embedding.dimension,
        enabled: true,
    };

    let provider: Arc<dyn EmbeddingProvider> = match config.embedding.provider_type {
        ProviderType::Openai | ProviderType::Azure => {
            Arc::new(OpenAIProvider::new(embedding_config).map_err(|e| {
                error!(error = %e, "Failed to create Openai Embedding provider");
                anyhow::anyhow!("Embedding provider initialization error: {}", e)
            })?)
        }
        ProviderType::Local => Arc::new(LocalProvider::new(embedding_config).map_err(|e| {
            error!(error = %e, "Failed to create local Embedding provider");
            anyhow::anyhow!("Embedding provider initialization error: {}", e)
        })?),
    };

    Ok((provider, provider_name))
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
