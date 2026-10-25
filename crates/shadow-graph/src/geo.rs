//! Just enough geography to make latencies honest.
//!
//! Routers live in real cities so that a New York to London hop costs what it
//! costs in reality (~35 ms one way over fibre, not the 28 ms the great
//! circle suggests, because cables do not follow great circles).

use crate::ids::CityId;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Region {
    NorthAmerica,
    SouthAmerica,
    Europe,
    Africa,
    Asia,
    Oceania,
}

impl Region {
    pub const ALL: [Region; 6] =
        [Region::NorthAmerica, Region::SouthAmerica, Region::Europe, Region::Africa, Region::Asia, Region::Oceania];

    pub fn code(self) -> &'static str {
        match self {
            Region::NorthAmerica => "NA",
            Region::SouthAmerica => "SA",
            Region::Europe => "EU",
            Region::Africa => "AF",
            Region::Asia => "AS",
            Region::Oceania => "OC",
        }
    }

    /// Rough share of networks per region, used when deciding where
    /// tier-2 and stub ASes live. Not census data; just the right shape.
    pub fn weight(self) -> f64 {
        match self {
            Region::NorthAmerica => 0.27,
            Region::Europe => 0.27,
            Region::Asia => 0.28,
            Region::SouthAmerica => 0.08,
            Region::Africa => 0.05,
            Region::Oceania => 0.05,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct City {
    pub code: &'static str,
    pub name: &'static str,
    pub lat: f64,
    pub lon: f64,
    pub region: Region,
    /// Carrier-hotel gravity. Big exchange cities attract more POPs and more
    /// interconnects (Frankfurt, Ashburn, Singapore...).
    pub hub: f64,
}

macro_rules! city {
    ($code:literal, $name:literal, $lat:expr, $lon:expr, $region:ident, $hub:expr) => {
        City { code: $code, name: $name, lat: $lat, lon: $lon, region: Region::$region, hub: $hub }
    };
}

pub static CITIES: &[City] = &[
    // North America
    city!("nyc", "New York", 40.71, -74.01, NorthAmerica, 1.0),
    city!("iad", "Ashburn", 39.04, -77.49, NorthAmerica, 1.0),
    city!("chi", "Chicago", 41.88, -87.63, NorthAmerica, 0.8),
    city!("dfw", "Dallas", 32.78, -96.80, NorthAmerica, 0.7),
    city!("atl", "Atlanta", 33.75, -84.39, NorthAmerica, 0.6),
    city!("mia", "Miami", 25.76, -80.19, NorthAmerica, 0.7),
    city!("lax", "Los Angeles", 34.05, -118.24, NorthAmerica, 0.9),
    city!("sjc", "San Jose", 37.34, -121.89, NorthAmerica, 0.9),
    city!("sea", "Seattle", 47.61, -122.33, NorthAmerica, 0.6),
    city!("den", "Denver", 39.74, -104.99, NorthAmerica, 0.4),
    city!("yyz", "Toronto", 43.65, -79.38, NorthAmerica, 0.5),
    city!("yul", "Montreal", 45.50, -73.57, NorthAmerica, 0.3),
    city!("mex", "Mexico City", 19.43, -99.13, NorthAmerica, 0.4),
    // South America
    city!("gru", "Sao Paulo", -23.55, -46.63, SouthAmerica, 0.8),
    city!("gig", "Rio de Janeiro", -22.91, -43.17, SouthAmerica, 0.4),
    city!("eze", "Buenos Aires", -34.60, -58.38, SouthAmerica, 0.5),
    city!("scl", "Santiago", -33.45, -70.67, SouthAmerica, 0.4),
    city!("bog", "Bogota", 4.71, -74.07, SouthAmerica, 0.3),
    city!("lim", "Lima", -12.05, -77.04, SouthAmerica, 0.3),
    // Europe
    city!("lon", "London", 51.51, -0.13, Europe, 1.0),
    city!("ams", "Amsterdam", 52.37, 4.90, Europe, 1.0),
    city!("fra", "Frankfurt", 50.11, 8.68, Europe, 1.0),
    city!("par", "Paris", 48.86, 2.35, Europe, 0.8),
    city!("mad", "Madrid", 40.42, -3.70, Europe, 0.5),
    city!("mil", "Milan", 45.46, 9.19, Europe, 0.5),
    city!("sto", "Stockholm", 59.33, 18.07, Europe, 0.5),
    city!("waw", "Warsaw", 52.23, 21.01, Europe, 0.4),
    city!("vie", "Vienna", 48.21, 16.37, Europe, 0.4),
    city!("mrs", "Marseille", 43.30, 5.37, Europe, 0.6),
    city!("dub", "Dublin", 53.35, -6.26, Europe, 0.5),
    city!("ist", "Istanbul", 41.01, 28.98, Europe, 0.4),
    // Africa (and the Gulf, which routes like it belongs here more often
    // than not)
    city!("jnb", "Johannesburg", -26.20, 28.05, Africa, 0.6),
    city!("cpt", "Cape Town", -33.92, 18.42, Africa, 0.4),
    city!("los", "Lagos", 6.52, 3.38, Africa, 0.4),
    city!("nbo", "Nairobi", -1.29, 36.82, Africa, 0.4),
    city!("cai", "Cairo", 30.04, 31.24, Africa, 0.4),
    city!("dxb", "Dubai", 25.20, 55.27, Africa, 0.6),
    // Asia
    city!("sin", "Singapore", 1.35, 103.82, Asia, 1.0),
    city!("hkg", "Hong Kong", 22.32, 114.17, Asia, 1.0),
    city!("tyo", "Tokyo", 35.68, 139.69, Asia, 1.0),
    city!("osa", "Osaka", 34.69, 135.50, Asia, 0.5),
    city!("sel", "Seoul", 37.57, 126.98, Asia, 0.7),
    city!("tpe", "Taipei", 25.03, 121.57, Asia, 0.5),
    city!("bom", "Mumbai", 19.08, 72.88, Asia, 0.8),
    city!("maa", "Chennai", 13.08, 80.27, Asia, 0.6),
    city!("del", "Delhi", 28.61, 77.21, Asia, 0.6),
    city!("blr", "Bengaluru", 12.97, 77.59, Asia, 0.5),
    city!("bkk", "Bangkok", 13.76, 100.50, Asia, 0.4),
    city!("cgk", "Jakarta", -6.21, 106.85, Asia, 0.5),
    city!("mnl", "Manila", 14.60, 120.98, Asia, 0.4),
    // Oceania
    city!("syd", "Sydney", -33.87, 151.21, Oceania, 0.8),
    city!("mel", "Melbourne", -37.81, 144.96, Oceania, 0.5),
    city!("per", "Perth", -31.95, 115.86, Oceania, 0.3),
    city!("akl", "Auckland", -36.85, 174.76, Oceania, 0.4),
];

pub fn city(id: CityId) -> &'static City {
    &CITIES[id.idx()]
}

pub fn cities_in(region: Region) -> impl Iterator<Item = CityId> {
    CITIES.iter().enumerate().filter(move |(_, c)| c.region == region).map(|(i, _)| CityId::from_idx(i))
}

const EARTH_RADIUS_KM: f64 = 6371.0;

/// Great-circle distance between two points, in kilometres.
pub fn haversine_km(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
    let dp = (lat2 - lat1).to_radians();
    let dl = (lon2 - lon1).to_radians();
    let a = (dp / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
    2.0 * EARTH_RADIUS_KM * a.sqrt().asin()
}

pub fn city_distance_km(a: CityId, b: CityId) -> f64 {
    let (a, b) = (city(a), city(b));
    haversine_km(a.lat, a.lon, b.lat, b.lon)
}

/// Light in glass does about 204 km per millisecond (refractive index ~1.47).
const FIBRE_KM_PER_SEC: f64 = 204_000.0;

/// Real cable paths are longer than the great circle. 1.25 is a commonly
/// quoted median for terrestrial routes; submarine systems are a bit worse
/// but it is not worth a second constant.
const ROUTE_INFLATION: f64 = 1.25;

/// One-way propagation delay in seconds for a fibre run between two points
/// `km` apart. Never zero: even a cross-connect in the same building has
/// patch panels and optics.
pub fn fibre_delay_s(km: f64) -> f64 {
    (km * ROUTE_INFLATION / FIBRE_KM_PER_SEC).max(20e-6)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(code: &str) -> CityId {
        CityId::from_idx(CITIES.iter().position(|c| c.code == code).unwrap())
    }

    #[test]
    fn codes_are_unique() {
        let mut codes: Vec<_> = CITIES.iter().map(|c| c.code).collect();
        codes.sort_unstable();
        let n = codes.len();
        codes.dedup();
        assert_eq!(codes.len(), n);
    }

    #[test]
    fn transatlantic_is_about_right() {
        let km = city_distance_km(find("nyc"), find("lon"));
        assert!((km - 5570.0).abs() < 30.0, "{km}");
        let ms = fibre_delay_s(km) * 1e3;
        // Published one-way latencies on the big NY-LON systems are 32-37 ms.
        assert!((30.0..40.0).contains(&ms), "{ms}");
    }

    #[test]
    fn every_region_has_cities() {
        for r in Region::ALL {
            assert!(cities_in(r).count() >= 4, "{r:?}");
        }
    }
}
