//! Decay Calculator Service
//!
//! Calculates decay scores for memories based on hit count and time since last activity.
//! Used for LFU (Least Frequently Used) eviction decisions.
//!
//! Decay Score Formula:
//! decay_score = (1 + ln(hit_count + 1) * hit_boost_factor)
//!             * exp(-days_since_last_activity / decay_half_life_days)
//!             * (is_global ? global_boost : 1.0)
//!
//! Requirements: 6.1, 6.2

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::{debug, info, instrument};

use crate::domain::Memory;
use crate::error::AppResult;
use crate::repository::MemoryRepository;

/// Configuration for decay score calculation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecayConfig {
    /// Half-life in days for decay (default: 7.0)
    /// After this many days without activity, the time component is halved
    pub decay_half_life_days: f32,
    /// Boost factor for hit count (default: 0.1)
    /// Higher values give more weight to frequently accessed memories
    pub hit_boost_factor: f32,
    /// Boost multiplier for global memories (default: 2.0)
    /// Global memories are considered more valuable
    pub global_boost: f32,
}

impl Default for DecayConfig {
    fn default() -> Self {
        Self {
            decay_half_life_days: 7.0,
            hit_boost_factor: 0.1,
            global_boost: 2.0,
        }
    }
}

/// Decay Calculator service for computing memory decay scores
#[derive(Clone)]
pub struct DecayCalculator {
    memory_repo: MemoryRepository,
    config: DecayConfig,
}

impl DecayCalculator {
    /// Create a new DecayCalculator
    pub fn new(memory_repo: MemoryRepository, config: DecayConfig) -> Self {
        Self {
            memory_repo,
            config,
        }
    }

    /// Create with default configuration
    pub fn with_defaults(memory_repo: MemoryRepository) -> Self {
        Self::new(memory_repo, DecayConfig::default())
    }

    /// Calculate decay score for a single memory
    ///
    /// Formula:
    /// decay_score = (1 + ln(hit_count + 1) * hit_boost_factor)
    ///             * exp(-days_since_last_activity / decay_half_life_days)
    ///             * (is_global ? global_boost : 1.0)
    pub fn calculate(&self, memory: &Memory) -> f32 {
        self.calculate_with_config(memory, &self.config)
    }

    /// Calculate decay score with custom configuration
    pub fn calculate_with_config(&self, memory: &Memory, config: &DecayConfig) -> f32 {
        calculate_decay_score(
            memory.hit_count,
            memory.last_hit_at,
            memory.created_at,
            memory.is_global,
            config,
        )
    }

    /// Calculate decay score at a specific point in time
    /// Useful for testing and simulations
    pub fn calculate_at_time(&self, memory: &Memory, now: DateTime<Utc>) -> f32 {
        calculate_decay_score_at_time(
            memory.hit_count,
            memory.last_hit_at,
            memory.created_at,
            memory.is_global,
            &self.config,
            now,
        )
    }

    /// Batch update decay scores for all memories of an owner
    ///
    /// This updates the decay_score field in the database for all
    /// current version, non-archived memories.
    #[instrument(skip(self), fields(owner_id = %owner_id))]
    pub async fn update_batch(&self, owner_id: &str) -> AppResult<usize> {
        let updated = self
            .memory_repo
            .update_decay_scores(
                owner_id,
                self.config.decay_half_life_days,
                self.config.hit_boost_factor,
                self.config.global_boost,
            )
            .await?;

        info!(
            owner_id = %owner_id,
            updated_count = updated,
            "Updated decay scores"
        );

        Ok(updated)
    }

    /// Get the current configuration
    pub fn config(&self) -> &DecayConfig {
        &self.config
    }

    /// Update the configuration
    pub fn set_config(&mut self, config: DecayConfig) {
        self.config = config;
    }
}

/// Pure function to calculate decay score
///
/// This is exposed for testing and can be used without a repository.
pub fn calculate_decay_score(
    hit_count: i64,
    last_hit_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    is_global: bool,
    config: &DecayConfig,
) -> f32 {
    calculate_decay_score_at_time(
        hit_count,
        last_hit_at,
        created_at,
        is_global,
        config,
        Utc::now(),
    )
}

/// Pure function to calculate decay score at a specific time
pub fn calculate_decay_score_at_time(
    hit_count: i64,
    last_hit_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    is_global: bool,
    config: &DecayConfig,
    now: DateTime<Utc>,
) -> f32 {
    // Calculate days since last activity
    let last_activity = last_hit_at.unwrap_or(created_at);
    let duration = now.signed_duration_since(last_activity);
    let days_since_last_activity = duration.num_seconds() as f32 / 86400.0;

    // Ensure non-negative days (in case of clock skew)
    let days_since_last_activity = days_since_last_activity.max(0.0);

    // Hit count component: 1 + ln(hit_count + 1) * hit_boost_factor
    let hit_component = 1.0 + ((hit_count + 1) as f32).ln() * config.hit_boost_factor;

    // Time decay component: exp(-days / half_life)
    // Using natural log for half-life: exp(-days * ln(2) / half_life)
    let time_component = (-days_since_last_activity * 0.693147 / config.decay_half_life_days).exp();

    // Global boost component
    let global_component = if is_global { config.global_boost } else { 1.0 };

    let score = hit_component * time_component * global_component;

    debug!(
        hit_count = hit_count,
        days_since = days_since_last_activity,
        is_global = is_global,
        hit_component = hit_component,
        time_component = time_component,
        global_component = global_component,
        score = score,
        "Calculated decay score"
    );

    score
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn default_config() -> DecayConfig {
        DecayConfig::default()
    }

    #[test]
    fn test_decay_config_default() {
        let config = DecayConfig::default();
        assert!((config.decay_half_life_days - 7.0).abs() < f32::EPSILON);
        assert!((config.hit_boost_factor - 0.1).abs() < f32::EPSILON);
        assert!((config.global_boost - 2.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_new_memory_has_high_score() {
        let config = default_config();
        let now = Utc::now();

        // New memory with no hits
        let score = calculate_decay_score_at_time(0, None, now, false, &config, now);

        // Should be close to 1.0 (hit_component = 1 + ln(1)*0.1 = 1.0, time_component = 1.0)
        assert!(score > 0.9, "New memory should have high score: {}", score);
        assert!(
            score < 1.1,
            "New memory score should be around 1.0: {}",
            score
        );
    }

    #[test]
    fn test_score_increases_with_hits() {
        let config = default_config();
        let now = Utc::now();

        let score_0_hits = calculate_decay_score_at_time(0, Some(now), now, false, &config, now);
        let score_10_hits = calculate_decay_score_at_time(10, Some(now), now, false, &config, now);
        let score_100_hits =
            calculate_decay_score_at_time(100, Some(now), now, false, &config, now);

        assert!(
            score_10_hits > score_0_hits,
            "More hits should increase score: {} > {}",
            score_10_hits,
            score_0_hits
        );
        assert!(
            score_100_hits > score_10_hits,
            "More hits should increase score: {} > {}",
            score_100_hits,
            score_10_hits
        );
    }

    #[test]
    fn test_score_decreases_with_time() {
        let config = default_config();
        let now = Utc::now();
        let created = now - Duration::days(30);

        let score_recent =
            calculate_decay_score_at_time(5, Some(now), created, false, &config, now);
        let score_week_ago = calculate_decay_score_at_time(
            5,
            Some(now - Duration::days(7)),
            created,
            false,
            &config,
            now,
        );
        let score_month_ago = calculate_decay_score_at_time(
            5,
            Some(now - Duration::days(30)),
            created,
            false,
            &config,
            now,
        );

        assert!(
            score_recent > score_week_ago,
            "Recent activity should have higher score: {} > {}",
            score_recent,
            score_week_ago
        );
        assert!(
            score_week_ago > score_month_ago,
            "Week ago should have higher score than month ago: {} > {}",
            score_week_ago,
            score_month_ago
        );
    }

    #[test]
    fn test_half_life_decay() {
        let config = DecayConfig {
            decay_half_life_days: 7.0,
            hit_boost_factor: 0.0, // Disable hit boost for this test
            global_boost: 1.0,
        };
        let now = Utc::now();
        let created = now;

        // Score at creation
        let score_now = calculate_decay_score_at_time(0, Some(now), created, false, &config, now);

        // Score after one half-life (7 days)
        let score_half_life = calculate_decay_score_at_time(
            0,
            Some(now),
            created,
            false,
            &config,
            now + Duration::days(7),
        );

        // After one half-life, score should be approximately half
        let ratio = score_half_life / score_now;
        assert!(
            (ratio - 0.5).abs() < 0.01,
            "Score should halve after half-life: ratio = {}",
            ratio
        );
    }

    #[test]
    fn test_global_boost() {
        let config = default_config();
        let now = Utc::now();

        let score_local = calculate_decay_score_at_time(5, Some(now), now, false, &config, now);
        let score_global = calculate_decay_score_at_time(5, Some(now), now, true, &config, now);

        let ratio = score_global / score_local;
        assert!(
            (ratio - config.global_boost).abs() < 0.01,
            "Global score should be {} times local: ratio = {}",
            config.global_boost,
            ratio
        );
    }

    #[test]
    fn test_score_never_negative() {
        let config = default_config();
        let now = Utc::now();
        let very_old = now - Duration::days(365);

        let score = calculate_decay_score_at_time(0, Some(very_old), very_old, false, &config, now);

        assert!(score >= 0.0, "Score should never be negative: {}", score);
    }

    #[test]
    fn test_hit_boost_factor_effect() {
        let now = Utc::now();

        let config_low_boost = DecayConfig {
            hit_boost_factor: 0.05,
            ..default_config()
        };
        let config_high_boost = DecayConfig {
            hit_boost_factor: 0.2,
            ..default_config()
        };

        let score_low =
            calculate_decay_score_at_time(100, Some(now), now, false, &config_low_boost, now);
        let score_high =
            calculate_decay_score_at_time(100, Some(now), now, false, &config_high_boost, now);

        assert!(
            score_high > score_low,
            "Higher hit boost should increase score: {} > {}",
            score_high,
            score_low
        );
    }

    #[test]
    fn test_monotonicity_with_hits() {
        // Property 9: Decay Score Monotonicity
        // For any Memory M, if hit_count increases, decay_score SHALL increase (all else equal)
        let config = default_config();
        let now = Utc::now();

        for hit_count in 0..100 {
            let score_current =
                calculate_decay_score_at_time(hit_count, Some(now), now, false, &config, now);
            let score_next =
                calculate_decay_score_at_time(hit_count + 1, Some(now), now, false, &config, now);

            assert!(
                score_next > score_current,
                "Score should increase with hits: {} (hits={}) < {} (hits={})",
                score_current,
                hit_count,
                score_next,
                hit_count + 1
            );
        }
    }
}
