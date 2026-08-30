use crate::config::GameConfig;
use crate::image::{self, Image};
use crate::location::coordinates::{Coordinate, ItemId, Location};
use crate::resources::cache::ImageCache;
use crate::resources::commons::CommonsClient;
use crate::resources::pool::{LocationPool, PreparedRound};
use crate::resources::wikidata::WikidataClient;
use std::sync::Arc;
use tokio::time::{Duration, sleep};
use tracing::{debug, info, warn};

pub struct ResourceManager {
    wikidata: WikidataClient,
    commons: CommonsClient,
    pub pool: LocationPool,
    cache: ImageCache,
    images_per_round: usize,
}

impl ResourceManager {
    pub fn new(config: &GameConfig) -> Self {
        let pool = LocationPool::new();

        Self {
            wikidata: WikidataClient::new(),
            commons: CommonsClient::new(),
            pool,
            cache: ImageCache::new(config.image_cache_ttl_secs, 7200),
            images_per_round: config.images_per_round,
        }
    }

    /// A single generated fallback image. `seed` varies the artwork so a grid
    /// of fallbacks is never a wall of identical tiles.
    fn fallback_image(coord: Coordinate, seed: usize) -> Image {
        let label = format!("scene-{:.0}-{:.0}", coord.latitude, coord.longitude);
        let hues = ["#38bdf8", "#34d399", "#fbbf24", "#f472b6", "#a78bfa"];
        let color = hues[seed % hues.len()];
        let cx = 100 + (seed * 97) % 600;
        let svg = format!(
            "<svg xmlns='http://www.w3.org/2000/svg' width='800' height='600'><rect width='100%' height='100%' fill='#0f172a'/><rect x='20' y='20' width='760' height='560' rx='24' fill='#1e293b'/><circle cx='{cx}' cy='260' r='120' fill='{color}' opacity='0.4'/><path d='M180 440C260 330, 540 330, 620 440' stroke='#facc15' stroke-width='12' fill='none'/><text x='400' y='290' font-family='Arial, sans-serif' font-size='34' text-anchor='middle' fill='#f8fafc'>{label}</text></svg>",
        );
        // Encode the SVG so it forms a valid data URL (<, >, #, spaces etc. are escaped).
        let encoded = percent_encoding::utf8_percent_encode(&svg, percent_encoding::NON_ALPHANUMERIC);
        let data_url = format!("data:image/svg+xml;charset=UTF-8,{encoded}");
        Image {
            url: data_url.clone(),
            thumb_url: data_url,
            width: 800,
            height: 600,
        }
    }

    /// Normalize a set of images to exactly `count` entries: truncate when there
    /// are too many, append distinct generated fallbacks when there are too few.
    /// Guarantees a stable grid size (e.g. 21 tiles in a 7-column layout).
    fn pad_images(images: Vec<Image>, count: usize, coord: Coordinate) -> Vec<Image> {
        let mut images: Vec<Image> = images.into_iter().take(count).collect();
        while images.len() < count {
            let fallback = Self::fallback_image(coord, images.len());
            images.push(fallback);
        }
        images
    }

    /// Pop a pre-fetched location from the pool.
    pub async fn next_location(&self) -> PreparedRound {
        let mut attempts = 0;
        loop {
            if let Some(pr) = self.pool.pop() {
                return pr;
            }
            if attempts >= 20 {
                warn!("location pool empty after waiting 10s, using emergency fallback round");
                let loc = Location::new(ItemId::new("Q_EMERGENCY".into()), Coordinate::new(0.0, 0.0).unwrap());
                let images = Self::pad_images(Vec::new(), self.images_per_round, loc.coordinate);
                return PreparedRound { location: loc, images };
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            attempts += 1;
        }
    }

    /// Get images for a location (from cache, or fetch from Commons if miss).
    ///
    /// Always returns exactly `images_per_round` images: real Commons photos
    /// when available, padded with generated fallbacks otherwise.
    pub async fn images_for(&self, item_id: &ItemId, coord: Coordinate) -> Vec<Image> {
        if let Some(cached) = self.cache.get(item_id) {
            debug!(item_id = %item_id.0, "image cache hit");
            return cached;
        }

        debug!(item_id = %item_id.0, "image cache miss, fetching from Commons");
        let (images, cacheable) = match self
            .commons
            .fetch_images(coord, 10000, self.images_per_round)
            .await
        {
            Ok(images) if !images.is_empty() => (images, true),
            Ok(_) => {
                warn!(item_id = %item_id.0, "no usable Commons images near coord");
                (Vec::new(), true)
            }
            Err(e) => {
                warn!(item_id = %item_id.0, error = %e, "Commons request failed; using fallback images");
                (Vec::new(), false)
            }
        };

        let padded = Self::pad_images(images, self.images_per_round, coord);
        if cacheable {
            self.cache.insert(item_id, padded.clone());
        }
        padded
    }

    /// Spawns a Tokio background worker task that keeps the location pool refilled.
    pub fn start_refill_worker(self: Arc<Self>, config: GameConfig) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            info!("started location pool refill worker");
            let mut backoff = Duration::from_secs(1);

            loop {
                let current_len = self.pool.len();
                if current_len < config.pool_refill_threshold {
                    warn!(
                        current_len,
                        threshold = config.pool_refill_threshold,
                        "refill threshold triggered, fetching batch from Wikidata"
                    );

                    match self.wikidata.fetch_batch(config.pool_batch_size).await {
                        
                        Ok(batch) => {
                            info!("fetched batch of {} locations from Wikidata", config.pool_batch_size);
                            let mut added = 0;
                            for loc in batch {
                                let images = self.images_for(&loc.item_id, loc.coordinate).await;
                                if images.is_empty() {
                                    warn!(item_id = %loc.item_id.0, "skipping location with no usable images");
                                    continue;
                                }
                                self.pool.push_batch(vec![PreparedRound { location: loc, images }]);
                                info!(added, new_total = self.pool.len(), "added location to pool");

                                added += 1;
                                // Small sleep between commons requests to avoid rate limits
                                sleep(Duration::from_millis(500)).await;
                            }
                            info!(added, new_total = self.pool.len(), "refilled location pool");
                            backoff = Duration::from_secs(1); // Reset backoff on success
                        }
                        Err(e) => {
                            warn!(
                                error = %e,
                                retry_in_secs = backoff.as_secs(),
                                "failed to fetch location batch from Wikidata"
                            );
                            sleep(backoff).await;
                            backoff = (backoff * 2).min(Duration::from_secs(60));
                            continue;
                        }
                    }
                }

                // Check again in 5 seconds
                sleep(Duration::from_secs(5)).await;
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn images_for_returns_fallback_images_when_commons_is_unavailable() {
        let config = GameConfig::default();
        let manager = ResourceManager::new(&config);
        let coord = Coordinate::new(51.5074, -0.1278).unwrap();

        let images = manager.images_for(&ItemId::new("Q42".to_string()), coord).await;

        // Every round must present the full grid, whether from Commons or fallback.
        assert_eq!(images.len(), manager.images_per_round);
    }

    #[test]
    fn pad_images_truncates_when_over_count() {
        let coord = Coordinate::new(10.0, 20.0).unwrap();
        let images = (0..25)
            .map(|_| Image {
                url: "https://example.com/x.jpg".into(),
                thumb_url: "https://example.com/x_thumb.jpg".into(),
                width: 100,
                height: 100,
            })
            .collect::<Vec<_>>();

        let padded = ResourceManager::pad_images(images, 21, coord);
        assert_eq!(padded.len(), 21);
    }

    #[test]
    fn pad_images_appends_distinct_fallbacks_to_reach_count() {
        let coord = Coordinate::new(10.0, 20.0).unwrap();
        let images = Vec::new();

        let padded = ResourceManager::pad_images(images, 21, coord);
        assert_eq!(padded.len(), 21);
        // Fallback tiles must be distinct (not identical copies of one SVG).
        let unique_urls: std::collections::HashSet<_> =
            padded.iter().map(|img| img.url.clone()).collect();
        assert_eq!(unique_urls.len(), 21, "fallbacks should be distinct per tile");
    }

    #[test]
    fn pad_images_keeps_partial_commons_results() {
        let coord = Coordinate::new(10.0, 20.0).unwrap();
        let images = (0..7)
            .map(|_| Image {
                url: "https://example.com/real.jpg".into(),
                thumb_url: "https://example.com/real_thumb.jpg".into(),
                width: 800,
                height: 600,
            })
            .collect::<Vec<_>>();

        let padded = ResourceManager::pad_images(images, 21, coord);
        assert_eq!(padded.len(), 21);
        let fallbacks = padded.iter().filter(|img| img.url.contains("data:image/svg"));
        assert_eq!(fallbacks.count(), 14);
    }
}
