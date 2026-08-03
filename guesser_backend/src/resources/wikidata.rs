use crate::location::coordinates::{Coordinate, CoordinateError, ItemId, Location};
use rand::RngExt;
use reqwest::Client;
use serde::Deserialize;
use tracing::{debug, warn};

const MAX_OFFSET: usize = 5000;
// The public query.wikidata.org endpoint is a shared resource that can take a
// long time to answer a VALUES + OFFSET query when under load; give it headroom.
const REQUEST_TIMEOUT_SECS: u64 = 90;

/// Client for fetching random geotagged locations from Wikidata via SPARQL.
pub struct WikidataClient {
    http: Client,
}

#[derive(Deserialize)]
struct SparqlResponse {
    results: SparqlResults,
}

#[derive(Deserialize)]
struct SparqlResults {
    bindings: Vec<Binding>,
}

#[derive(Deserialize)]
struct Binding {
    item: ValueField,
    location: ValueField,
}

#[derive(Deserialize)]
struct ValueField {
    value: String,
}

impl WikidataClient {
    pub fn new() -> Self {
        let http = Client::builder()
            .timeout(std::time::Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .build()
            .expect("failed to build reqwest client");
        Self { http }
    }

    /// Curated Wikidata place classes (`wdt:P31`) eligible for the game.
    /// Deliberately geographic: settlements, natural features, and landmarks —
    /// excludes events, people, and other items that carry a coordinate but
    /// have no scenery to guess from.
    pub const PLACE_CLASSES: &'static [&'static str] = &[
        // settlements (direct P31 types)
        "wd:Q486972", // human settlement
        "wd:Q131681", // populated place
        "wd:Q515",    // city
        "wd:Q3957",   // town
        "wd:Q532",    // village
        "wd:Q5084",   // hamlet
        "wd:Q15284",  // municipality
        // natural features
        "wd:Q8502",    // mountain
        "wd:Q47521",   // mountain range
        "wd:Q23397",   // lake
        "wd:Q4022",    // river
        "wd:Q1287518", // island
        "wd:Q35509",   // volcano
        "wd:Q16970",   // waterfall
        "wd:Q46831",   // valley
        "wd:Q233591",  // glacier
        "wd:Q37893",   // desert
        "wd:Q1002697", // beach
        "wd:Q47566",   // cape
        "wd:Q41253",   // forest
        "wd:Q47059",   // fjord
        "wd:Q46169",   // national park
        // landmarks
        "wd:Q23413", // castle
        "wd:Q33506", // museum
        "wd:Q39715", // lighthouse
    ];

    /// Fetch a batch of random geotagged locations from Wikidata.
    ///
    /// Only items that are real places (`P31` in `PLACE_CLASSES`), carry a
    /// Wikimedia image (`P18`), and have coordinates (`P625`) are returned.
    /// Locations in polar regions (|latitude| > 70°) are filtered out
    /// post-parse since SPARQL `geof:` functions are unreliable on
    /// the public Wikidata endpoint.
    pub async fn fetch_batch(&self, batch_size: usize) -> Result<Vec<Location>, reqwest::Error> {
        let offset = rand::rng().random_range(0..MAX_OFFSET);
        let query = Self::sparql_query(batch_size, offset);

        let text = self
            .http
            .get("https://query.wikidata.org/sparql")
            .query(&[("query", &query), ("format", &"json".to_string())])
            .header(reqwest::header::USER_AGENT, "wikiguessr-backend/0.1")
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;

        let locations = Self::parse_locations(&text);
        debug!(
            raw_count = locations.len(),
            "fetched location batch from Wikidata"
        );

        // Filter polar regions (|lat| > 70°).
        let filtered: Vec<Location> = locations
            .into_iter()
            .filter(|loc| loc.coordinate.latitude.abs() <= 70.0)
            .collect();

        debug!(
            filtered_count = filtered.len(),
            "locations after polar filter"
        );
        Ok(filtered)
    }

    /// Build the SPARQL query for a random batch of place locations.
    ///
    /// Only items typed as a place (`P31` in `PLACE_CLASSES`) that also carry a
    /// Wikimedia image (`P18`) and coordinates (`P625`) are eligible, keeping
    /// events, people, and other coordinate-bearing-but-photoless items out of
    /// the pool.
    ///
    /// Deliberately avoids `SELECT DISTINCT`: Blazegraph materializes the whole
    /// result set to dedupe before applying `OFFSET`, which is a known
    /// performance killer on the public endpoint. Duplicate rows from an item
    /// matching several place classes are instead collapsed in
    /// `parse_locations`.
    fn sparql_query(batch_size: usize, offset: usize) -> String {
        let place_classes = Self::PLACE_CLASSES.join("\n                    ");
        format!(
            r#"
            SELECT ?item ?location WHERE {{
                VALUES ?placeClass {{
                    {place_classes}
                }}
                ?item wdt:P31 ?placeClass.
                ?item wdt:P625 ?location.
                ?item wdt:P18 ?image.
            }}
            LIMIT {batch_size}
            OFFSET {offset}
            "#,
        )
    }

    /// Parse a Wikidata SPARQL JSON response into `Location`s.
    ///
    /// Silently skips malformed bindings rather than failing the whole batch.
    fn parse_locations(json: &str) -> Vec<Location> {
        let response: SparqlResponse = match serde_json::from_str(json) {
            Ok(r) => r,
            Err(e) => {
                warn!(error = %e, "failed to parse SPARQL response");
                return Vec::new();
            }
        };

        let mut locations = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for binding in response.results.bindings {
            let item_id = match binding.item.value.rsplit('/').next() {
                Some(id) => ItemId(id.to_owned()),
                None => continue,
            };

            // An item can match several `P31` place classes, so the same row may
            // appear more than once in the (non-DISTINCT) result; keep one copy.
            if !seen.insert(item_id.0.clone()) {
                continue;
            }

            // WKT format: "Point(longitude latitude)" or
            //              "<http://...> Point(longitude latitude)"
            let point = match binding.location.value.split("Point(").nth(1) {
                Some(p) => p.trim_end_matches(')'),
                None => continue,
            };

            let mut coords = point.split_whitespace();
            let longitude: f64 = match coords.next().and_then(|s| s.parse().ok()) {
                Some(v) => v,
                None => continue,
            };
            let latitude: f64 = match coords.next().and_then(|s| s.parse().ok()) {
                Some(v) => v,
                None => continue,
            };

            // Validate (and normalize) the coordinate; skip out-of-range or
            // non-finite values instead of letting them poison the pool.
            let coordinate: Coordinate = match Coordinate::new(latitude, longitude) {
                Ok(c) => c,
                Err(CoordinateError) => continue,
            };

            locations.push(Location { item_id, coordinate });
        }
        locations
    }
}
impl Default for WikidataClient {
    fn default() -> Self {
        Self::new()
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_single_location() {
        let json = r#"
        {
            "results": {
                "bindings": [{
                    "item": { "type": "uri", "value": "http://www.wikidata.org/entity/Q42" },
                    "location": {
                        "type": "literal",
                        "value": "Point(2.3522 48.8566)"
                    }
                }]
            }
        }"#;

        let locations = WikidataClient::parse_locations(json);
        assert_eq!(locations.len(), 1);
        assert_eq!(locations[0].item_id.0, "Q42");
        assert!((locations[0].coordinate.longitude - 2.3522).abs() < 1e-4);
        assert!((locations[0].coordinate.latitude - 48.8566).abs() < 1e-4);
    }

    #[test]
    fn parses_location_with_crs_prefix() {
        let json = r#"
        {
            "results": {
                "bindings": [{
                    "item": { "type": "uri", "value": "http://www.wikidata.org/entity/Q25908933" },
                    "location": {
                        "type": "literal",
                        "value": "<http://www.wikidata.org/entity/Q3123> Point(-79.92 -35.5)"
                    }
                }]
            }
        }"#;

        let locations = WikidataClient::parse_locations(json);
        assert_eq!(locations.len(), 1);
        assert_eq!(locations[0].item_id.0, "Q25908933");
        assert!((locations[0].coordinate.longitude - (-79.92)).abs() < 1e-4);
        assert!((locations[0].coordinate.latitude - (-35.5)).abs() < 1e-4);
    }

    #[test]
    fn skips_malformed_bindings_gracefully() {
        let json = r#"
        {
            "results": {
                "bindings": [
                    {
                        "item": { "value": "http://www.wikidata.org/entity/Q1" },
                        "location": { "value": "not a point" }
                    },
                    {
                        "item": { "value": "http://www.wikidata.org/entity/Q2" },
                        "location": { "value": "Point(10.0 20.0)" }
                    }
                ]
            }
        }"#;

        let locations = WikidataClient::parse_locations(json);
        assert_eq!(locations.len(), 1);
        assert_eq!(locations[0].item_id.0, "Q2");
    }

    #[test]
    fn skips_out_of_range_coordinates() {
        let json = r#"
        {
            "results": {
                "bindings": [
                    {
                        "item": { "value": "http://www.wikidata.org/entity/Q1" },
                        "location": { "value": "Point(999.0 10.0)" }
                    },
                    {
                        "item": { "value": "http://www.wikidata.org/entity/Q2" },
                        "location": { "value": "Point(12.0 -35.0)" }
                    },
                    {
                        "item": { "value": "http://www.wikidata.org/entity/Q3" },
                        "location": { "value": "Point(10.0 91.0)" }
                    }
                ]
            }
        }"#;

        let locations = WikidataClient::parse_locations(json);
        assert_eq!(locations.len(), 1, "out-of-range coords must be skipped");
        assert_eq!(locations[0].item_id.0, "Q2");
    }

    #[test]
    fn sparql_query_avoids_distinct_and_uses_place_classes() {
        let query = WikidataClient::sparql_query(200, 42);
        assert!(!query.contains("DISTINCT"), "DISTINCT + OFFSET is slow on Blazegraph");
        assert!(query.contains("VALUES ?placeClass"), "query should use VALUES");
        assert!(query.contains("wd:Q515"), "query should list place classes");
        assert!(query.contains("?item wdt:P31 ?placeClass."), "query should filter P31");
        assert!(query.contains("?item wdt:P625 ?location."), "query should require coordinates");
        assert!(query.contains("?item wdt:P18 ?image."), "query should require a Wikimedia image");
        assert!(query.contains("LIMIT 200"));
        assert!(query.contains("OFFSET 42"));
    }

    #[test]
    fn deduplicates_items_matching_multiple_place_classes() {
        let json = r#"
        {
            "results": {
                "bindings": [
                    {
                        "item": { "value": "http://www.wikidata.org/entity/Q515" },
                        "location": { "value": "Point(2.3522 48.8566)" }
                    },
                    {
                        "item": { "value": "http://www.wikidata.org/entity/Q515" },
                        "location": { "value": "Point(2.3522 48.8566)" }
                    }
                ]
            }
        }"#;

        let locations = WikidataClient::parse_locations(json);
        assert_eq!(locations.len(), 1, "duplicate item rows must be collapsed");
    }

    #[test]
    fn invalid_json_returns_empty() {
        assert!(WikidataClient::parse_locations("not json").is_empty());
    }

    #[test]
    fn empty_bindings_returns_empty() {
        let json = r#"{ "results": { "bindings": [] } }"#;
        assert!(WikidataClient::parse_locations(json).is_empty());
    }
}
