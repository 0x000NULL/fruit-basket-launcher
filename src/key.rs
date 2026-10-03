//! The one key every build is checked against.
//!
//! The site signs `fruit-basket/feed.json` with this key (`make sign`), and
//! the feed carries the SHA-256 of every build, so a build is trusted only
//! when its hash is in a feed whose signature checks out.

/// minisign public key, base64 (second line of `fruitbasket.pub`).
pub const PUBLIC_KEY: &str = "RWTpYGSe+VZ8mgRF92/WTidtLIaI0SvSGJ4U08ji9ePPkUgqV2llWyfQ";

/// The key ID minisign prints, shown in the UI as `key 9A7C56F99E6460E9`.
pub const KEY_ID: &str = "9A7C56F99E6460E9";

/// Where the feed lives. `FRUITBASKET_FEED` overrides it for testing
/// against a local copy of the site.
pub const FEED_URL: &str = "https://projects.ethanaldrich.net/fruit-basket/feed.json";

pub fn feed_url() -> String {
    std::env::var("FRUITBASKET_FEED").unwrap_or_else(|_| FEED_URL.to_string())
}

/// `FRUITBASKET_KEY` swaps the key, so tests and a local site can sign
/// with a throwaway one. Release builds ignore it.
pub fn public_key() -> String {
    if cfg!(debug_assertions) {
        if let Ok(key) = std::env::var("FRUITBASKET_KEY") {
            return key;
        }
    }
    PUBLIC_KEY.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_id_matches_public_key() {
        // A minisign public key is "Ed" + 8-byte key ID (little-endian) + 32-byte key.
        let raw = base64_decode(PUBLIC_KEY);
        assert_eq!(&raw[..2], b"Ed");
        let id: String = raw[2..10].iter().rev().map(|b| format!("{b:02X}")).collect();
        assert_eq!(id, KEY_ID);
    }

    fn base64_decode(s: &str) -> Vec<u8> {
        const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = Vec::new();
        let (mut buf, mut bits) = (0u32, 0);
        for c in s.bytes().filter(|&c| c != b'=') {
            buf = buf << 6 | T.iter().position(|&t| t == c).unwrap() as u32;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                out.push((buf >> bits) as u8);
            }
        }
        out
    }
}
