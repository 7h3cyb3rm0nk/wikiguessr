use crate::location::coordinates::Coordinate;

const EARTH_RADIUS_METER: f64 = 6_371_000.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScoringConfig {
    pub max_score: u32,
    pub decay_m: f64,
}

impl Default for ScoringConfig {
    fn default() -> Self {
        Self {
            max_score: 5000,
            decay_m: 2000.0,
        }
    }
}

pub fn haversine_distance_meters(a: Coordinate, b: Coordinate) -> f64 {
    let lat1 = a.latitude().to_radians();
    let lat2 = b.latitude().to_radians();
    let delta_lat = (b.latitude() - a.latitude()).to_radians();
    let delta_lon = (b.longitude() - a.longitude()).to_radians();

    let sin_lat = (delta_lat / 2.0).sin();
    let sin_lon = (delta_lon / 2.0).sin();

    let h = sin_lat * sin_lat + lat1.cos() * lat2.cos() * sin_lon * sin_lon;

    let h = h.clamp(0.0, 1.0);

    2.0 * EARTH_RADIUS_METER * h.sqrt().asin()
}

pub fn score_guess(guess: Coordinate, actual: Coordinate, config: ScoringConfig) -> u32 {
    let distance = haversine_distance_meters(guess, actual);
    let raw = config.max_score as f64 * (-distance / config.decay_m).exp();
    raw.round().clamp(0.0, config.max_score as f64) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn coord(lat: f64, lon: f64) -> Coordinate {
        Coordinate::new(lat, lon).unwrap()
    }

    #[test]
    fn zero_distance_is_max_score() {
        let london = coord(51.5074, -0.1278);
        let config = ScoringConfig::default();
        assert_eq!(score_guess(london, london, config), config.max_score);
    }

    #[test]
    fn known_distance_london_paris() {
        // London to Paris is ~344 km, well-established reference value.
        let london = coord(51.5074, -0.1278);
        let paris = coord(48.8566, 2.3522);
        let d = haversine_distance_meters(london, paris);
        // allow 1% tolerance against the commonly cited ~344km figure
        assert!((343_000.0..347_000.0).contains(&d), "got {d}");
    }

    #[test]
    fn far_distance_scores_near_zero() {
        let london = coord(51.5074, -0.1278);
        let sydney = coord(-33.8688, 151.2093);
        let config = ScoringConfig::default();
        let score = score_guess(london, sydney, config);
        assert!(score < 5, "expected near-zero score, got {score}");
    }

    #[test]
    fn score_never_exceeds_max_or_goes_negative() {
        let a = coord(0.0, 0.0);
        let b = coord(0.0, 0.0001); // tiny distance, should round up toward max but not exceed it
        let config = ScoringConfig::default();
        let score = score_guess(a, b, config);
        assert!(score <= config.max_score);
    }

    #[test]
    fn antipodal_points_do_not_panic() {
        // asin domain guard: h can overshoot 1.0 by float error near antipodes
        let a = coord(0.0, 0.0);
        let b = coord(0.0, 180.0);
        let d = haversine_distance_meters(a, b);
        assert!((d - std::f64::consts::PI * EARTH_RADIUS_METER).abs() < 1.0);
    }

    #[test]
    fn distance_is_symmetric() {
        let a = coord(10.0, 20.0);
        let b = coord(-5.0, 100.0);
        assert!((haversine_distance_meters(a, b) - haversine_distance_meters(b, a)).abs() < 1e-6);
    }
}
