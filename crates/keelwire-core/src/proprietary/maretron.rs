//! Maretron / Carling proprietary messages.
//!
//! Public, verified knowledge (from the CANboat project, Apache-2.0):
//!  * 126720 "Proprietary Configuration": header · product code (16) ·
//!    software code (16) · opcode (8) · payload.
//!  * Opcode table, product codes, label PGN 130818, breaker current 65284,
//!    switch indicator status 130826, number of channels 65282.
//!
//! How requests travel (observed on the wire by the mbj4668/n2k project, MIT,
//! and consistent with Maretron's public SSC300 manual): a request is a
//! PGN 126208 *Command* addressed to the device, commanding PGN 126720 with
//! parameter 1 = Maretron header (0x9889), 2 = product code, 3 = software
//! code, followed directly by the Maretron command byte and its data (the
//! 4th "parameter" has no index byte). The device answers with a 126720
//! "slave response": header · product code · software code · command · data,
//! and/or a 126208 Acknowledge.
//!
//! Per-model payload layouts are described by schema files (see `schema.rs`).

use super::ProprietaryHeader;
use crate::iso::{encode_string_lau, string_lau};

pub const MANUFACTURER_CODE: u16 = 137;
pub const INDUSTRY_MARINE: u8 = 4;
pub const PGN_CONFIG: u32 = 126720;
pub const PGN_LABEL: u32 = 130818;
pub const PGN_NUMBER_OF_CHANNELS: u32 = 65282;
pub const PGN_BREAKER_CURRENT: u32 = 65284;
pub const PGN_SWITCH_INDICATOR_STATUS: u32 = 130826;
pub const PGN_DATA_INSTANCE_CORRELATION: u32 = 130825;

/// Commands observed on the wire (mbj4668/n2k; SSC300 user's manual).
pub mod command {
    pub const TANK_SET_LEVEL_CONFIG: u8 = 33;
    pub const COMPASS_INSTALLATION_OFFSET: u8 = 36;
    pub const TANK_GET_LEVEL_CONFIG: u8 = 49;
    pub const COMPASS_RESEND_CALIBRATION_STATUS: u8 = 80;
    pub const COMPASS_VARIATION_SOURCE: u8 = 93;
    pub const COMPASS_ROT_CONFIG: u8 = 94;
    pub const COMPASS_RESET_START_CALIBRATION: u8 = 240;
    pub const INFORMATION: u8 = 252;
}

/// Sub-commands of COMPASS_ROT_CONFIG (16-bit little-endian in traced responses).
pub mod rot {
    pub const GET_DAMPING: u16 = 1;
    pub const SET_DAMPING: u16 = 2;
    pub const START_ZEROING: u16 = 17;
    pub const CANCEL_ZEROING: u16 = 18;
    pub const GET_INSTALLATION: u16 = 513;
    pub const SET_INSTALLATION: u16 = 514;
}

pub const COMMANDS: &[(u8, &str)] = &[
    (33, "Tank: set level configuration"),
    (36, "Compass: installation offset"),
    (49, "Tank: get level configuration"),
    (80, "Compass: resend calibration status"),
    (93, "Compass: variation source (0xFF = get)"),
    (94, "Compass: rate of turn / installation configuration"),
    (240, "Compass: reset and start deviation calibration"),
];

/// Encode a Maretron request as the PGN 126208 payload to send to the device.
pub fn command_payload(product_code: u16, software_code: u16, command: u8, data: &[u8]) -> Vec<u8> {
    let mut v = vec![1u8]; // Command group function
    v.extend_from_slice(&PGN_CONFIG.to_le_bytes()[..3]);
    v.push(0xF8); // priority: no change
    v.push(4); // parameter count as sent by Maretron tools
    v.push(1);
    v.extend_from_slice(&ProprietaryHeader::MARETRON.encode());
    v.push(2);
    v.extend_from_slice(&product_code.to_le_bytes());
    v.push(3);
    v.extend_from_slice(&software_code.to_le_bytes());
    v.push(command);
    v.extend_from_slice(data);
    v
}

/// Decode a 126208 payload built by [`command_payload`]: (product code, software code, command, data).
pub fn parse_command_payload(b: &[u8]) -> Option<(u16, u16, u8, Vec<u8>)> {
    if b.len() < 16 || b[0] != 1 || (b[1] as u32 | (b[2] as u32) << 8 | (b[3] as u32) << 16) != PGN_CONFIG {
        return None;
    }
    if b[6] != 1 || b[7..9] != ProprietaryHeader::MARETRON.encode() || b[9] != 2 || b[12] != 3 {
        return None;
    }
    Some((u16::from_le_bytes([b[10], b[11]]), u16::from_le_bytes([b[13], b[14]]), b[15], b[16..].to_vec()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[repr(u8)]
pub enum Opcode {
    ReadAll = 0,
    WriteRegister = 1,
    ReadConfig = 2,
    WriteConfig = 3,
    Calibrate = 4,
    ClearCalibration = 5,
    Status = 6,
    ClearStatus = 7,
    ResetFactoryDefault = 8,
    Debug = 9,
    WriteInstance = 16,
    ReadInstance = 17,
    WriteLabel = 32,
    ReadLabel = 33,
    WriteSwitchConfig = 48,
    ReadSwitchConfig = 49,
    WriteAlertConfig = 64,
    ReadAlertConfig = 65,
    WriteChannelConfig = 80,
    ReadChannelConfig = 81,
    ReadChannelConfigExtended = 86,
    WriteChannelConfigExtended = 87,
}

impl Opcode {
    pub fn from_u8(v: u8) -> Option<Self> {
        use Opcode::*;
        Some(match v {
            0 => ReadAll,
            1 => WriteRegister,
            2 => ReadConfig,
            3 => WriteConfig,
            4 => Calibrate,
            5 => ClearCalibration,
            6 => Status,
            7 => ClearStatus,
            8 => ResetFactoryDefault,
            9 => Debug,
            16 => WriteInstance,
            17 => ReadInstance,
            32 => WriteLabel,
            33 => ReadLabel,
            48 => WriteSwitchConfig,
            49 => ReadSwitchConfig,
            64 => WriteAlertConfig,
            65 => ReadAlertConfig,
            80 => WriteChannelConfig,
            81 => ReadChannelConfig,
            86 => ReadChannelConfigExtended,
            87 => WriteChannelConfigExtended,
            _ => return None,
        })
    }

    pub fn is_write(&self) -> bool {
        use Opcode::*;
        matches!(
            self,
            WriteRegister | WriteConfig | Calibrate | ClearCalibration | ClearStatus | ResetFactoryDefault | WriteInstance | WriteLabel | WriteSwitchConfig | WriteAlertConfig | WriteChannelConfig | WriteChannelConfigExtended
        )
    }
}

/// Maretron product codes (NMEA product code in 126996).
pub const PRODUCT_CODES: &[(u16, &str)] = &[
    (434, "SSC200"),
    (1047, "SMS100"),
    (1151, "MBB200C"),
    (1534, "DST110"),
    (1776, "GPS100"),
    (2606, "CLM100"),
    (2686, "SSC300"),
    (2781, "TLA100"),
    (3373, "GPS200"),
    (3563, "DST100"),
    (3637, "FFM100"),
    (3979, "NBE100"),
    (4018, "RAA100"),
    (4078, "RIM100"),
    (4319, "J2K100"),
    (8165, "ALM100"),
    (9339, "IPG100"),
    (9375, "DCM100"),
    (9845, "EMS100"),
    (12337, "CLMD16"),
    (16434, "DSM250"),
    (20067, "TMP100"),
    (20298, "DSM150"),
    (21703, "FPM100"),
    (22585, "DCR100"),
    (23603, "SIM100"),
    (26493, "ACM100"),
    (27244, "MBB300C"),
    (27725, "TLM100"),
    (28077, "MConnect"),
];

pub fn product_name(code: u16) -> Option<&'static str> {
    PRODUCT_CODES.iter().find(|(c, _)| *c == code).map(|(_, n)| *n)
}

/// 126720 Proprietary Configuration message.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ConfigMessage {
    pub product_code: u16,
    pub software_code: u16,
    pub opcode: u8,
    pub payload: Vec<u8>,
}

impl ConfigMessage {
    pub fn parse(b: &[u8]) -> Option<Self> {
        let h = ProprietaryHeader::decode(b)?;
        if h.manufacturer_code != MANUFACTURER_CODE || b.len() < 7 {
            return None;
        }
        Some(ConfigMessage {
            product_code: u16::from_le_bytes([b[2], b[3]]),
            software_code: u16::from_le_bytes([b[4], b[5]]),
            opcode: b[6],
            payload: b[7..].to_vec(),
        })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut v = ProprietaryHeader::MARETRON.encode().to_vec();
        v.extend_from_slice(&self.product_code.to_le_bytes());
        v.extend_from_slice(&self.software_code.to_le_bytes());
        v.push(self.opcode);
        v.extend_from_slice(&self.payload);
        v
    }

    pub fn opcode(&self) -> Option<Opcode> {
        Opcode::from_u8(self.opcode)
    }
}

/// 130818 Label.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Label {
    pub instance: u8,
    pub data_source: u8,
    pub data_indicator: u8,
    pub label: String,
    pub hardware_channel: Option<u8>,
}

impl Label {
    pub fn parse(b: &[u8]) -> Option<Self> {
        let h = ProprietaryHeader::decode(b)?;
        if h.manufacturer_code != MANUFACTURER_CODE || b.len() < 7 {
            return None;
        }
        let (label, n) = string_lau(&b[5..])?;
        Some(Label { instance: b[2], data_source: b[3], data_indicator: b[4], label, hardware_channel: b.get(5 + n).copied() })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut v = ProprietaryHeader::MARETRON.encode().to_vec();
        v.extend_from_slice(&[self.instance, self.data_source, self.data_indicator]);
        v.extend(encode_string_lau(&self.label));
        if let Some(h) = self.hardware_channel {
            v.push(h);
        }
        v
    }
}

/// 65284 Proprietary DC Breaker Current: (bank instance, indicator, amps).
pub fn parse_breaker_current(b: &[u8]) -> Option<(u8, u8, Option<f64>)> {
    let h = ProprietaryHeader::decode(b)?;
    if h.manufacturer_code != MANUFACTURER_CODE || b.len() < 6 {
        return None;
    }
    let raw = u16::from_le_bytes([b[4], b[5]]);
    let amps = if raw >= 0xFFFD { None } else { Some(raw as f64 / 10.0) };
    Some((b[2], b[3], amps))
}

/// 130826 Switch Indicator Status: (bank instance, statuses 0 off/1 on/2 tripped/3 unknown).
pub fn parse_switch_indicator_status(b: &[u8]) -> Option<(u8, Vec<u8>)> {
    let h = ProprietaryHeader::decode(b)?;
    if h.manufacturer_code != MANUFACTURER_CODE || b.len() < 4 {
        return None;
    }
    let n = b[3] as usize;
    Some((b[2], b.iter().skip(4).take(n).copied().collect()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_roundtrip() {
        let m = ConfigMessage { product_code: 12337, software_code: 1, opcode: Opcode::ReadConfig as u8, payload: vec![1, 2, 3] };
        let e = m.encode();
        assert_eq!(&e[..2], &[0x89, 0x98]);
        assert_eq!(ConfigMessage::parse(&e).unwrap(), m);
        assert_eq!(m.opcode(), Some(Opcode::ReadConfig));
        assert_eq!(product_name(12337), Some("CLMD16"));
    }

    #[test]
    fn command_payload_matches_observed_bytes() {
        // mbj4668/n2k compass_set_rate_of_turn_damping layout: 01 | 126720 LE | F8 | 04 |
        // 01 89 98 | 02 <SSC300 2686 LE> | 03 01 00 | command | data
        let p = command_payload(2686, 1, command::COMPASS_VARIATION_SOURCE, &[0xFF]);
        assert_eq!(p, vec![0x01, 0x00, 0xEF, 0x01, 0xF8, 0x04, 0x01, 0x89, 0x98, 0x02, 0x7E, 0x0A, 0x03, 0x01, 0x00, 93, 0xFF]);
        assert_eq!(parse_command_payload(&p), Some((2686, 1, 93, vec![0xFF])));
    }

    #[test]
    fn label_roundtrip() {
        let l = Label { instance: 3, data_source: 0, data_indicator: 5, label: "RUNNING LIGHTS".into(), hardware_channel: Some(4) };
        assert_eq!(Label::parse(&l.encode()).unwrap(), l);
    }

    #[test]
    fn breaker_current() {
        let mut b = ProprietaryHeader::MARETRON.encode().to_vec();
        b.extend_from_slice(&[2, 7, 0x7B, 0x00, 0xFF, 0xFF]);
        assert_eq!(parse_breaker_current(&b), Some((2, 7, Some(12.3))));
    }
}
