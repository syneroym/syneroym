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
/// N-2: a stranger's bytes sit on a SynOrg owner's disk once a directory
/// replicates a listing, and an unbounded label is an unbounded field like
/// any other.
pub const MAX_NAMED_LABEL_LEN: usize = 128;

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
    #[error("a named area label is longer than {MAX_NAMED_LABEL_LEN} bytes")]
    LabelTooLong,
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
                if label.len() > MAX_NAMED_LABEL_LEN {
                    return Err(AreaError::LabelTooLong);
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

/// Exact box-in-box intersection over the sieve's own over-covered
/// candidates.
#[must_use]
pub fn boxes_intersect(a: &BoundingBox, b: &BoundingBox) -> bool {
    a.min_lat_e6 <= b.max_lat_e6
        && b.min_lat_e6 <= a.max_lat_e6
        && a.min_lon_e6 <= b.max_lon_e6
        && b.min_lon_e6 <= a.max_lon_e6
}

fn lat_deg(e6: i64) -> f64 {
    e6 as f64 / 1e6
}
fn lon_deg(e6: i64) -> f64 {
    e6 as f64 / 1e6
}

/// Metres between two lat/lon points, using the same flat-earth
/// approximation `bounding_box` uses -- exactness here only has to match
/// the same model the sieve was built on, not survey-grade geodesy.
fn approx_distance_m(lat1_e6: i64, lon1_e6: i64, lat2_e6: i64, lon2_e6: i64) -> f64 {
    let d_lat_m = (lat_deg(lat1_e6) - lat_deg(lat2_e6)) * M_PER_LAT_DEG;
    let mean_lat_rad = ((lat_deg(lat1_e6) + lat_deg(lat2_e6)) / 2.0).to_radians();
    let d_lon_m = (lon_deg(lon1_e6) - lon_deg(lon2_e6)) * M_PER_LAT_DEG * mean_lat_rad.cos();
    (d_lat_m * d_lat_m + d_lon_m * d_lon_m).sqrt()
}

/// Exact, on the sieve's own over-covered candidates. `None` when either
/// side is `Named` -- named areas never match geometrically, in either
/// direction, and the caller must render that as its own reason rather
/// than as "no match".
#[must_use]
pub fn areas_intersect(a: &Area, b: &Area) -> Option<bool> {
    match (a, b) {
        (Area::Named { .. }, _) | (_, Area::Named { .. }) => None,
        (
            Area::Circle { lat_e6: la, lon_e6: lo, radius_m: ra },
            Area::Circle { lat_e6: lb, lon_e6: lob, radius_m: rb },
        ) => {
            let d = approx_distance_m(*la, *lo, *lb, *lob);
            Some(d <= (*ra + *rb) as f64)
        }
        (
            Area::Circle { lat_e6, lon_e6, radius_m },
            Area::Bbox { min_lat_e6, min_lon_e6, max_lat_e6, max_lon_e6 },
        )
        | (
            Area::Bbox { min_lat_e6, min_lon_e6, max_lat_e6, max_lon_e6 },
            Area::Circle { lat_e6, lon_e6, radius_m },
        ) => {
            let closest_lat = (*lat_e6).clamp(*min_lat_e6, *max_lat_e6);
            let closest_lon = (*lon_e6).clamp(*min_lon_e6, *max_lon_e6);
            let d = approx_distance_m(*lat_e6, *lon_e6, closest_lat, closest_lon);
            Some(d <= *radius_m as f64)
        }
        (
            Area::Bbox {
                min_lat_e6: a_min_lat,
                min_lon_e6: a_min_lon,
                max_lat_e6: a_max_lat,
                max_lon_e6: a_max_lon,
            },
            Area::Bbox {
                min_lat_e6: b_min_lat,
                min_lon_e6: b_min_lon,
                max_lat_e6: b_max_lat,
                max_lon_e6: b_max_lon,
            },
        ) => {
            let ba = BoundingBox {
                min_lat_e6: *a_min_lat,
                min_lon_e6: *a_min_lon,
                max_lat_e6: *a_max_lat,
                max_lon_e6: *a_max_lon,
            };
            let bb = BoundingBox {
                min_lat_e6: *b_min_lat,
                min_lon_e6: *b_min_lon,
                max_lat_e6: *b_max_lat,
                max_lon_e6: *b_max_lon,
            };
            Some(boxes_intersect(&ba, &bb))
        }
    }
}

/// Case-folded, trimmed label equality. `false` for any pairing that is not
/// two `Named` areas.
#[must_use]
pub fn labels_match(a: &Area, b: &Area) -> bool {
    match (a, b) {
        (Area::Named { label: la, .. }, Area::Named { label: lb, .. }) => {
            la.trim().to_lowercase() == lb.trim().to_lowercase()
        }
        _ => false,
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
        assert_eq!(
            Area::Named { label: "x".repeat(MAX_NAMED_LABEL_LEN + 1), code: None }.validate(),
            Err(AreaError::LabelTooLong)
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

    #[test]
    fn boxes_intersect_touches_and_overlaps() {
        let a = BoundingBox { min_lat_e6: 0, min_lon_e6: 0, max_lat_e6: 10, max_lon_e6: 10 };
        let overlapping =
            BoundingBox { min_lat_e6: 5, min_lon_e6: 5, max_lat_e6: 15, max_lon_e6: 15 };
        assert!(boxes_intersect(&a, &overlapping));
        let touching =
            BoundingBox { min_lat_e6: 10, min_lon_e6: 10, max_lat_e6: 20, max_lon_e6: 20 };
        assert!(boxes_intersect(&a, &touching));
        let disjoint =
            BoundingBox { min_lat_e6: 20, min_lon_e6: 20, max_lat_e6: 30, max_lon_e6: 30 };
        assert!(!boxes_intersect(&a, &disjoint));
    }

    #[test]
    fn areas_intersect_named_is_never_geometric() {
        let named = Area::Named { label: "x".to_string(), code: None };
        let circle = Area::Circle { lat_e6: 0, lon_e6: 0, radius_m: 1000 };
        assert_eq!(areas_intersect(&named, &circle), None);
        assert_eq!(areas_intersect(&circle, &named), None);
        assert_eq!(areas_intersect(&named, &named), None);
    }

    #[test]
    fn areas_intersect_two_circles() {
        // Two 5 km-radius circles 6 km apart: sum of radii (10 km) > 6 km,
        // so they intersect.
        let a = Area::Circle { lat_e6: 13_000_000, lon_e6: 77_500_000, radius_m: 5_000 };
        // ~6 km north: 6000 / 110_574 deg ~ 0.0543 deg ~ 54_260 e6.
        let b = Area::Circle { lat_e6: 13_054_260, lon_e6: 77_500_000, radius_m: 5_000 };
        assert_eq!(areas_intersect(&a, &b), Some(true));
        let far = Area::Circle { lat_e6: 20_000_000, lon_e6: 77_500_000, radius_m: 5_000 };
        assert_eq!(areas_intersect(&a, &far), Some(false));
    }

    #[test]
    fn areas_intersect_circle_and_bbox() {
        let circle = Area::Circle { lat_e6: 0, lon_e6: 0, radius_m: 5_000 };
        let overlapping_box =
            Area::Bbox { min_lat_e6: -1000, min_lon_e6: -1000, max_lat_e6: 1000, max_lon_e6: 1000 };
        assert_eq!(areas_intersect(&circle, &overlapping_box), Some(true));
        assert_eq!(areas_intersect(&overlapping_box, &circle), Some(true));
        let far_box = Area::Bbox {
            min_lat_e6: 10_000_000,
            min_lon_e6: 10_000_000,
            max_lat_e6: 10_100_000,
            max_lon_e6: 10_100_000,
        };
        assert_eq!(areas_intersect(&circle, &far_box), Some(false));
    }

    #[test]
    fn a_box_inside_the_over_covered_circle_projection_but_outside_the_true_circle_is_excluded() {
        // A 10 km circle's bounding box over-covers by design; pick a point
        // inside the box's corner but far outside the true circle radius.
        let circle = Area::Circle { lat_e6: 13_000_000, lon_e6: 77_500_000, radius_m: 10_000 };
        let bbox = bounding_box(&circle).unwrap();
        assert!(boxes_intersect(
            &bbox,
            &BoundingBox {
                min_lat_e6: bbox.max_lat_e6 - 10,
                min_lon_e6: bbox.max_lon_e6 - 10,
                max_lat_e6: bbox.max_lat_e6,
                max_lon_e6: bbox.max_lon_e6,
            }
        ));
        let corner_point = Area::Bbox {
            min_lat_e6: bbox.max_lat_e6,
            min_lon_e6: bbox.max_lon_e6,
            max_lat_e6: bbox.max_lat_e6,
            max_lon_e6: bbox.max_lon_e6,
        };
        assert_eq!(areas_intersect(&circle, &corner_point), Some(false));
    }

    #[test]
    fn labels_match_is_case_folded_and_trimmed() {
        let a = Area::Named { label: "  Bengaluru  ".to_string(), code: None };
        let b = Area::Named { label: "bengaluru".to_string(), code: None };
        assert!(labels_match(&a, &b));
        let c = Area::Named { label: "Mumbai".to_string(), code: None };
        assert!(!labels_match(&a, &c));
        assert!(!labels_match(&a, &Area::Circle { lat_e6: 0, lon_e6: 0, radius_m: 1 }));
    }
}
