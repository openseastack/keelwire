//! Proprietary PGN support: the common manufacturer header, a data-driven
//! schema codec and Maretron/Carling specifics.

pub mod maretron;
pub mod schema;

/// True for PGNs in the proprietary ranges.
pub fn is_proprietary_pgn(pgn: u32) -> bool {
    pgn == 61184 || (65280..=65535).contains(&pgn) || pgn == 126720 || (130816..=131071).contains(&pgn)
}

/// Manufacturer Code (11) · Reserved (2, ones) · Industry Code (3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ProprietaryHeader {
    pub manufacturer_code: u16,
    pub industry_code: u8,
}

impl ProprietaryHeader {
    pub const MARETRON: ProprietaryHeader = ProprietaryHeader { manufacturer_code: 137, industry_code: 4 };

    pub fn decode(b: &[u8]) -> Option<Self> {
        if b.len() < 2 {
            return None;
        }
        let w = u16::from_le_bytes([b[0], b[1]]);
        Some(ProprietaryHeader { manufacturer_code: w & 0x7FF, industry_code: (w >> 13) as u8 })
    }

    pub fn encode(&self) -> [u8; 2] {
        let w = (self.manufacturer_code & 0x7FF) | (0b11 << 11) | ((self.industry_code as u16 & 0x7) << 13);
        w.to_le_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maretron_header_bytes() {
        assert_eq!(ProprietaryHeader::MARETRON.encode(), [0x89, 0x98]);
        assert_eq!(ProprietaryHeader::decode(&[0x89, 0x98]).unwrap(), ProprietaryHeader::MARETRON);
    }
}
