use sha2::{Digest, Sha256};
use sim_lib_net_core::hex_encode;

pub(crate) fn content_digest(bytes: &[u8]) -> String {
    hex_encode(&Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_spelling_is_lowercase_sha256_hex() {
        assert_eq!(
            content_digest(b"SIM\n"),
            "7040c16de1e23dddf77df8ff8043c2bee23b42b47a0f326e5e124ae9bc2178e0"
        );
    }
}
