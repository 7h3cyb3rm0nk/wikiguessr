use crate::image::Image;
use crate::location::coordinates::Coordinate;
use reqwest::Client;
use reqwest::StatusCode;
use serde::Deserialize;
use tokio::time::{Duration, sleep};
use tracing::debug;

const REQUEST_TIMEOUT_SECS: u64 = 10;
const MAX_RETRIES: usize = 3;

/// Terms whose presence in metadata suggests geographic/landscape content.
const GEO_TERMS: &[&str] = &[
    "landscape",
    "panorama",
    "aerial",
    "mountain",
    "river",
    "lake",
    "coast",
    "valley",
    "forest",
    "desert",
    "glacier",
    "waterfall",
    "canyon",
    "plain",
    "island",
    "bay",
    "cape",
    "beach",
    "cliff",
    "hill",
    "volcano",
    "geography",
    "natural",
    "scenery",
    "terrain",
    "vegetation",
    "wetland",
    "estuary",
];

/// Terms identifying satellite/orbital imagery — excluded outright.
const SPACE_TERMS: &[&str] = &[
    "satellite image",
    "satellite view",
    "satellite photo",
    "satellite picture",
    "from space",
    "seen from orbit",
    "landsat",
    "sentinel-2",
    "copernicus",
    "earth observatory",
    "international space station",
    "space station",
    "astronaut photograph",
    "spot image",
    "worldview-",
    "weather satellite",
    "earth from orbit",
    "modis",
    "nasa",
];

/// Category keywords that indicate non-geographic content.
const EXCLUDED_CATEGORIES: &[&str] = &["portrait", "logo", "coat of arms", "flag of"];

/// Client for fetching geotagged images from Wikimedia Commons.
pub struct CommonsClient {
    http: Client,
}

// ── Commons API response types ────────────────────────────────────────────

#[derive(Deserialize)]
struct QueryResponse {
    query: Option<QueryData>,
}

#[derive(Deserialize)]
struct QueryData {
    pages: std::collections::HashMap<String, Page>,
}

#[derive(Deserialize)]
struct Page {
    title: Option<String>,
    imageinfo: Option<Vec<ImageInfo>>,
}

#[derive(Deserialize)]
struct ImageInfo {
    url: Option<String>,
    thumburl: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    mediatype: Option<String>,
    extmetadata: Option<ExtMetadata>,
}

#[derive(Deserialize)]
struct ExtMetadata {
    #[serde(rename = "Categories")]
    categories: Option<MetadataValue>,
    #[serde(rename = "ImageDescription")]
    image_description: Option<MetadataValue>,
}

#[derive(Deserialize)]
struct MetadataValue {
    value: String,
}

/// An intermediate candidate image before filtering and ranking.
struct Candidate {
    url: String,
    thumb_url: String,
    width: u32,
    height: u32,
    /// Number of GEO_TERMS found in the image's metadata.
    geo_score: usize,
}

impl CommonsClient {
    pub fn new() -> Self {
        let http = Client::builder()
            .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .build()
            .expect("failed to build reqwest client");
        Self { http }
    }

    /// Fetch the Commons geosearch response with a small retry/backoff loop.
    ///
    /// Retries connect errors, HTTP 429 (rate limit), and 5xx responses up to
    /// `MAX_RETRIES` times with exponential backoff. Non-retryable HTTP statuses
    /// (e.g. 4xx) surface as an error immediately.
    async fn request_geosearch(
        &self,
        params: &[(&str, String)],
    ) -> Result<QueryResponse, reqwest::Error> {
        let mut attempt = 0;
        let mut backoff = Duration::from_millis(500);

        loop {
            let result = self
                .http
                .get("https://commons.wikimedia.org/w/api.php")
                .query(params)
                .header(reqwest::header::USER_AGENT, "wikiguessr-backend/0.1")
                .send()
                .await;

            match result {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        return resp.json::<QueryResponse>().await;
                    }
                    let retryable =
                        status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error();
                    if !retryable || attempt >= MAX_RETRIES {
                        return Err(resp.error_for_status().err().unwrap());
                    }
                }
                Err(e) => {
                    if attempt >= MAX_RETRIES {
                        return Err(e);
                    }
                }
            }

            attempt += 1;
            debug!(
                attempt,
                retry_in_ms = backoff.as_millis(),
                "retrying Commons request"
            );
            sleep(backoff).await;
            backoff = backoff.saturating_mul(2);
        }
    }

    /// Fetch, filter, and rank images near a coordinate.
    ///
    /// - Fetches up to 50 candidates within `radius_m` meters.
    /// - Excludes non-BITMAP, portraits, logos, flags, satellite imagery.
    /// - Ranks by geo-relevance keyword score.
    /// - Returns the top `limit` image URLs.
    pub async fn fetch_images(
        &self,
        coord: Coordinate,
        radius_m: u32,
        limit: usize,
    ) -> Result<Vec<Image>, reqwest::Error> {
        let params = [
            ("action", "query".to_string()),
            ("format", "json".to_string()),
            ("generator", "geosearch".to_string()),
            ("ggsprimary", "all".to_string()),
            ("ggsnamespace", "6".to_string()),
            ("ggsradius", radius_m.to_string()),
            (
                "ggscoord",
                format!("{}|{}", coord.latitude, coord.longitude),
            ),
            ("ggslimit", "50".to_string()),
            ("prop", "imageinfo".to_string()),
            ("iiprop", "url|extmetadata|mediatype|size".to_string()),
            ("iiurlwidth", "800".to_string()),
        ];

        let resp: QueryResponse = self.request_geosearch(&params).await?;

        let pages = match resp.query {
            Some(q) => q.pages,
            None => {
                debug!(
                    "Commons returned no pages for coord ({}, {})",
                    coord.latitude, coord.longitude
                );
                return Ok(Vec::new());
            }
        };

        let mut candidates: Vec<Candidate> = Vec::new();

        for page in pages.values() {
            let info = match page.imageinfo.as_ref().and_then(|ii| ii.first()) {
                Some(i) => i,
                None => continue,
            };

            // Skip non-photographic files (SVG, PDF, audio, video).
            if info.mediatype.as_deref() != Some("BITMAP") {
                continue;
            }

            let url = match &info.url {
                Some(u) => u.clone(),
                None => continue,
            };

            let thumb_url = info.thumburl.clone().unwrap_or_else(|| url.clone());
            let width = info.width.unwrap_or(0);
            let height = info.height.unwrap_or(0);

            let ext = info.extmetadata.as_ref();
            let categories = ext
                .and_then(|e| e.categories.as_ref())
                .map(|v| v.value.as_str())
                .unwrap_or("");
            let description = ext
                .and_then(|e| e.image_description.as_ref())
                .map(|v| v.value.as_str())
                .unwrap_or("");
            let title = page.title.as_deref().unwrap_or("");

            let lower_cats = categories.to_lowercase();

            // Skip portraits, logos, flags, coats of arms.
            if EXCLUDED_CATEGORIES.iter().any(|kw| lower_cats.contains(kw)) {
                continue;
            }

            // Skip satellite/orbital imagery.
            let search_text = format!("{title} {description} {categories}").to_lowercase();
            if SPACE_TERMS.iter().any(|term| search_text.contains(term)) {
                continue;
            }

            // Score by geo-relevance keywords.
            let geo_score = GEO_TERMS
                .iter()
                .filter(|term| search_text.contains(*term))
                .count();

            candidates.push(Candidate {
                url,
                thumb_url,
                width,
                height,
                geo_score,
            });
        }

        // Sort by geo-relevance descending, take top `limit`.
        candidates.sort_by_key(|a| std::cmp::Reverse(a.geo_score));
        candidates.truncate(limit);

        let images = candidates
            .into_iter()
            .map(|c| Image {
                url: c.url,
                thumb_url: c.thumb_url,
                width: c.width,
                height: c.height,
            })
            .collect();

        Ok(images)
    }
}

impl Default for CommonsClient {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geo_relevance_counts_matching_terms() {
        // Simulate the scoring logic in isolation.
        let text = "beautiful mountain landscape with a river valley";
        let score: usize = GEO_TERMS.iter().filter(|t| text.contains(*t)).count();
        // Matches: mountain, landscape, river, valley, and cape (substring of landscape) = 5
        assert_eq!(score, 5);
    }

    #[test]
    fn space_terms_detected() {
        let text = "landsat satellite image of earth";
        assert!(SPACE_TERMS.iter().any(|t| text.contains(t)));
    }

    #[test]
    fn new_satellite_terms_detected() {
        for text in [
            "nasa modis image of the atlantic",
            "weather satellite composite",
            "earth from orbit captured yesterday",
        ] {
            assert!(
                SPACE_TERMS.iter().any(|t| text.contains(t)),
                "expected {text} to be excluded"
            );
        }
    }

    #[test]
    fn excluded_categories_detected() {
        let cats = "portraits of people in france";
        assert!(EXCLUDED_CATEGORIES.iter().any(|kw| cats.contains(kw)));
    }

    #[test]
    fn clean_text_passes_filters() {
        let text = "a photograph of a quiet village street";
        assert!(!SPACE_TERMS.iter().any(|t| text.contains(t)));
        assert!(!EXCLUDED_CATEGORIES.iter().any(|kw| text.contains(kw)));
    }
}
