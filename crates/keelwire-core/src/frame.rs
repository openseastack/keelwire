//! CAN frames, 29-bit identifier decomposition and N2K messages.

pub const BROADCAST: u8 = 0xFF;
pub const NULL_ADDRESS: u8 = 0xFE;

/// A single raw CAN frame as received from (or sent to) the bus.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RawCanFrame {
    /// Microseconds since engine start (or capture start for replays).
    pub timestamp_us: u64,
    pub can_id: u32,
    pub is_extended: bool,
    pub dlc: u8,
    pub data: [u8; 8],
}

impl RawCanFrame {
    pub fn new(timestamp_us: u64, can_id: u32, payload: &[u8]) -> Self {
        let mut data = [0xFFu8; 8];
        let n = payload.len().min(8);
        data[..n].copy_from_slice(&payload[..n]);
        RawCanFrame { timestamp_us, can_id: can_id & 0x1FFF_FFFF, is_extended: true, dlc: n as u8, data }
    }

    pub fn payload(&self) -> &[u8] {
        &self.data[..(self.dlc as usize).min(8)]
    }

    pub fn id(&self) -> CanId {
        CanId::decode(self.can_id)
    }
}

/// Decomposed 29-bit J1939 / NMEA 2000 identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CanId {
    pub priority: u8,
    pub pgn: u32,
    pub destination: u8,
    pub source: u8,
}

/// True when `pgn` is a PDU1 (destination-addressed) PGN, i.e. PF < 240.
pub fn is_pdu1(pgn: u32) -> bool {
    ((pgn >> 8) & 0xFF) < 0xF0
}

impl CanId {
    pub fn decode(id: u32) -> Self {
        let id = id & 0x1FFF_FFFF;
        let source = (id & 0xFF) as u8;
        let ps = ((id >> 8) & 0xFF) as u8;
        let pf = (id >> 16) & 0xFF;
        let dp_edp = (id >> 24) & 0x03;
        let priority = ((id >> 26) & 0x07) as u8;
        let (pgn, destination) = if pf < 0xF0 {
            ((dp_edp << 16) | (pf << 8), ps)
        } else {
            ((dp_edp << 16) | (pf << 8) | ps as u32, BROADCAST)
        };
        CanId { priority, pgn, destination, source }
    }

    pub fn encode(&self) -> u32 {
        let pf = (self.pgn >> 8) & 0xFF;
        let dp_edp = (self.pgn >> 16) & 0x03;
        let ps = if pf < 0xF0 { self.destination as u32 } else { self.pgn & 0xFF };
        ((self.priority as u32 & 0x07) << 26) | (dp_edp << 24) | (pf << 16) | (ps << 8) | self.source as u32
    }
}

/// A complete N2K message (single frame, reassembled fast-packet or ISO TP).
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct N2kMessage {
    pub timestamp_us: u64,
    pub priority: u8,
    pub pgn: u32,
    pub source: u8,
    pub destination: u8,
    pub data: Vec<u8>,
}

impl N2kMessage {
    pub fn new(priority: u8, pgn: u32, source: u8, destination: u8, data: Vec<u8>) -> Self {
        N2kMessage { timestamp_us: 0, priority, pgn, source, destination, data }
    }

    pub fn can_id(&self) -> CanId {
        CanId { priority: self.priority, pgn: self.pgn, destination: self.destination, source: self.source }
    }
}

pub fn hex(data: &[u8]) -> String {
    let mut s = String::with_capacity(data.len() * 2);
    for b in data {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

pub fn parse_hex(s: &str) -> Option<Vec<u8>> {
    let clean: String = s.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if !clean.len().is_multiple_of(2) {
        return None;
    }
    (0..clean.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&clean[i..i + 2], 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdu2_broadcast() {
        // 0x09F80117: prio 2, PGN 129025 (0x1F801), src 0x17
        let id = CanId::decode(0x09F8_0117);
        assert_eq!(id, CanId { priority: 2, pgn: 129025, destination: 255, source: 0x17 });
        assert_eq!(id.encode(), 0x09F8_0117);
    }

    #[test]
    fn pdu1_addressed() {
        // ISO request (59904 = 0xEA00) to 0x23 from 0x80, prio 6
        let id = CanId { priority: 6, pgn: 59904, destination: 0x23, source: 0x80 };
        let raw = id.encode();
        assert_eq!(raw, 0x18EA_2380);
        assert_eq!(CanId::decode(raw), id);
        // 126720 is PDU1 (PF 0xEF) with DP=1
        let p = CanId { priority: 7, pgn: 126720, destination: 5, source: 9 };
        assert_eq!(CanId::decode(p.encode()), p);
        assert!(is_pdu1(126720));
        assert!(!is_pdu1(127501));
    }

    #[test]
    fn hex_roundtrip() {
        assert_eq!(parse_hex("01 ff:a0").unwrap(), vec![1, 0xff, 0xa0]);
        assert_eq!(hex(&[1, 0xab]), "01ab");
    }
}
