//! Qdrant vector database repository
//!
//! Manages vector collections per embedding provider and provides
//! vector storage and retrieval operations.

use qdrant_client::qdrant::{
    vectors_config::Config, Condition, CreateCollectionBuilder, DeletePointsBuilder, Distance,
    Filter, PointId, PointStruct, SearchPointsBuilder, UpsertPointsBuilder, VectorParamsBuilder,
};
use qdrant_client::Qdrant;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::error::{AppError, AppResult};

/// Collection naming prefix
const COLLECTION_PREFIX: &str = "memories_";

/// Payload field names
const FIELD_MEMORY_ID: &str = "memory_id";
const FIELD_PROFILE_ID: &str = "profile_id";
const FIELD_OWNER_ID: &str = "owner_id";
const FIELD_SCOPE_ID: &str = "scope_id";
const FIELD_CATEGORY: &str = "category";
const FIELD_IS_GLOBAL: &str = "is_global";
const FIELD_STATUS: &str = "status";

/// Vector search result
#[derive(Debug, Clone)]
pub struct VectorSearchResult {
    /// Memory ID
    pub memory_id: Uuid,
    /// Similarity score (0.0 - 1.0)
    pub score: f32,
}

/// Qdrant repository for vector operations
#[derive(Clone)]
pub struct QdrantRepository {
    /// Qdrant client
    client: Arc<Qdrant>,
    /// Cache of collection names and their dimensions
    collections: Arc<RwLock<HashMap<String, usize>>>,
}

impl QdrantRepository {
    /// Create a new QdrantRepository
    pub async fn new(url: &str) -> AppResult<Self> {
        let client = Qdrant::from_url(url)
            .build()
            .map_err(|e| AppError::VectorDb(format!("Failed to create Qdrant client: {}", e)))?;

        let repo = Self {
            client: Arc::new(client),
            collections: Arc::new(RwLock::new(HashMap::new())),
        };

        // Load existing collections
        repo.load_existing_collections().await?;

        Ok(repo)
    }

    /// Load existing collections from Qdrant
    async fn load_existing_collections(&self) -> AppResult<()> {
        let collections = self
            .client
            .list_collections()
            .await
            .map_err(|e| AppError::VectorDb(format!("Failed to list collections: {}", e)))?;

        let mut cache = self.collections.write().await;

        for collection in collections.collections {
            if collection.name.starts_with(COLLECTION_PREFIX) {
                // Get collection info to retrieve dimension
                if let Ok(info) = self.client.collection_info(&collection.name).await {
                    if let Some(config) = info.result.and_then(|r| r.config) {
                        if let Some(params) = config.params {
                            if let Some(vectors_config) = params.vectors_config {
                                if let Some(Config::Params(vector_params)) = vectors_config.config {
                                    cache.insert(
                                        collection.name.clone(),
                                        vector_params.size as usize,
                                    );
                                    debug!(
                                        collection = %collection.name,
                                        dimension = vector_params.size,
                                        "Loaded existing collection"
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }

        info!(count = cache.len(), "Loaded existing Qdrant collections");

        Ok(())
    }

    /// Get collection name for a provider
    pub fn collection_name(provider_name: &str) -> String {
        format!("{}{}", COLLECTION_PREFIX, provider_name.replace('-', "_"))
    }

    /// Create a collection for an embedding provider
    pub async fn create_collection(&self, provider_name: &str, dimension: usize) -> AppResult<()> {
        let collection_name = Self::collection_name(provider_name);

        // Check if collection already exists
        {
            let cache = self.collections.read().await;
            if cache.contains_key(&collection_name) {
                debug!(
                    collection = %collection_name,
                    "Collection already exists"
                );
                return Ok(());
            }
        }

        // Create collection using builder pattern
        self.client
            .create_collection(
                CreateCollectionBuilder::new(&collection_name)
                    .vectors_config(VectorParamsBuilder::new(dimension as u64, Distance::Cosine)),
            )
            .await
            .map_err(|e| {
                // Check if it's a "collection already exists" error
                if e.to_string().contains("already exists") {
                    warn!(
                        collection = %collection_name,
                        "Collection already exists (race condition)"
                    );
                    return AppError::VectorDb(format!(
                        "Collection {} already exists",
                        collection_name
                    ));
                }
                AppError::VectorDb(format!("Failed to create collection: {}", e))
            })?;

        // Update cache
        {
            let mut cache = self.collections.write().await;
            cache.insert(collection_name.clone(), dimension);
        }

        info!(
            collection = %collection_name,
            dimension = dimension,
            "Created Qdrant collection"
        );

        Ok(())
    }

    /// Delete a collection for an embedding provider
    pub async fn delete_collection(&self, provider_name: &str) -> AppResult<()> {
        let collection_name = Self::collection_name(provider_name);

        self.client
            .delete_collection(&collection_name)
            .await
            .map_err(|e| AppError::VectorDb(format!("Failed to delete collection: {}", e)))?;

        // Update cache
        {
            let mut cache = self.collections.write().await;
            cache.remove(&collection_name);
        }

        info!(collection = %collection_name, "Deleted Qdrant collection");

        Ok(())
    }

    /// Check if a collection exists
    pub async fn collection_exists(&self, provider_name: &str) -> bool {
        let collection_name = Self::collection_name(provider_name);
        let cache = self.collections.read().await;
        cache.contains_key(&collection_name)
    }

    /// Get collection dimension
    pub async fn get_collection_dimension(&self, provider_name: &str) -> Option<usize> {
        let collection_name = Self::collection_name(provider_name);
        let cache = self.collections.read().await;
        cache.get(&collection_name).copied()
    }

    /// List all collections with their dimensions
    pub async fn list_collections(&self) -> HashMap<String, usize> {
        let cache = self.collections.read().await;
        cache.clone()
    }

    /// Upsert a vector for a memory
    pub async fn upsert_vector(
        &self,
        provider_name: &str,
        memory_id: Uuid,
        embedding: Vec<f32>,
        payload: VectorPayload,
    ) -> AppResult<()> {
        let collection_name = Self::collection_name(provider_name);

        // Verify collection exists
        {
            let cache = self.collections.read().await;
            if !cache.contains_key(&collection_name) {
                return Err(AppError::VectorDb(format!(
                    "Collection {} does not exist",
                    collection_name
                )));
            }
        }

        let point = PointStruct::new(
            memory_id.to_string(),
            embedding,
            payload.to_qdrant_payload(),
        );

        self.client
            .upsert_points(UpsertPointsBuilder::new(&collection_name, vec![point]))
            .await
            .map_err(|e| AppError::VectorDb(format!("Failed to upsert vector: {}", e)))?;

        debug!(
            collection = %collection_name,
            memory_id = %memory_id,
            "Upserted vector"
        );

        Ok(())
    }

    /// Delete a vector for a memory
    pub async fn delete_vector(&self, provider_name: &str, memory_id: Uuid) -> AppResult<()> {
        let collection_name = Self::collection_name(provider_name);

        let point_id: PointId = memory_id.to_string().into();
        self.client
            .delete_points(DeletePointsBuilder::new(&collection_name).points(vec![point_id]))
            .await
            .map_err(|e| AppError::VectorDb(format!("Failed to delete vector: {}", e)))?;

        debug!(
            collection = %collection_name,
            memory_id = %memory_id,
            "Deleted vector"
        );

        Ok(())
    }

    /// Search for similar vectors in a single collection
    pub async fn search(
        &self,
        provider_name: &str,
        query_vector: Vec<f32>,
        limit: usize,
        filter: Option<VectorFilter>,
    ) -> AppResult<Vec<VectorSearchResult>> {
        let collection_name = Self::collection_name(provider_name);

        let mut search_builder =
            SearchPointsBuilder::new(&collection_name, query_vector, limit as u64)
                .with_payload(true);

        // Apply filter if provided
        if let Some(f) = filter {
            search_builder = search_builder.filter(f.to_qdrant_filter());
        }

        let results = self
            .client
            .search_points(search_builder)
            .await
            .map_err(|e| AppError::VectorDb(format!("Failed to search vectors: {}", e)))?;

        let search_results: Vec<VectorSearchResult> = results
            .result
            .into_iter()
            .filter_map(|point| {
                let memory_id = point
                    .payload
                    .get(FIELD_MEMORY_ID)
                    .and_then(|v| v.as_str())
                    .and_then(|s| Uuid::parse_str(s).ok())?;

                Some(VectorSearchResult {
                    memory_id,
                    score: point.score,
                })
            })
            .collect();

        debug!(
            collection = %collection_name,
            results = search_results.len(),
            "Vector search completed"
        );

        Ok(search_results)
    }

    /// Search across multiple collections and merge results
    pub async fn search_multi(
        &self,
        searches: Vec<MultiCollectionSearch>,
        limit: usize,
    ) -> AppResult<Vec<VectorSearchResult>> {
        let mut all_results: Vec<VectorSearchResult> = Vec::new();

        for search in searches {
            match self
                .search(
                    &search.provider_name,
                    search.query_vector,
                    limit,
                    search.filter,
                )
                .await
            {
                Ok(results) => {
                    all_results.extend(results);
                }
                Err(e) => {
                    warn!(
                        provider = %search.provider_name,
                        error = %e,
                        "Failed to search collection, skipping"
                    );
                }
            }
        }

        // Sort by score descending and deduplicate by memory_id
        all_results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Deduplicate - keep highest score for each memory_id
        let mut seen: HashMap<Uuid, f32> = HashMap::new();
        let mut deduped: Vec<VectorSearchResult> = Vec::new();

        for result in all_results {
            if let Some(&existing_score) = seen.get(&result.memory_id) {
                if result.score > existing_score {
                    // Update with higher score
                    seen.insert(result.memory_id, result.score);
                    // Remove old entry and add new one
                    deduped.retain(|r| r.memory_id != result.memory_id);
                    deduped.push(result);
                }
            } else {
                seen.insert(result.memory_id, result.score);
                deduped.push(result);
            }
        }

        // Re-sort after deduplication
        deduped.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Limit results
        deduped.truncate(limit);

        Ok(deduped)
    }

    /// Get health status
    pub async fn health_check(&self) -> AppResult<bool> {
        self.client
            .health_check()
            .await
            .map_err(|e| AppError::VectorDb(format!("Health check failed: {}", e)))?;
        Ok(true)
    }
}

/// Payload stored with each vector
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorPayload {
    pub profile_id: Uuid,
    pub owner_id: String,
    pub memory_id: Uuid,
    pub scope_id: Option<String>,
    pub category: Option<String>,
    pub is_global: bool,
    pub status: String,
}

impl VectorPayload {
    /// Convert to Qdrant payload format (HashMap<String, Value>)
    fn to_qdrant_payload(&self) -> HashMap<String, qdrant_client::qdrant::Value> {
        use qdrant_client::qdrant::{value::Kind, Value};

        let mut payload: HashMap<String, Value> = HashMap::new();
        payload.insert(
            FIELD_PROFILE_ID.to_string(),
            Value {
                kind: Some(Kind::StringValue(self.profile_id.to_string())),
            },
        );
        payload.insert(
            FIELD_OWNER_ID.to_string(),
            Value {
                kind: Some(Kind::StringValue(self.owner_id.clone())),
            },
        );
        payload.insert(
            FIELD_MEMORY_ID.to_string(),
            Value {
                kind: Some(Kind::StringValue(self.memory_id.to_string())),
            },
        );
        if let Some(ref scope_id) = self.scope_id {
            payload.insert(
                FIELD_SCOPE_ID.to_string(),
                Value {
                    kind: Some(Kind::StringValue(scope_id.clone())),
                },
            );
        }
        if let Some(ref category) = self.category {
            payload.insert(
                FIELD_CATEGORY.to_string(),
                Value {
                    kind: Some(Kind::StringValue(category.clone())),
                },
            );
        }
        payload.insert(
            FIELD_IS_GLOBAL.to_string(),
            Value {
                kind: Some(Kind::BoolValue(self.is_global)),
            },
        );
        payload.insert(
            FIELD_STATUS.to_string(),
            Value {
                kind: Some(Kind::StringValue(self.status.clone())),
            },
        );

        payload
    }
}

/// Filter for vector search
#[derive(Debug, Clone, Default)]
pub struct VectorFilter {
    pub profile_id: Option<Uuid>,
    pub owner_id: Option<String>,
    pub scope_id: Option<String>,
    pub category_prefix: Option<String>,
    pub is_global: Option<bool>,
    pub include_global: Option<bool>,
    pub statuses: Option<Vec<String>>,
}

impl VectorFilter {
    /// Convert to Qdrant filter format
    fn to_qdrant_filter(&self) -> Filter {
        let mut must: Vec<Condition> = Vec::new();

        if let Some(profile_id) = self.profile_id {
            must.push(Condition::matches(FIELD_PROFILE_ID, profile_id.to_string()));
        }

        if let Some(ref owner_id) = self.owner_id {
            must.push(Condition::matches(FIELD_OWNER_ID, owner_id.clone()));
        }

        if let Some(ref scope_id) = self.scope_id {
            if self.include_global.unwrap_or(false) {
                let scope_match =
                    Filter::must(vec![Condition::matches(FIELD_SCOPE_ID, scope_id.clone())]);
                let global_match = Filter::must(vec![Condition::matches(FIELD_IS_GLOBAL, true)]);
                must.push(Filter::should(vec![scope_match.into(), global_match.into()]).into());
            } else {
                must.push(Condition::matches(FIELD_SCOPE_ID, scope_id.clone()));
            }
        } else if self.include_global.unwrap_or(false) {
            must.push(Condition::matches(FIELD_IS_GLOBAL, true));
        }

        if let Some(ref category_prefix) = self.category_prefix {
            // Use prefix match for category
            must.push(Condition::matches(
                FIELD_CATEGORY,
                format!("{}*", category_prefix),
            ));
        }

        if let Some(is_global) = self.is_global {
            must.push(Condition::matches(FIELD_IS_GLOBAL, is_global));
        }

        // Status filter - should match any of the provided statuses (OR condition)
        if let Some(ref statuses) = self.statuses {
            if !statuses.is_empty() {
                // Create a nested filter with should (OR) conditions
                let status_conditions: Vec<Condition> = statuses
                    .iter()
                    .map(|s| Condition::matches(FIELD_STATUS, s.clone()))
                    .collect();

                // Wrap in a filter condition for OR logic
                must.push(Filter::should(status_conditions).into());
            }
        }

        Filter::must(must)
    }
}

/// Search request for multiple collections
#[derive(Debug, Clone)]
pub struct MultiCollectionSearch {
    pub provider_name: String,
    pub query_vector: Vec<f32>,
    pub filter: Option<VectorFilter>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_collection_name() {
        assert_eq!(
            QdrantRepository::collection_name("openai"),
            "memories_openai"
        );
        assert_eq!(
            QdrantRepository::collection_name("local-bge"),
            "memories_local_bge"
        );
        assert_eq!(
            QdrantRepository::collection_name("text-embedding-3-small"),
            "memories_text_embedding_3_small"
        );
    }

    #[test]
    fn test_vector_payload_creation() {
        let payload = VectorPayload {
            profile_id: Uuid::new_v4(),
            owner_id: "owner123".to_string(),
            memory_id: Uuid::new_v4(),
            scope_id: Some("scope123".to_string()),
            category: Some("work.code".to_string()),
            is_global: false,
            status: "active".to_string(),
        };

        // Just verify it doesn't panic
        let _ = payload.to_qdrant_payload();
    }

    #[test]
    fn test_vector_filter_empty() {
        let filter = VectorFilter::default();
        let qdrant_filter = filter.to_qdrant_filter();
        assert!(qdrant_filter.must.is_empty());
    }

    #[test]
    fn test_vector_filter_with_scope() {
        let filter = VectorFilter {
            scope_id: Some("scope123".to_string()),
            category_prefix: Some("work".to_string()),
            ..Default::default()
        };
        let qdrant_filter = filter.to_qdrant_filter();
        assert_eq!(qdrant_filter.must.len(), 2);
    }
}
