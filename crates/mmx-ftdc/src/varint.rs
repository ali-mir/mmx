use std::io::{self, Read};

/// Read an unsigned varint (LEB128 / protobuf-style) from the given reader.
///
/// Returns the decoded `u64` value. Returns an error if the varint encoding
/// exceeds 10 bytes (would overflow a u64).
pub fn read_uvarint(reader: &mut impl Read) -> io::Result<u64> {
    let mut result: u64 = 0;
    let mut shift: u32 = 0;
    let mut buf = [0u8; 1];

    loop {
        reader.read_exact(&mut buf)?;
        let byte = buf[0];

        if shift >= 63 && byte > 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "varint overflows u64",
            ));
        }

        result |= ((byte & 0x7F) as u64) << shift;

        if byte & 0x80 == 0 {
            return Ok(result);
        }

        shift += 7;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn encode_uvarint(mut value: u64) -> Vec<u8> {
        let mut buf = Vec::new();
        loop {
            let mut byte = (value & 0x7F) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            buf.push(byte);
            if value == 0 {
                break;
            }
        }
        buf
    }

    #[test]
    fn test_zero() {
        let data = encode_uvarint(0);
        let mut cursor = Cursor::new(&data);
        assert_eq!(read_uvarint(&mut cursor).unwrap(), 0);
    }

    #[test]
    fn test_one() {
        let data = encode_uvarint(1);
        let mut cursor = Cursor::new(&data);
        assert_eq!(read_uvarint(&mut cursor).unwrap(), 1);
    }

    #[test]
    fn test_127() {
        let data = encode_uvarint(127);
        assert_eq!(data, vec![127]);
        let mut cursor = Cursor::new(&data);
        assert_eq!(read_uvarint(&mut cursor).unwrap(), 127);
    }

    #[test]
    fn test_128() {
        let data = encode_uvarint(128);
        assert_eq!(data, vec![0x80, 0x01]);
        let mut cursor = Cursor::new(&data);
        assert_eq!(read_uvarint(&mut cursor).unwrap(), 128);
    }

    #[test]
    fn test_300() {
        let data = encode_uvarint(300);
        let mut cursor = Cursor::new(&data);
        assert_eq!(read_uvarint(&mut cursor).unwrap(), 300);
    }

    #[test]
    fn test_u64_max() {
        let data = encode_uvarint(u64::MAX);
        let mut cursor = Cursor::new(&data);
        assert_eq!(read_uvarint(&mut cursor).unwrap(), u64::MAX);
    }

    #[test]
    fn test_overflow() {
        // 11 bytes with continuation bits — would overflow u64
        let data = vec![0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x02];
        let mut cursor = Cursor::new(&data);
        assert!(read_uvarint(&mut cursor).is_err());
    }

    #[test]
    fn test_roundtrip_various() {
        for &val in &[
            0u64,
            1,
            127,
            128,
            255,
            256,
            16384,
            1_000_000,
            u64::MAX / 2,
            u64::MAX,
        ] {
            let data = encode_uvarint(val);
            let mut cursor = Cursor::new(&data);
            assert_eq!(read_uvarint(&mut cursor).unwrap(), val, "failed for {val}");
        }
    }

    #[test]
    fn test_eof() {
        let data: Vec<u8> = vec![0x80]; // continuation bit but no more data
        let mut cursor = Cursor::new(&data);
        assert!(read_uvarint(&mut cursor).is_err());
    }
}
