use crate::image::Image;
use crate::location::coordinates::ItemId;
use dashmap::DashMap;
use rand::RngExt;
use std::time::{Duration, Instant};

/// A cached set of images with an expiration time.
struct CachedEntry {
    images: Vec<Image>,
    expires_at: Instant,
}

/// Thread-safe image cache with jittered TTL.
///
/// Each entry expires at `base_ttl ± jitter` to avoid thundering-herd
/// correlated expiry when many entries are populated around the same time.
pub struct ImageCache {
    entries: DashMap<String, CachedEntry>,
    base_ttl: Duration,
    jitter: Duration,
}

impl ImageCache {
    /// Create a new cache.
    ///
    /// - `base_ttl_secs`: Base time-to-live in seconds (e.g. 86400 for 24h).
    /// - `jitter_secs`: Maximum jitter in seconds (e.g. 7200 for ±2h).
    pub fn new(base_ttl_secs: u64, jitter_secs: u64) -> Self {
        Self {
            entries: DashMap::new(),
            base_ttl: Duration::from_secs(base_ttl_secs),
            jitter: Duration::from_secs(jitter_secs),
        }
    }

    /// Look up cached images for a given item.
    /// Returns `None` on miss or expiry (expired entries are evicted).
    pub fn get(&self, item_id: &ItemId) -> Option<Vec<Image>> {
        let key = &item_id.0;

        // Check if entry exists and is still valid.
        let entry = self.entries.get(key)?;
        if Instant::now() >= entry.expires_at {
            // Expired — drop the ref before removing.
            drop(entry);
            self.entries.remove(key);
            return None;
        }

        Some(entry.images.clone())
    }

    /// Insert images into the cache with a jittered TTL.
    pub fn insert(&self, item_id: &ItemId, images: Vec<Image>) {
        let jitter_offset = if self.jitter.is_zero() {
            0
        } else {
            rand::rng().random_range(0..self.jitter.as_secs() * 2)
        };

        let ttl = self.base_ttl + Duration::from_secs(jitter_offset) - self.jitter;
        let expires_at = Instant::now() + ttl;

        self.entries.insert(
            item_id.0.clone(),
            CachedEntry { images, expires_at },
        );
    }

    /// Number of entries currently in the cache (including potentially expired).
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_image() -> Image {
        Image {
            url: "https://example.com/img.jpg".to_string(),
            thumb_url: "https://example.com/img_thumb.jpg".to_string(),
            width: 800,
            height: 600,
        }
    }

    #[test]
    fn cache_miss_returns_none() {
        let cache = ImageCache::new(3600, 0);
        assert!(cache.get(&ItemId("Q42".to_string())).is_none());
    }

    #[test]
    fn cache_hit_returns_images() {
        let cache = ImageCache::new(3600, 0);
        let id = ItemId("Q42".to_string());
        cache.insert(&id, vec![test_image()]);

        let result = cache.get(&id).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].url, "https://example.com/img.jpg");
    }

    #[test]
    fn expired_entry_returns_none() {
        // TTL = 0 seconds, jitter = 0 → expires immediately.
        let cache = ImageCache::new(0, 0);
        let id = ItemId("Q42".to_string());
        cache.insert(&id, vec![test_image()]);

        // Should be expired by now.
        assert!(cache.get(&id).is_none());
    }

    #[test]
    fn empty_image_set_is_cacheable() {
        // A location with no images is a valid cache entry (not a miss).
        let cache = ImageCache::new(3600, 0);
        let id = ItemId("Q99".to_string());
        cache.insert(&id, vec![]);

        let result = cache.get(&id).unwrap();
        assert!(result.is_empty());
    }
}
