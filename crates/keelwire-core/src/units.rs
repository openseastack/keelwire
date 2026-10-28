//! Display units (Settings → Units). Values on the bus are SI;
//! this only changes how they are shown.

use std::sync::RwLock;

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize), serde(default))]
pub struct Units {
    /// Depth, altitude and other short lengths: "m", "ft", "fathom".
    pub depth: String,
    /// Logs and distances: "nm", "km", "mi".
    pub distance: String,
    /// "C" or "F".
    pub temperature: String,
    /// "kn", "km/h", "mph", "m/s".
    pub speed: String,
    /// "L", "gal" (US), "impgal".
    pub volume: String,
    /// "kPa", "bar", "psi", "inHg" (barometric pressure uses "hPa" when kPa is chosen).
    pub pressure: String,
}

impl Default for Units {
    fn default() -> Self {
        Self::metric()
    }
}

impl Units {
    pub fn metric() -> Self {
        Units { depth: "m".into(), distance: "nm".into(), temperature: "C".into(), speed: "kn".into(), volume: "L".into(), pressure: "kPa".into() }
    }
    pub fn us() -> Self {
        Units { depth: "ft".into(), distance: "nm".into(), temperature: "F".into(), speed: "kn".into(), volume: "gal".into(), pressure: "psi".into() }
    }
}

static CURRENT: RwLock<Option<Units>> = RwLock::new(None);

pub fn current() -> Units {
    CURRENT.read().ok().and_then(|g| g.clone()).unwrap_or_default()
}

pub fn set(u: Units) {
    if let Ok(mut g) = CURRENT.write() {
        *g = Some(u);
    }
}

/// Format an SI value with its CANboat unit in the chosen display units.
/// `hint` is the field id, used to tell distances from depths.
/// Returns None when the unit has no conversion (caller renders raw).
pub fn format(v: f64, unit: &str, hint: &str, u: &Units) -> Option<String> {
    let h = hint.to_ascii_lowercase();
    Some(match unit {
        "rad" => format!("{:.1}°", v.to_degrees()),
        "rad/s" => format!("{:.1}°/min", v.to_degrees() * 60.0),
        "K" => match u.temperature.as_str() {
            "F" => format!("{:.1} °F", (v - 273.15) * 9.0 / 5.0 + 32.0),
            _ => format!("{:.1} °C", v - 273.15),
        },
        "m/s" => match u.speed.as_str() {
            "km/h" => format!("{:.1} km/h", v * 3.6),
            "mph" => format!("{:.1} mph", v * 2.236_936),
            "m/s" => format!("{:.2} m/s", v),
            _ => format!("{:.2} kn", v * 1.943_844),
        },
        "m" if h.contains("log") || h.contains("distance") || h.contains("range") || v.abs() >= 20_000.0 => match u.distance.as_str() {
            "km" => format!("{:.2} km", v / 1000.0),
            "mi" => format!("{:.2} mi", v / 1609.344),
            _ => format!("{:.2} nm", v / 1852.0),
        },
        "m" => match u.depth.as_str() {
            "ft" => format!("{:.1} ft", v * 3.280_84),
            "fathom" => format!("{:.2} fathom", v / 1.8288),
            _ => format!("{:.2} m", v),
        },
        "L" => match u.volume.as_str() {
            "gal" => format!("{:.1} gal", v / 3.785_412),
            "impgal" => format!("{:.1} imp gal", v / 4.546_09),
            _ => format!("{:.1} L", v),
        },
        "L/h" => match u.volume.as_str() {
            "gal" => format!("{:.2} gal/h", v / 3.785_412),
            "impgal" => format!("{:.2} imp gal/h", v / 4.546_09),
            _ => format!("{:.1} L/h", v),
        },
        "Pa" => {
            let baro = (80_000.0..=110_000.0).contains(&v) && (h.contains("atmospheric") || h.contains("pressure") && !h.contains("oil") && !h.contains("fuel") && !h.contains("boost") && !h.contains("coolant"));
            match u.pressure.as_str() {
                "psi" => format!("{:.1} psi", v / 6_894.757),
                "bar" => format!("{:.3} bar", v / 100_000.0),
                "inHg" => format!("{:.2} inHg", v / 3_386.389),
                _ if baro => format!("{:.1} hPa", v / 100.0),
                _ if v >= 10_000.0 => format!("{:.1} kPa", v / 1000.0),
                _ => format!("{:.0} Pa", v),
            }
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn conversions() {
        let us = Units::us();
        assert_eq!(format(10.0, "m", "depth", &us).unwrap(), "32.8 ft");
        assert_eq!(format(293.15, "K", "temperature", &us).unwrap(), "68.0 °F");
        assert_eq!(format(378.5412, "L", "capacity", &us).unwrap(), "100.0 gal");
        assert_eq!(format(37_040.0, "m", "log", &Units::metric()).unwrap(), "20.00 nm");
        assert_eq!(format(101_325.0, "Pa", "pressure", &Units::metric()).unwrap(), "1013.2 hPa");
        assert_eq!(format(400_000.0, "Pa", "oilPressure", &us).unwrap(), "58.0 psi");
    }
}
