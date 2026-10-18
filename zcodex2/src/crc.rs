const fn make_table() -> [u8; 256] {
    let mut t = [0u8; 256];
    let mut i = 0;
    while i < 256 {
        let mut n = i as u8;
        let mut j = 0;
        while j < 8 {
            n = if n & 0x80 != 0 { (n << 1) ^ 0xD5 } else { n << 1 };
            j += 1;
        }
        t[i] = n;
        i += 1;
    }
    t
}

static TABLE: [u8; 256] = make_table();

/// CRC-8, polynomial 0xD5, init 0, no reflection, no final xor.
pub fn crc8(data: &[u8]) -> u8 {
    data.iter().fold(0u8, |c, &b| TABLE[(c ^ b) as usize])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_record() {
        // `0801 ff 04 983a0000` -> crc 0x5f (from Calibration Cube.zcodex2)
        assert_eq!(crc8(&[0x08, 0x01, 0xff, 0x04, 0x98, 0x3a, 0x00, 0x00]), 0x5f);
    }
}
