//! ISO 11783 / NMEA 2000 network-management messages: ISO NAME, address
//! claim, ISO request/acknowledge, product & configuration information, PGN
//! lists and heartbeat.

use crate::bits::{get_bits, set_bits};

pub const PGN_ISO_ACK: u32 = 59392;
pub const PGN_ISO_REQUEST: u32 = 59904;
pub const PGN_ADDRESS_CLAIM: u32 = 60928;
pub const PGN_COMMANDED_ADDRESS: u32 = 65240;
pub const PGN_GROUP_FUNCTION: u32 = 126208;
pub const PGN_PGN_LIST: u32 = 126464;
pub const PGN_HEARTBEAT: u32 = 126993;
pub const PGN_PRODUCT_INFO: u32 = 126996;
pub const PGN_CONFIG_INFO: u32 = 126998;

/// 64-bit ISO NAME.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct IsoName {
    pub unique_number: u32,     // 21 bits
    pub manufacturer_code: u16, // 11 bits
    pub device_instance_lower: u8,
    pub device_instance_upper: u8,
    pub device_function: u8,
    pub device_class: u8, // 7 bits
    pub system_instance: u8,
    pub industry_group: u8,
    pub arbitrary_address_capable: bool,
}

impl IsoName {
    pub fn from_u64(v: u64) -> Self {
        IsoName {
            unique_number: (v & 0x1F_FFFF) as u32,
            manufacturer_code: ((v >> 21) & 0x7FF) as u16,
            device_instance_lower: ((v >> 32) & 0x7) as u8,
            device_instance_upper: ((v >> 35) & 0x1F) as u8,
            device_function: ((v >> 40) & 0xFF) as u8,
            device_class: ((v >> 49) & 0x7F) as u8,
            system_instance: ((v >> 56) & 0xF) as u8,
            industry_group: ((v >> 60) & 0x7) as u8,
            arbitrary_address_capable: (v >> 63) & 1 == 1,
        }
    }

    pub fn to_u64(&self) -> u64 {
        (self.unique_number as u64 & 0x1F_FFFF)
            | (self.manufacturer_code as u64 & 0x7FF) << 21
            | (self.device_instance_lower as u64 & 0x7) << 32
            | (self.device_instance_upper as u64 & 0x1F) << 35
            | (self.device_function as u64) << 40
            | (self.device_class as u64 & 0x7F) << 49
            | (self.system_instance as u64 & 0xF) << 56
            | (self.industry_group as u64 & 0x7) << 60
            | (self.arbitrary_address_capable as u64) << 63
    }

    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        if b.len() < 8 {
            return None;
        }
        Some(Self::from_u64(u64::from_le_bytes(b[..8].try_into().ok()?)))
    }

    pub fn to_bytes(&self) -> [u8; 8] {
        self.to_u64().to_le_bytes()
    }

    pub fn device_instance(&self) -> u8 {
        (self.device_instance_upper << 3) | self.device_instance_lower
    }

    pub fn set_device_instance(&mut self, v: u8) {
        self.device_instance_lower = v & 0x7;
        self.device_instance_upper = v >> 3;
    }

    pub fn hex(&self) -> String {
        format!("{:016X}", self.to_u64())
    }
}

/// Parsed PGN 126996 Product Information.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ProductInfo {
    pub nmea2000_version: Option<u16>, // in 0.001 units (e.g. 2100 = 2.100)
    pub product_code: Option<u16>,
    pub model_id: String,
    pub software_version: String,
    pub model_version: String,
    pub serial_code: String,
    pub certification_level: Option<u8>,
    pub load_equivalency: Option<u8>,
}

/// Decode a fixed-length N2K string (padded with 0xFF, 0x00 or '@').
pub fn string_fix(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0x00 || c == 0xFF).unwrap_or(b.len());
    let s: String = b[..end].iter().map(|&c| c as char).collect();
    s.trim_end_matches(['@', ' ']).to_string()
}

pub fn encode_string_fix(s: &str, len: usize) -> Vec<u8> {
    let mut v: Vec<u8> = s.bytes().take(len).collect();
    v.resize(len, 0xFF);
    v
}

/// Decode a STRING_LAU (length, encoding, chars). Returns (string, bytes consumed).
pub fn string_lau(b: &[u8]) -> Option<(String, usize)> {
    if b.len() < 2 {
        return None;
    }
    let len = b[0] as usize;
    if len < 2 {
        // Some devices send len 0/1 for empty; consume the header only.
        return Some((String::new(), 2.min(b.len())));
    }
    if len > b.len() {
        return None;
    }
    let body = &b[2..len];
    let s = if b[1] == 0 {
        let units: Vec<u16> = body.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        String::from_utf16_lossy(&units).trim_end_matches('\0').to_string()
    } else {
        body.iter().take_while(|&&c| c != 0).map(|&c| c as char).collect()
    };
    Some((s, len))
}

pub fn encode_string_lau(s: &str) -> Vec<u8> {
    let ascii = s.is_ascii();
    let mut v = vec![0u8, if ascii { 1 } else { 0 }];
    if ascii {
        v.extend(s.bytes().take(250));
    } else {
        for u in s.encode_utf16().take(125) {
            v.extend_from_slice(&u.to_le_bytes());
        }
    }
    v[0] = v.len() as u8;
    v
}

fn opt_u16(b: &[u8], off: usize) -> Option<u16> {
    let v = u16::from_le_bytes([*b.get(off)?, *b.get(off + 1)?]);
    if v >= 0xFFFD {
        None
    } else {
        Some(v)
    }
}

fn opt_u8(b: &[u8], off: usize) -> Option<u8> {
    let v = *b.get(off)?;
    if v >= 0xFD {
        None
    } else {
        Some(v)
    }
}

impl ProductInfo {
    pub fn parse(b: &[u8]) -> Option<Self> {
        if b.len() < 4 + 32 {
            return None;
        }
        let s = |off: usize| b.get(off..off + 32).map(string_fix).unwrap_or_default();
        Some(ProductInfo {
            nmea2000_version: opt_u16(b, 0),
            product_code: opt_u16(b, 2),
            model_id: s(4),
            software_version: s(36),
            model_version: s(68),
            serial_code: s(100),
            certification_level: opt_u8(b, 132),
            load_equivalency: opt_u8(b, 133),
        })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(134);
        v.extend_from_slice(&self.nmea2000_version.unwrap_or(0xFFFF).to_le_bytes());
        v.extend_from_slice(&self.product_code.unwrap_or(0xFFFF).to_le_bytes());
        for s in [&self.model_id, &self.software_version, &self.model_version, &self.serial_code] {
            v.extend(encode_string_fix(s, 32));
        }
        v.push(self.certification_level.unwrap_or(0xFF));
        v.push(self.load_equivalency.unwrap_or(0xFF));
        v
    }
}

/// Parsed PGN 126998 Configuration Information.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ConfigInfo {
    pub installation_description1: String,
    pub installation_description2: String,
    pub manufacturer_information: String,
}

impl ConfigInfo {
    pub fn parse(b: &[u8]) -> Option<Self> {
        let (a, n1) = string_lau(b)?;
        let (c, n2) = string_lau(&b[n1..]).unwrap_or_default();
        let (m, _) = string_lau(&b[(n1 + n2).min(b.len())..]).unwrap_or_default();
        Some(ConfigInfo { installation_description1: a, installation_description2: c, manufacturer_information: m })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut v = encode_string_lau(&self.installation_description1);
        v.extend(encode_string_lau(&self.installation_description2));
        v.extend(encode_string_lau(&self.manufacturer_information));
        v
    }
}

/// PGN 126464: function 0 = transmit list, 1 = receive list.
pub fn parse_pgn_list(b: &[u8]) -> Option<(u8, Vec<u32>)> {
    let f = *b.first()?;
    let list = b[1..]
        .chunks_exact(3)
        .map(|c| c[0] as u32 | (c[1] as u32) << 8 | (c[2] as u32) << 16)
        .filter(|&p| p != 0xFF_FFFF)
        .collect();
    Some((f, list))
}

pub fn encode_pgn_list(function: u8, pgns: &[u32]) -> Vec<u8> {
    let mut v = vec![function];
    for p in pgns {
        v.extend_from_slice(&p.to_le_bytes()[..3]);
    }
    v
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Heartbeat {
    pub interval_ms: Option<u16>,
    pub sequence: u8,
}

impl Heartbeat {
    pub fn parse(b: &[u8]) -> Option<Self> {
        Some(Heartbeat { interval_ms: opt_u16(b, 0), sequence: *b.get(2)? })
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut v = self.interval_ms.unwrap_or(0xFFFF).to_le_bytes().to_vec();
        v.push(self.sequence);
        v.extend_from_slice(&[0xFF; 5]);
        v
    }
}

pub fn iso_request(pgn: u32) -> Vec<u8> {
    pgn.to_le_bytes()[..3].to_vec()
}

pub fn parse_iso_request(b: &[u8]) -> Option<u32> {
    if b.len() < 3 {
        return None;
    }
    Some(b[0] as u32 | (b[1] as u32) << 8 | (b[2] as u32) << 16)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum AckControl {
    Ack,
    Nak,
    AccessDenied,
    CannotRespond,
    Other(u8),
}

pub fn parse_iso_ack(b: &[u8]) -> Option<(AckControl, u32)> {
    if b.len() < 8 {
        return None;
    }
    let c = match b[0] {
        0 => AckControl::Ack,
        1 => AckControl::Nak,
        2 => AckControl::AccessDenied,
        3 => AckControl::CannotRespond,
        x => AckControl::Other(x),
    };
    Some((c, get_bits(b, 40, 24)? as u32))
}

pub fn iso_ack(control: u8, pgn: u32) -> Vec<u8> {
    let mut v = vec![control, 0xFF, 0xFF, 0xFF, 0xFF];
    v.extend_from_slice(&pgn.to_le_bytes()[..3]);
    v
}

pub fn commanded_address(name: &IsoName, new_address: u8) -> Vec<u8> {
    let mut v = name.to_bytes().to_vec();
    v.push(new_address);
    v
}

/// Build a 127502 Switch Bank Control payload: all switches "no change"
/// except the given (indicator 1..=28, on).
pub fn switch_bank_control(instance: u8, changes: &[(u8, bool)]) -> Vec<u8> {
    let mut v = vec![instance, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];
    for &(ind, on) in changes {
        if (1..=28).contains(&ind) {
            set_bits(&mut v, 8 + (ind as usize - 1) * 2, 2, on as u64);
        }
    }
    v
}

/// Parse 127501 / 127502: (instance, [state; 28]) where state 0=off 1=on 2=error 3=n/a.
pub fn parse_switch_bank(b: &[u8]) -> Option<(u8, [u8; 28])> {
    let inst = *b.first()?;
    let mut st = [3u8; 28];
    for (i, s) in st.iter_mut().enumerate() {
        *s = get_bits(b, 8 + i * 2, 2).unwrap_or(3) as u8;
    }
    Some((inst, st))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_roundtrip() {
        let n = IsoName {
            unique_number: 0x12345,
            manufacturer_code: 137,
            device_instance_lower: 5,
            device_instance_upper: 3,
            device_function: 130,
            device_class: 25,
            system_instance: 2,
            industry_group: 4,
            arbitrary_address_capable: true,
        };
        let v = n.to_u64();
        assert_eq!(IsoName::from_u64(v), n);
        assert_eq!(n.device_instance(), 29);
        assert_eq!(IsoName::from_bytes(&n.to_bytes()).unwrap(), n);
        // Cross-check bit placement with the generic bit reader.
        let b = n.to_bytes();
        assert_eq!(get_bits(&b, 21, 11), Some(137));
        assert_eq!(get_bits(&b, 49, 7), Some(25));
    }

    #[test]
    fn product_info_roundtrip() {
        let p = ProductInfo {
            nmea2000_version: Some(2100),
            product_code: Some(12337),
            model_id: "CLMD16".into(),
            software_version: "1.0.5".into(),
            model_version: "Rev A".into(),
            serial_code: "1234567".into(),
            certification_level: Some(1),
            load_equivalency: Some(1),
        };
        let e = p.encode();
        assert_eq!(e.len(), 134);
        assert_eq!(ProductInfo::parse(&e).unwrap(), p);
    }

    #[test]
    fn config_info_roundtrip() {
        let c = ConfigInfo {
            installation_description1: "Engine room".into(),
            installation_description2: "".into(),
            manufacturer_information: "Maretron".into(),
        };
        assert_eq!(ConfigInfo::parse(&c.encode()).unwrap(), c);
        let u = encode_string_lau("Café");
        assert_eq!(u[1], 0);
        assert_eq!(string_lau(&u).unwrap().0, "Café");
    }

    #[test]
    fn switch_bank() {
        let v = switch_bank_control(3, &[(1, true), (5, false), (28, true)]);
        let (inst, st) = parse_switch_bank(&v).unwrap();
        assert_eq!(inst, 3);
        assert_eq!(st[0], 1);
        assert_eq!(st[4], 0);
        assert_eq!(st[27], 1);
        assert_eq!(st[1], 3);
    }

    #[test]
    fn pgn_list() {
        let e = encode_pgn_list(0, &[60928, 126996, 127501]);
        assert_eq!(parse_pgn_list(&e).unwrap(), (0, vec![60928, 126996, 127501]));
    }
}
