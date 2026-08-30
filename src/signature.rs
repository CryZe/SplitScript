//! Parser for the compiler-known signature literal format.

/// Parses the textual signature format used by ASR. Each byte consists of two
/// hexadecimal nibbles and either nibble may be `?`.
pub fn parse_signature(signature: &str) -> Result<(Vec<u8>, Vec<u8>), String> {
    let nibbles: Vec<u8> = signature
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect();
    if nibbles.is_empty() || !nibbles.len().is_multiple_of(2) {
        return Err("a signature needs one or more complete bytes".to_owned());
    }
    if nibbles.len() / 2 >= 256 {
        return Err("signatures are limited to 255 bytes".to_owned());
    }
    let mut needle = Vec::with_capacity(nibbles.len() / 2);
    let mut mask = Vec::with_capacity(nibbles.len() / 2);
    let (pairs, remainder) = nibbles.as_chunks::<2>();
    debug_assert!(remainder.is_empty());
    for &[high_nibble, low_nibble] in pairs {
        let (high, high_mask) = signature_nibble(high_nibble)?;
        let (low, low_mask) = signature_nibble(low_nibble)?;
        needle.push((high << 4) | low);
        mask.push((high_mask << 4) | low_mask);
    }
    Ok((needle, mask))
}

fn signature_nibble(byte: u8) -> Result<(u8, u8), String> {
    Ok(match byte {
        b'0'..=b'9' => (byte - b'0', 0xF),
        b'a'..=b'f' => (byte - b'a' + 10, 0xF),
        b'A'..=b'F' => (byte - b'A' + 10, 0xF),
        b'?' => (0, 0),
        _ => {
            return Err(format!(
                "invalid signature character `{}`",
                char::from(byte)
            ));
        }
    })
}
