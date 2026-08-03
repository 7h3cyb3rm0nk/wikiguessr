use std::env;

/// Top-level application configuration.
#[derive(Debug, Clone, Default)]
pub struct Config {
    pub game: GameConfig,
    pub server: ServerConfig,
}

/// Tunable game parameters.
#[derive(Debug, Clone)]
pub struct GameConfig {
    /// Number of rounds per game.
    pub total_rounds: u8,
    /// Duration of each round in seconds.
    pub round_duration_secs: u64,
    /// Maximum points awarded for a perfect guess.
    pub max_score: u32,
    /// Exponential decay constant (meters). Defaults to 2000 km, matching the
    /// original game.js scoring curve (~3900 pts at 100 km, ~2100 at 500 km).
    pub scoring_decay_m: f64,
    /// Target number of pre-fetched locations in the pool.
    pub pool_target_size: usize,
    /// Trigger a refill when pool drops below this threshold.
    pub pool_refill_threshold: usize,
    /// Number of locations to fetch per Wikidata batch.
    pub pool_batch_size: usize,
    /// Number of images shown per round (a fixed 7-column grid).
    pub images_per_round: usize,
    /// Base TTL for cached image sets (seconds). Jitter is added on top.
    pub image_cache_ttl_secs: u64,
    /// Destroy a room after this many seconds of inactivity.
    pub inactivity_timeout_secs: u64,
    /// Destroy a lobby if no player readies within this many seconds.
    pub lobby_timeout_secs: u64,
}

/// Server network configuration.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

// ── Defaults ──────────────────────────────────────────────────────────────

impl Default for GameConfig {
    fn default() -> Self {
        Self {
            total_rounds: 5,
            round_duration_secs: 60,
            max_score: 5000,
            scoring_decay_m: 2_000_000.0,
            pool_target_size: 1000,
            pool_refill_threshold: 300,
            pool_batch_size: 200,
            images_per_round: 21,
            image_cache_ttl_secs: 24 * 60 * 60, // 24 hours
            inactivity_timeout_secs: 300,       // 5 minutes
            lobby_timeout_secs: 120,            // 2 minutes
        }
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".to_string(),
            port: 3000,
        }
    }
}

// ── Environment loading ───────────────────────────────────────────────────

/// Parse an env var or fall back to a default value.
fn env_or<T: std::str::FromStr>(key: &str, default: T) -> T {
    env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

impl Config {
    /// Load config with defaults, overridden by environment variables.
    ///
    /// All env vars are prefixed with `WIKIGUESSR_`.
    pub fn from_env() -> Self {
        let d = Self::default();
        Self {
            game: GameConfig {
                total_rounds: env_or("WIKIGUESSR_TOTAL_ROUNDS", d.game.total_rounds),
                round_duration_secs: env_or(
                    "WIKIGUESSR_ROUND_DURATION_SECS",
                    d.game.round_duration_secs,
                ),
                max_score: env_or("WIKIGUESSR_MAX_SCORE", d.game.max_score),
                scoring_decay_m: env_or("WIKIGUESSR_SCORING_DECAY_M", d.game.scoring_decay_m),
                pool_target_size: env_or("WIKIGUESSR_POOL_TARGET_SIZE", d.game.pool_target_size),
                pool_refill_threshold: env_or(
                    "WIKIGUESSR_POOL_REFILL_THRESHOLD",
                    d.game.pool_refill_threshold,
                ),
                pool_batch_size: env_or("WIKIGUESSR_POOL_BATCH_SIZE", d.game.pool_batch_size),
                images_per_round: env_or("WIKIGUESSR_IMAGES_PER_ROUND", d.game.images_per_round),
                image_cache_ttl_secs: env_or(
                    "WIKIGUESSR_IMAGE_CACHE_TTL_SECS",
                    d.game.image_cache_ttl_secs,
                ),
                inactivity_timeout_secs: env_or(
                    "WIKIGUESSR_INACTIVITY_TIMEOUT_SECS",
                    d.game.inactivity_timeout_secs,
                ),
                lobby_timeout_secs: env_or(
                    "WIKIGUESSR_LOBBY_TIMEOUT_SECS",
                    d.game.lobby_timeout_secs,
                ),
            },
            server: ServerConfig {
                host: env_or("WIKIGUESSR_HOST", d.server.host),
                port: env_or("WIKIGUESSR_PORT", d.server.port),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sane() {
        let c = Config::default();
        assert_eq!(c.game.total_rounds, 5);
        assert_eq!(c.game.max_score, 5000);
        assert_eq!(c.server.port, 3000);
        assert!(c.game.scoring_decay_m > 0.0);
    }

    #[test]
    fn env_override_works() {
        // Set a test env var, call from_env, verify override.
        unsafe { env::set_var("WIKIGUESSR_PORT", "8080") };
        let c = Config::from_env();
        assert_eq!(c.server.port, 8080);
        unsafe { env::remove_var("WIKIGUESSR_PORT") };
    }
}
