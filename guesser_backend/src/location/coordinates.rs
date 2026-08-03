use serde::{Deserialize, Serialize};
use thiserror::Error;
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
pub struct Coordinate {
    pub latitude: f64,
    pub longitude: f64,
}
#[derive(Debug, Serialize, Deserialize, Error, PartialEq)]
#[error("CoordinateError")]
pub struct CoordinateError;

impl Coordinate {
    pub fn new(latitude: f64, longitude: f64) -> Result<Self, CoordinateError> {
        if !latitude.is_finite() || !longitude.is_finite() {
            return Err(CoordinateError);
        }
        if !(-90.0..=90.0).contains(&latitude) {
            return Err(CoordinateError);
        }
        if !(-180.0..=180.0).contains(&longitude) {
            return Err(CoordinateError);
        }
        Ok(Self {
            latitude,
            longitude,
        })
    }

    pub fn latitude(&self) -> f64 {
        self.latitude
    }

    pub fn longitude(&self) -> f64 {
        self.longitude
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Location {
    pub item_id: ItemId,
    pub coordinate: Coordinate,
}

impl Location {
    pub fn new(item_id: ItemId, coordinate: Coordinate) -> Self {
        Self {
            item_id,
            coordinate,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ItemId(pub String);
impl ItemId {
    pub fn new(id: String) -> Self {
        Self(id)
    }
}
//  A player's submitted guess for a round. Distinct from `Location`
//  because a guess has no `item_id` — the player doesn't know what
// the target's Wikidata item is, only where they clicked.
// #[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
// pub struct Guess {
//     pub coordinate: Coordinate,
// }
//

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_coordinate_accepted() {
        assert!(Coordinate::new(51.5074, -0.1278).is_ok()); // London
    }

    #[test]
    fn latitude_out_of_range_rejected() {
        assert_eq!(Coordinate::new(91.0, 0.0), Err(CoordinateError));
        assert_eq!(Coordinate::new(-91.0, 0.0), Err(CoordinateError));
    }

    #[test]
    fn longitude_out_of_range_rejected() {
        assert_eq!(Coordinate::new(0.0, 181.0), Err(CoordinateError));
        assert_eq!(Coordinate::new(0.0, -181.0), Err(CoordinateError));
    }

    #[test]
    fn boundary_values_accepted() {
        assert!(Coordinate::new(90.0, 180.0).is_ok());
        assert!(Coordinate::new(-90.0, -180.0).is_ok());
    }

    #[test]
    fn nan_and_infinite_rejected() {
        assert_eq!(Coordinate::new(f64::NAN, 0.0), Err(CoordinateError));
        assert_eq!(Coordinate::new(f64::INFINITY, 0.0), Err(CoordinateError));
    }
}
