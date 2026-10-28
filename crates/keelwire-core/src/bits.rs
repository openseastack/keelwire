//! Little-endian, bit-addressed field access as used by NMEA 2000.
//!
//! Bit `n` of a payload is bit `n % 8` of byte `n / 8`; multi-bit fields are
//! little-endian and may straddle byte boundaries.

/// Read `len` (1..=64) bits starting at bit `offset`. Returns `None` when the
/// field extends beyond the end of `data`.
pub fn get_bits(data: &[u8], offset: usize, len: usize) -> Option<u64> {
    if len == 0 || len > 64 {
        return None;
    }
    if offset + len > data.len() * 8 {
        return None;
    }
    let mut value: u64 = 0;
    let mut written = 0usize;
    let mut bit = offset;
    while written < len {
        let byte = data[bit / 8];
        let shift = bit % 8;
        let take = (8 - shift).min(len - written);
        let mask = if take == 8 { 0xFF } else { ((1u16 << take) - 1) as u8 };
        let chunk = (byte >> shift) & mask;
        value |= (chunk as u64) << written;
        written += take;
        bit += take;
    }
    Some(value)
}

/// Sign-extend a `len`-bit two's complement value.
pub fn sign_extend(value: u64, len: usize) -> i64 {
    if len == 0 || len >= 64 {
        return value as i64;
    }
    let shift = 64 - len;
    ((value << shift) as i64) >> shift
}

/// Write `len` bits of `value` at bit `offset`, growing `data` (with 0xFF
/// padding, the N2K "not available" filler) if required.
pub fn set_bits(data: &mut Vec<u8>, offset: usize, len: usize, value: u64) {
    if len == 0 {
        return;
    }
    let needed = (offset + len).div_ceil(8);
    if data.len() < needed {
        data.resize(needed, 0xFF);
    }
    let mut remaining = len;
    let mut bit = offset;
    let mut v = value;
    while remaining > 0 {
        let shift = bit % 8;
        let take = (8 - shift).min(remaining);
        let mask: u8 = (if take == 8 { 0xFFu16 } else { (1u16 << take) - 1 } as u8) << shift;
        let b = &mut data[bit / 8];
        *b = (*b & !mask) | (((v as u8) << shift) & mask);
        v = if take >= 64 { 0 } else { v >> take };
        remaining -= take;
        bit += take;
    }
}

/// All-ones value for an unsigned field of `len` bits.
pub fn max_unsigned(len: usize) -> u64 {
    if len >= 64 {
        u64::MAX
    } else {
        (1u64 << len) - 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_aligned_and_unaligned() {
        let d = [0x34, 0x12, 0xFF];
        assert_eq!(get_bits(&d, 0, 16), Some(0x1234));
        assert_eq!(get_bits(&d, 4, 8), Some(0x23));
        assert_eq!(get_bits(&d, 0, 4), Some(0x4));
        assert_eq!(get_bits(&d, 20, 4), Some(0xF));
        assert_eq!(get_bits(&d, 20, 8), None);
    }

    #[test]
    fn write_roundtrip() {
        let mut d = vec![0u8; 4];
        set_bits(&mut d, 3, 11, 0x5A5);
        assert_eq!(get_bits(&d, 3, 11), Some(0x5A5));
        set_bits(&mut d, 30, 6, 0x3F);
        assert_eq!(d.len(), 5);
        assert_eq!(get_bits(&d, 30, 6), Some(0x3F));
        let mut e = Vec::new();
        set_bits(&mut e, 0, 64, 0x0123_4567_89AB_CDEF);
        assert_eq!(get_bits(&e, 0, 64), Some(0x0123_4567_89AB_CDEF));
    }

    #[test]
    fn signs() {
        assert_eq!(sign_extend(0xFFFF, 16), -1);
        assert_eq!(sign_extend(0x7FFF, 16), 32767);
        assert_eq!(sign_extend(0b100, 3), -4);
    }
}
