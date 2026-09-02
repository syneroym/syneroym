//! Where a listing applies. Micro-degrees (1e-6 deg, about 11 cm) because
//! a signed payload may hold no number that is not an integer: the
//! canonical encoding is only reproducible for integers, so the host
//! refuses a decimal before it signs.

use serde::{Deserialize, Serialize};

pub const MAX_AREAS: usize = 8;
pub const LAT_E6_MIN: i64 = -90_000_000;
pub const LAT_E6_MAX: i64 = 90_000_000;
pub const LON_E6_MIN: i64 = -180_000_000;
pub const LON_E6_MAX: i64 = 180_000_000;
/// Half the earth's circumference: a larger radius covers the whole globe,
/// so anything beyond this is a mistake, not a wider area.
pub const MAX_RADIUS_M: u64 = 40_075_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Area {
    /// What an index wants.
    Bbox { min_lat_e6: i64, min_lon_e6: i64, max_lat_e6: i64, max_lon_e6: i64 },
    /// What a provider actually thinks in: "I travel this far from here".
    Circle { lat_e6: i64, lon_e6: i64, radius_m: u64 },
    /// What a person reads. Never queried geometrically.
    Named {
        label: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        code: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoundingBox {
    pub min_lat_e6: i64,
    pub min_lon_e6: i64,
    pub max_lat_e6: i64,
    pub max_lon_e6: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AreaError {
    #[error("latitude {0} is outside [-90e6, 90e6]")]
    LatOutOfRange(i64),
    #[error("longitude {0} is outside [-180e6, 180e6]")]
    LonOutOfRange(i64),
    #[error("bbox min {min} is greater than max {max}")]
    MinAboveMax { min: i64, max: i64 },
    #[error("radius {0} m exceeds half the earth's circumference")]
    RadiusTooLarge(u64),
    #[error("a named area needs a non-empty label")]
    EmptyLabel,
}

fn check_lat(v: i64) -> Result<(), AreaError> {
    (LAT_E6_MIN..=LAT_E6_MAX).contains(&v).then_some(()).ok_or(AreaError::LatOutOfRange(v))
}

fn check_lon(v: i64) -> Result<(), AreaError> {
    (LON_E6_MIN..=LON_E6_MAX).contains(&v).then_some(()).ok_or(AreaError::LonOutOfRange(v))
}

impl Area {
    pub fn validate(&self) -> Result<(), AreaError> {
        match self {
            Area::Bbox { min_lat_e6, min_lon_e6, max_lat_e6, max_lon_e6 } => {
                check_lat(*min_lat_e6)?;
                check_lat(*max_lat_e6)?;
                check_lon(*min_lon_e6)?;
                check_lon(*max_lon_e6)?;
                if min_lat_e6 > max_lat_e6 {
                    return Err(AreaError::MinAboveMax { min: *min_lat_e6, max: *max_lat_e6 });
                }
                if min_lon_e6 > max_lon_e6 {
                    return Err(AreaError::MinAboveMax { min: *min_lon_e6, max: *max_lon_e6 });
                }
                Ok(())
            }
            Area::Circle { lat_e6, lon_e6, radius_m } => {
                check_lat(*lat_e6)?;
                check_lon(*lon_e6)?;
                if *radius_m > MAX_RADIUS_M {
                    return Err(AreaError::RadiusTooLarge(*radius_m));
                }
                Ok(())
            }
            Area::Named { label, .. } => {
                if label.trim().is_empty() {
                    return Err(AreaError::EmptyLabel);
                }
                Ok(())
            }
        }
    }
}

/// A conservative metres-per-degree of latitude. The real value ranges
/// ~110 574 (equator) to ~111 694 (pole); the smaller number turns a given
/// radius into *more* degrees, so the box always over-covers.
const M_PER_LAT_DEG: f64 = 110_000.0;

/// `None` for `Named`, which has no geometry. One definition, so the
/// service that publishes an area and the service that indexes it cannot
/// disagree about what it covers.
///
/// A `Circle` is projected to a box that **over**-covers, never
/// under-covers: an index built on it can return a false positive a later
/// exact check drops, and can never miss a match.
#[must_use]
pub fn bounding_box(area: &Area) -> Option<BoundingBox> {
    match area {
        Area::Bbox { min_lat_e6, min_lon_e6, max_lat_e6, max_lon_e6 } => Some(BoundingBox {
            min_lat_e6: *min_lat_e6,
            min_lon_e6: *min_lon_e6,
            max_lat_e6: *max_lat_e6,
            max_lon_e6: *max_lon_e6,
        }),
        Area::Circle { lat_e6, lon_e6, radius_m } => {
            let lat_deg = *lat_e6 as f64 / 1e6;
            let d_lat_deg = *radius_m as f64 / M_PER_LAT_DEG;
            // Longitude degrees shrink with cos(latitude); dividing by a
            // floor on cos widens the box. Near the poles cos -> 0, so we
            // fall back to the whole meridian rather than an infinity.
            let cos_lat = lat_deg.to_radians().cos().abs();
            let d_lon_deg = if cos_lat < 0.01 { 360.0 } else { d_lat_deg / cos_lat };
            let d_lat_e6 = (d_lat_deg * 1e6).ceil() as i64;
            let d_lon_e6 = (d_lon_deg * 1e6).ceil() as i64;
            Some(BoundingBox {
                min_lat_e6: (lat_e6 - d_lat_e6).max(LAT_E6_MIN),
                max_lat_e6: (lat_e6 + d_lat_e6).min(LAT_E6_MAX),
                min_lon_e6: (lon_e6 - d_lon_e6).max(LON_E6_MIN),
                max_lon_e6: (lon_e6 + d_lon_e6).min(LON_E6_MAX),
            })
        }
        Area::Named { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bbox_validation_bounds() {
        assert!(
            Area::Bbox { min_lat_e6: 0, min_lon_e6: 0, max_lat_e6: 1, max_lon_e6: 1 }
                .validate()
                .is_ok()
        );
        assert_eq!(
            Area::Bbox { min_lat_e6: 90_000_001, min_lon_e6: 0, max_lat_e6: 0, max_lon_e6: 0 }
                .validate(),
            Err(AreaError::LatOutOfRange(90_000_001))
        );
        assert_eq!(
            Area::Bbox { min_lat_e6: 0, min_lon_e6: 0, max_lat_e6: 0, max_lon_e6: -1 }.validate(),
            Err(AreaError::MinAboveMax { min: 0, max: -1 })
        );
        assert_eq!(
            Area::Bbox { min_lat_e6: 0, min_lon_e6: -180_000_001, max_lat_e6: 0, max_lon_e6: 0 }
                .validate(),
            Err(AreaError::LonOutOfRange(-180_000_001))
        );
    }

    #[test]
    fn circle_validation_and_radius_cap() {
        assert!(
            Area::Circle { lat_e6: 12_000_000, lon_e6: 77_000_000, radius_m: 5_000 }
                .validate()
                .is_ok()
        );
        assert_eq!(
            Area::Circle { lat_e6: 0, lon_e6: 0, radius_m: MAX_RADIUS_M + 1 }.validate(),
            Err(AreaError::RadiusTooLarge(MAX_RADIUS_M + 1))
        );
    }

    #[test]
    fn named_needs_a_label() {
        assert!(Area::Named { label: "Bengaluru".to_string(), code: None }.validate().is_ok());
        assert_eq!(
            Area::Named { label: "  ".to_string(), code: None }.validate(),
            Err(AreaError::EmptyLabel)
        );
    }

    #[test]
    fn named_has_no_geometry() {
        assert!(bounding_box(&Area::Named { label: "x".to_string(), code: None }).is_none());
    }

    #[test]
    fn circle_box_over_covers_never_under_covers() {
        // A 10 km circle near Bengaluru (~13N).
        let c = Area::Circle { lat_e6: 13_000_000, lon_e6: 77_500_000, radius_m: 10_000 };
        let b = bounding_box(&c).unwrap();
        // The exact latitude half-span of 10 km is ~0.0904 deg = 90_400 e6-less...
        // in micro-degrees ~90_360. The box must be at least that wide.
        let exact_d_lat_e6 = (10_000.0 / 111_320.0 * 1e6) as i64;
        assert!(b.max_lat_e6 - 13_000_000 >= exact_d_lat_e6, "latitude span under-covers");
        // Longitude at 13N: 1 deg ~ 108.5 km, so half-span ~ 0.0922 deg.
        let exact_d_lon_e6 = (10_000.0 / (111_320.0 * 13.0_f64.to_radians().cos()) * 1e6) as i64;
        assert!(b.max_lon_e6 - 77_500_000 >= exact_d_lon_e6, "longitude span under-covers");
        // And it stays inside the valid ranges.
        assert!(b.min_lat_e6 >= LAT_E6_MIN && b.max_lat_e6 <= LAT_E6_MAX);
    }

    #[test]
    fn circle_box_near_pole_falls_back_to_whole_meridian() {
        let c = Area::Circle { lat_e6: 89_500_000, lon_e6: 0, radius_m: 50_000 };
        let b = bounding_box(&c).unwrap();
        assert_eq!(b.min_lon_e6, LON_E6_MIN);
        assert_eq!(b.max_lon_e6, LON_E6_MAX);
    }

    #[test]
    fn bbox_round_trips_through_bounding_box() {
        let a = Area::Bbox { min_lat_e6: -5, min_lon_e6: -6, max_lat_e6: 7, max_lon_e6: 8 };
        assert_eq!(
            bounding_box(&a),
            Some(BoundingBox { min_lat_e6: -5, min_lon_e6: -6, max_lat_e6: 7, max_lon_e6: 8 })
        );
    }
}
