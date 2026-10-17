//! Offline reverse geocoding: the nearest sizeable city to a GPS position, from a table
//! of GeoNames places (see `data/cities.tsv` and `scripts/build-cities.mjs`). No network
//! calls; the table is compiled into the binary and loaded on first use.

use std::collections::HashMap;
use std::sync::LazyLock;

const CITIES: &str = include_str!("../data/cities.tsv");

/// Photos further than this from any listed city get no place name.
const MAX_DISTANCE_KM: f64 = 100.0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub city: String,
    pub country: String,
}

struct City {
    name: &'static str,
    country: &'static str,
    lat: f64,
    lon: f64,
    /// Larger cities "reach" further, so a photo at the Eiffel Tower is labelled Paris
    /// rather than the slightly closer suburb of Neuilly-sur-Seine.
    weight: f64,
}

/// Cities bucketed by whole degree of latitude/longitude for quick neighbour lookups.
struct Index {
    cells: HashMap<(i32, i32), Vec<City>>,
}

static INDEX: LazyLock<Index> = LazyLock::new(|| {
    let mut cells: HashMap<(i32, i32), Vec<City>> = HashMap::new();
    for line in CITIES.lines() {
        let mut fields = line.split('\t');
        let (Some(name), Some(country), Some(lat), Some(lon), Some(population)) = (
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
        ) else {
            continue;
        };
        let (Ok(lat), Ok(lon), Ok(population)) = (
            lat.parse::<f64>(),
            lon.parse::<f64>(),
            population.parse::<f64>(),
        ) else {
            continue;
        };
        let weight = (population / 100_000.0).max(0.01).powf(0.25);
        cells.entry(cell(lat, lon)).or_default().push(City {
            name,
            country,
            lat,
            lon,
            weight,
        });
    }
    Index { cells }
});

fn cell(lat: f64, lon: f64) -> (i32, i32) {
    (lat.floor() as i32, lon.floor() as i32)
}

/// Approximate distance in km (equirectangular; plenty accurate at these ranges).
fn distance_km(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    const EARTH_RADIUS_KM: f64 = 6371.0;
    let mean_lat = ((lat1 + lat2) / 2.0).to_radians();
    let mut dlon = (lon2 - lon1).abs();
    if dlon > 180.0 {
        dlon = 360.0 - dlon;
    }
    let x = dlon.to_radians() * mean_lat.cos();
    let y = (lat2 - lat1).to_radians();
    (x * x + y * y).sqrt() * EARTH_RADIUS_KM
}

/// The nearest city within [`MAX_DISTANCE_KM`], if any.
pub fn lookup(lat: f64, lon: f64) -> Option<Place> {
    let index = &*INDEX;
    let (clat, clon) = cell(lat, lon);
    // One degree of latitude is ~111 km, so a 1-cell ring always covers the radius in
    // latitude; longitude cells shrink towards the poles, so widen the ring there.
    let lon_ring = (1.0 / lat.to_radians().cos().max(0.05)).ceil() as i32;
    let mut best: Option<(&City, f64)> = None;
    for dlat in -1..=1 {
        for dlon in -lon_ring..=lon_ring {
            let mut wrapped = clon + dlon;
            if wrapped >= 180 {
                wrapped -= 360;
            } else if wrapped < -180 {
                wrapped += 360;
            }
            for city in index
                .cells
                .get(&(clat + dlat, wrapped))
                .into_iter()
                .flatten()
            {
                let d = distance_km(lat, lon, city.lat, city.lon);
                let score = d / city.weight;
                if d <= MAX_DISTANCE_KM && best.is_none_or(|(_, b)| score < b) {
                    best = Some((city, score));
                }
            }
        }
    }
    best.map(|(city, _)| Place {
        city: city.name.to_string(),
        country: city.country.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn city(lat: f64, lon: f64) -> Option<(String, String)> {
        lookup(lat, lon).map(|p| (p.city, p.country))
    }

    #[test]
    fn finds_nearest_city() {
        // Kiyomizu-dera, Kyoto.
        assert_eq!(
            city(34.9949, 135.7850),
            Some(("Kyoto".into(), "Japan".into()))
        );
        // Eiffel Tower: Paris, not a closer but smaller suburb.
        assert_eq!(
            city(48.8584, 2.2945),
            Some(("Paris".into(), "France".into()))
        );
        // Clearly inside a suburb, the suburb wins.
        assert_eq!(city(48.8846, 2.2697).unwrap().0, "Neuilly-sur-Seine");
        // Near the antimeridian (Fiji).
        assert_eq!(city(-18.1416, 178.4415).unwrap().1, "Fiji");
    }

    #[test]
    fn nothing_in_the_middle_of_the_ocean() {
        assert_eq!(city(0.0, -140.0), None);
    }

    #[test]
    fn distance_is_sane() {
        let paris_london = distance_km(48.8566, 2.3522, 51.5074, -0.1278);
        assert!((330.0..360.0).contains(&paris_london), "{paris_london}");
    }
}
