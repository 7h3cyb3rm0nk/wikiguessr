use crate::location::coordinates::{Coordinate, ItemId, Location};
use rand::RngExt;
use reqwest::Client;
pub struct ResourceManager {
    pub http: reqwest::Client,
    pub locations: Vec<Location>,
}

use serde::Deserialize;

#[derive(Deserialize)]
struct SparqlResponse {
    results: Results,
}

#[derive(Deserialize)]
struct Results {
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
const PAGE_SIZE: usize = 100;
const MAX_PAGE: usize = 5000;

impl ResourceManager {
    pub fn new() -> Self {
        Self {
            http: Client::new(),
            locations: Vec::new(),
        }
    }

    pub async fn load_locations(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let json = self.fetch_locations().await?;
        self.locations = Self::parse_locations(&json)?;
        Ok(())
    }

    pub async fn fetch_locations(&mut self) -> Result<String, reqwest::Error> {
        let offset = rand::rng().random_range(0..MAX_PAGE);
        let query = format!(
            r#"
        SELECT ?item ?location WHERE {{
        ?item wdt:P625 ?location.
         }}
         LIMIT 100
         OFFSET {}
        "#,
            offset
        );
        self.http
            .get("https://query.wikidata.org/sparql")
            .query(&[("query", query), ("format", "json".to_string())])
            .header(reqwest::header::USER_AGENT, "wikiguessr/0.1")
            .send()
            .await?
            .error_for_status()?
            .text()
            .await
    }

    pub fn parse_locations(json: &str) -> Result<Vec<Location>, serde_json::Error> {
        let response: SparqlResponse = serde_json::from_str(json)?;
        let mut locations = Vec::new();

        for binding in response.results.bindings {
            let item_id = ItemId(binding.item.value.rsplit('/').next().unwrap().to_owned());
            let point = binding
                .location
                .value
                .split("Point(")
                .nth(1)
                .unwrap()
                .trim_end_matches(")");

            let mut coords = point.split_whitespace();

            let longitude: f64 = coords.next().unwrap().parse().unwrap();
            let latitude: f64 = coords.next().unwrap().parse().unwrap();

            locations.push(Location {
                item_id,
                coordinate: Coordinate {
                    latitude,
                    longitude,
                },
            })
        }
        Ok(locations)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_single_location_with_crs() {
        let json = r#"
        {
            "results": {
                "bindings": [
                    {
                        "item": {
                            "type": "uri",
                            "value": "http://www.wikidata.org/entity/Q25908933"
                        },
                        "location": {
                            "datatype": "http://www.opengis.net/ont/geosparql#wktLiteral",
                            "type": "literal",
                            "value": "<http://www.wikidata.org/entity/Q3123> Point(-354.53 -79.92)"
                        }
                    }
                ]
            }
        }
        "#;

        let locations = ResourceManager::parse_locations(json).unwrap();

        assert_eq!(locations.len(), 1);

        let location = &locations[0];

        assert_eq!(location.item_id.0, "Q25908933");
        assert_eq!(location.coordinate.longitude, -354.53);
        assert_eq!(location.coordinate.latitude, -79.92);
    }

    #[test]
    fn parses_multiple_locations_with_crs() {
        let json = r#"
        {
            "results": {
                "bindings": [
                    {
                        "item": {
                            "type": "uri",
                            "value": "http://www.wikidata.org/entity/Q25908933"
                        },
                        "location": {
                            "datatype": "http://www.opengis.net/ont/geosparql#wktLiteral",
                            "type": "literal",
                            "value": "<http://www.wikidata.org/entity/Q3123> Point(-354.53 -79.92)"
                        }
                    },
                    {
                        "item": {
                            "type": "uri",
                            "value": "http://www.wikidata.org/entity/Q112252041"
                        },
                        "location": {
                            "datatype": "http://www.opengis.net/ont/geosparql#wktLiteral",
                            "type": "literal",
                            "value": "<http://www.wikidata.org/entity/Q308> Point(-358.41 -70.34)"
                        }
                    }
                ]
            }
        }
        "#;

        let locations = ResourceManager::parse_locations(json).unwrap();

        assert_eq!(locations.len(), 2);

        assert_eq!(locations[0].item_id.0, "Q25908933");
        assert_eq!(locations[1].item_id.0, "Q112252041");

        assert_eq!(locations[0].coordinate.longitude, -354.53);
        assert_eq!(locations[1].coordinate.longitude, -358.41);
    }

    #[test]
    fn parses_empty_results() {
        let json = r#"
        {
            "results": {
                "bindings": []
            }
        }
        "#;

        let locations = ResourceManager::parse_locations(json).unwrap();

        assert!(locations.is_empty());
    }

    #[test]
    fn rejects_invalid_json() {
        assert!(ResourceManager::parse_locations("not json").is_err());
    }

    #[tokio::test]
    #[ignore]
    async fn fetches_locations_from_wikidata() {
        let mut manager = ResourceManager::new();

        let json = manager.fetch_locations().await.unwrap();
        let locations = ResourceManager::parse_locations(&json).unwrap();

        println!("Fetched {} locations", locations.len());

        for location in &locations {
            println!("{:?}", location);
        }

        assert!(!locations.is_empty());
    }
}
