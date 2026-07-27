use crate::location::coordinates::{Coordinate, ItemId, Location};
use rand::RngExt;
use reqwest::Client;
use serde::Deserialize;
use tracing::{debug, warn};

const MAX_OFFSET: usize = 5000;

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
        Self {
            http: Client::new(),
        }
    }

    /// Fetch a batch of random geotagged locations from Wikidata.
    ///
    /// Locations in polar regions (|latitude| > 70°) are filtered out
    /// post-parse since SPARQL `geof:` functions are unreliable on
    /// the public Wikidata endpoint.
    pub async fn fetch_batch(&self, batch_size: usize) -> Result<Vec<Location>, reqwest::Error> {
        let offset = rand::rng().random_range(0..MAX_OFFSET);
        let query = format!(
            r#"
            SELECT ?item ?location WHERE {{
                ?item wdt:P625 ?location.
            }}
            LIMIT {batch_size}
            OFFSET {offset}
            "#,
        );

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
        for binding in response.results.bindings {
            let item_id = match binding.item.value.rsplit('/').next() {
                Some(id) => ItemId(id.to_owned()),
                None => continue,
            };

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

            locations.push(Location {
                item_id,
                coordinate: Coordinate {
                    latitude,
                    longitude,
                },
            });
        }
        locations
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
                        "value": "<http://www.wikidata.org/entity/Q3123> Point(-354.53 -79.92)"
                    }
                }]
            }
        }"#;

        let locations = WikidataClient::parse_locations(json);
        assert_eq!(locations.len(), 1);
        assert_eq!(locations[0].item_id.0, "Q25908933");
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
    fn invalid_json_returns_empty() {
        assert!(WikidataClient::parse_locations("not json").is_empty());
    }

    #[test]
    fn empty_bindings_returns_empty() {
        let json = r#"{ "results": { "bindings": [] } }"#;
        assert!(WikidataClient::parse_locations(json).is_empty());
    }
}
