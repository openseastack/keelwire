//! Named Maretron device actions built from publicly documented / community
//! observed commands (mbj4668/n2k, MIT; Maretron SSC300 user's manual).
//!
//! `experimental` marks actions whose byte layout is ambiguous between the
//! available sources (the two sources disagree on whether the rate-of-turn
//! sub-command is 8 or 16 bits). "Get" requests are identical under both
//! readings; "set" requests are not, so those need expert mode.

use keelwire_core::proprietary::maretron::{command, rot};
use serde::Serialize;

const SSC: &[u16] = &[434, 2686]; // SSC200, SSC300
const TLM: &[u16] = &[27725, 2781]; // TLM100, TLA100

#[derive(Clone, Copy, Debug, Serialize)]
pub struct ActionInfo {
    pub id: &'static str,
    pub group: &'static str,
    pub label: &'static str,
    pub help: &'static str,
    /// Changes device state: requires TX to be armed.
    pub control: bool,
    /// Layout not confirmed on hardware: requires expert mode.
    pub experimental: bool,
    /// Label of the numeric input, if any.
    pub value: Option<&'static str>,
    pub value2: Option<&'static str>,
    #[serde(skip)]
    pub products: &'static [u16],
}

pub const ACTIONS: &[ActionInfo] = &[
    ActionInfo { id: "compass.get_variation_source", group: "Variation", label: "Read variation source", help: "Where the compass takes magnetic variation from.", control: false, experimental: false, value: None, value2: None, products: SSC },
    ActionInfo { id: "compass.set_variation_source", group: "Variation", label: "Set variation source", help: "0 = manual, 1 = NMEA 2000, 2 = NMEA 0183, 3 = NMEA 2000 or 0183.", control: true, experimental: false, value: Some("Source (0–3)"), value2: None, products: SSC },
    ActionInfo { id: "compass.get_rot_damping", group: "Rate of turn", label: "Read rate-of-turn damping", help: "Damping period applied to rate of turn.", control: false, experimental: false, value: None, value2: None, products: SSC },
    ActionInfo { id: "compass.set_rot_damping", group: "Rate of turn", label: "Set rate-of-turn damping", help: "0.1 to 60 seconds.", control: true, experimental: true, value: Some("Seconds"), value2: None, products: SSC },
    ActionInfo { id: "compass.start_rot_zeroing", group: "Rate of turn", label: "Start rate-of-turn zeroing", help: "The boat must stay at rest for one minute afterwards.", control: true, experimental: true, value: None, value2: None, products: SSC },
    ActionInfo { id: "compass.cancel_rot_zeroing", group: "Rate of turn", label: "Cancel rate-of-turn zeroing", help: "", control: true, experimental: true, value: None, value2: None, products: SSC },
    ActionInfo { id: "compass.get_installation", group: "Installation", label: "Read installation orientation", help: "Orientation (normal / upside down) and direction (normal / backward).", control: false, experimental: false, value: None, value2: None, products: SSC },
    ActionInfo { id: "compass.set_installation", group: "Installation", label: "Set installation orientation", help: "Orientation 1 = normal, 2 = upside down; direction 1 = normal, 2 = backward.", control: true, experimental: true, value: Some("Orientation (1–2)"), value2: Some("Direction (1–2)"), products: SSC },
    ActionInfo { id: "compass.installation_offset", group: "Installation", label: "Calibrate installation offset", help: "Zeroes pitch and roll at the current attitude and sets the current heading to the value entered.", control: true, experimental: true, value: Some("Heading (°)"), value2: None, products: SSC },
    ActionInfo { id: "compass.start_calibration", group: "Deviation calibration", label: "Start deviation calibration", help: "Resets the compass and starts deviation calibration: turn the boat in slow circles until it reports completion.", control: true, experimental: false, value: None, value2: None, products: SSC },
    ActionInfo { id: "compass.resend_calibration_status", group: "Deviation calibration", label: "Read calibration status", help: "Asks the compass to repeat its last deviation-calibration status.", control: false, experimental: false, value: None, value2: None, products: SSC },
    ActionInfo { id: "tank.get_level_config", group: "Tank calibration", label: "Read level configuration", help: "Asks for the tank calibration table; the request's data layout is not known yet.", control: false, experimental: true, value: Some("Row (optional)"), value2: None, products: TLM },
];

pub fn for_product(product_code: u16) -> Vec<ActionInfo> {
    ACTIONS.iter().copied().filter(|a| a.products.contains(&product_code)).collect()
}

pub fn find(id: &str, product_code: u16) -> Option<ActionInfo> {
    ACTIONS.iter().copied().find(|a| a.id == id && a.products.contains(&product_code))
}

fn need(v: Option<f64>, what: &str) -> Result<f64, String> {
    v.ok_or_else(|| format!("{what} is required"))
}

/// (command, data) for an action.
pub fn encode(id: &str, value: Option<f64>, value2: Option<f64>) -> Result<(u8, Vec<u8>), String> {
    let le = |x: u16| x.to_le_bytes().to_vec();
    Ok(match id {
        "compass.get_variation_source" => (command::COMPASS_VARIATION_SOURCE, vec![0xFF]),
        "compass.set_variation_source" => {
            let v = need(value, "source")?;
            if !(0.0..=3.0).contains(&v) || v.fract() != 0.0 {
                return Err("source must be 0, 1, 2 or 3".into());
            }
            (command::COMPASS_VARIATION_SOURCE, vec![v as u8])
        }
        "compass.get_rot_damping" => (command::COMPASS_ROT_CONFIG, le(rot::GET_DAMPING)),
        "compass.set_rot_damping" => {
            let s = need(value, "damping period")?;
            if !(0.1..=60.0).contains(&s) {
                return Err("damping must be 0.1 to 60 seconds".into());
            }
            let mut d = le(rot::SET_DAMPING);
            d.extend(le((s * 1000.0).round() as u16));
            (command::COMPASS_ROT_CONFIG, d)
        }
        "compass.start_rot_zeroing" => (command::COMPASS_ROT_CONFIG, le(rot::START_ZEROING)),
        "compass.cancel_rot_zeroing" => (command::COMPASS_ROT_CONFIG, le(rot::CANCEL_ZEROING)),
        "compass.get_installation" => (command::COMPASS_ROT_CONFIG, le(rot::GET_INSTALLATION)),
        "compass.set_installation" => {
            let o = need(value, "orientation")?;
            let d = need(value2, "direction")?;
            if ![1.0, 2.0].contains(&o) || ![1.0, 2.0].contains(&d) {
                return Err("orientation and direction must each be 1 or 2".into());
            }
            let mut v = le(rot::SET_INSTALLATION);
            v.extend(le(o as u16));
            v.extend(le(d as u16));
            (command::COMPASS_ROT_CONFIG, v)
        }
        "compass.installation_offset" => {
            let h = need(value, "heading")?;
            if !(0.0..360.0).contains(&h) {
                return Err("heading must be 0 to 359.9 degrees".into());
            }
            (command::COMPASS_INSTALLATION_OFFSET, le((h * 10.0).round() as u16))
        }
        "compass.start_calibration" => (command::COMPASS_RESET_START_CALIBRATION, vec![]),
        "compass.resend_calibration_status" => (command::COMPASS_RESEND_CALIBRATION_STATUS, vec![]),
        "tank.get_level_config" => (command::TANK_GET_LEVEL_CONFIG, value.map(|r| vec![r as u8]).unwrap_or_default()),
        _ => return Err(format!("unknown action '{id}'")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_action_encodes() {
        for a in ACTIONS {
            let r = encode(a.id, Some(1.0), Some(1.0));
            assert!(r.is_ok(), "{}: {:?}", a.id, r);
        }
        assert_eq!(encode("compass.get_rot_damping", None, None).unwrap(), (94, vec![1, 0]));
        assert_eq!(encode("compass.set_rot_damping", Some(1.5), None).unwrap(), (94, vec![2, 0, 0xDC, 0x05]));
        assert!(encode("compass.set_variation_source", Some(7.0), None).is_err());
        assert_eq!(for_product(2686).len(), 11);
        assert!(find("compass.get_installation", 27725).is_none());
    }
}
