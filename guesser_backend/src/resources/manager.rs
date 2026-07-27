use crate::config::GameConfig;
use crate::image::Image;
use crate::location::coordinates::{Coordinate, ItemId, Location};
use crate::resources::cache::ImageCache;
use crate::resources::commons::CommonsClient;
use crate::resources::pool::LocationPool;
use crate::resources::wikidata::WikidataClient;
use std::sync::Arc;
use tokio::time::{Duration, sleep};
use tracing::{debug, info, warn};

pub struct ResourceManager {
    wikidata: WikidataClient,
    commons: CommonsClient,
    pub pool: LocationPool,
    cache: ImageCache,
}

impl ResourceManager {
    pub fn new(config: &GameConfig) -> Self {
        Self {
            wikidata: WikidataClient::new(),
            commons: CommonsClient::new(),
            pool: LocationPool::new(),
            cache: ImageCache::new(config.image_cache_ttl_secs, 7200),
        }
    }

    /// Pop a pre-fetched location from the pool.
    pub fn next_location(&self) -> Option<Location> {
        self.pool.pop()
    }

    /// Get images for a location (from cache, or fetch from Commons if miss).
    pub async fn images_for(&self, item_id: &ItemId, coord: Coordinate) -> Vec<Image> {
        if let Some(cached) = self.cache.get(item_id) {
            debug!(item_id = %item_id.0, "image cache hit");
            return cached;
        }

        debug!(item_id = %item_id.0, "image cache miss, fetching from Commons");
        match self.commons.fetch_images(coord, 10000, 20).await {
            Ok(images) => {
                self.cache.insert(item_id, images.clone());
                images
            }
            Err(e) => {
                warn!(item_id = %item_id.0, error = %e, "failed to fetch images from Commons");
                Vec::new()
            }
        }
    }

    /// Spawns a Tokio background worker task that keeps the location pool refilled.
    pub fn start_refill_worker(self: Arc<Self>, config: GameConfig) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            info!("started location pool refill worker");
            let mut backoff = Duration::from_secs(1);

            loop {
                let current_len = self.pool.len();
                if current_len < config.pool_refill_threshold {
                    debug!(
                        current_len,
                        threshold = config.pool_refill_threshold,
                        "refill threshold triggered, fetching batch from Wikidata"
                    );

                    match self.wikidata.fetch_batch(config.pool_batch_size).await {
                        Ok(batch) => {
                            let count = batch.len();
                            self.pool.push_batch(batch);
                            info!(count, new_total = self.pool.len(), "refilled location pool");
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
