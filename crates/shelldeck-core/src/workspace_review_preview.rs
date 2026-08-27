const MAX_IMAGE_DIMENSION: u32 = 8_192;
const MAX_IMAGE_PIXELS: u64 = 32_000_000;

fn valid_image_size(width: u32, height: u32) -> bool {
    width > 0
        && height > 0
        && width <= MAX_IMAGE_DIMENSION
        && height <= MAX_IMAGE_DIMENSION
        && u64::from(width) * u64::from(height) <= MAX_IMAGE_PIXELS
}

pub(super) fn looks_like_image(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\x89PNG\r\n\x1a\n")
        || bytes.starts_with(&[0xff, 0xd8, 0xff])
        || bytes.starts_with(b"GIF87a")
        || bytes.starts_with(b"GIF89a")
        || (bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP")
}

pub(super) fn validated_image_metadata(bytes: &[u8]) -> Option<(&'static str, u32, u32)> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n")
        && bytes.len() >= 45
        && bytes[8..12] == [0, 0, 0, 13]
        && &bytes[12..16] == b"IHDR"
        && bytes.windows(4).any(|window| window == b"IEND")
    {
        let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
        let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
        return valid_image_size(width, height).then_some(("image/png", width, height));
    }
    if (bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a"))
        && bytes.len() >= 14
        && bytes.last() == Some(&0x3b)
    {
        let width = u32::from(u16::from_le_bytes(bytes[6..8].try_into().ok()?));
        let height = u32::from(u16::from_le_bytes(bytes[8..10].try_into().ok()?));
        return valid_image_size(width, height).then_some(("image/gif", width, height));
    }
    if bytes.starts_with(&[0xff, 0xd8]) && bytes.ends_with(&[0xff, 0xd9]) {
        let mut offset = 2;
        while offset + 4 <= bytes.len() {
            if bytes[offset] != 0xff {
                offset += 1;
                continue;
            }
            let marker = bytes[offset + 1];
            offset += 2;
            if matches!(marker, 0xd8 | 0xd9) || (0xd0..=0xd7).contains(&marker) {
                continue;
            }
            if offset + 2 > bytes.len() {
                return None;
            }
            let length = usize::from(u16::from_be_bytes(
                bytes[offset..offset + 2].try_into().ok()?,
            ));
            if length < 2 || offset + length > bytes.len() {
                return None;
            }
            if matches!(marker, 0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf)
                && length >= 7
            {
                let height = u32::from(u16::from_be_bytes(
                    bytes[offset + 3..offset + 5].try_into().ok()?,
                ));
                let width = u32::from(u16::from_be_bytes(
                    bytes[offset + 5..offset + 7].try_into().ok()?,
                ));
                return valid_image_size(width, height).then_some(("image/jpeg", width, height));
            }
            offset += length;
        }
    }
    None
}
