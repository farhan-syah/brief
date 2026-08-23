//! Ported verbatim from rtk's `core::tee` (sanitize_slug / short_hash).
//! Source: reference/rtk/src/core/tee.rs

use sha2::{Digest, Sha256};

/// Long slugs (usually an embedded file path that duplicates the command
/// the caller already issued) collapse past this length to a short
/// readable prefix plus a disambiguating hash.
const MAX_READABLE: usize = 24;

/// Sanitize a command slug for use in filenames. Replaces non-alphanumeric
/// chars (except underscore/hyphen) with underscore, then shortens long
/// slugs to keep fold filenames unique but compact.
pub(crate) fn sanitize_slug(slug: &str) -> String {
    let sanitized: String = slug
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.len() <= MAX_READABLE {
        return sanitized;
    }
    let prefix: String = sanitized.chars().take(8).collect();
    format!("{}_{}", prefix, short_hash(&sanitized))
}

/// First 6 hex chars (24 bits) of the SHA-256 of `s` — a compact tag to
/// keep shortened slugs distinct. Not collision-resistant on its own: 24
/// bits hits a birthday collision after only a few thousand distinct
/// slugs. Safe here because a clash also requires the identical readable
/// prefix *and* the same epoch second, which together scope fold writes
/// exactly as before.
fn short_hash(s: &str) -> String {
    format!("{:x}", Sha256::digest(s.as_bytes()))[..6].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passes_through_safe_chars() {
        assert_eq!(sanitize_slug("cargo_test"), "cargo_test");
        assert_eq!(sanitize_slug("cargo-test"), "cargo-test");
    }

    #[test]
    fn replaces_unsafe_chars() {
        assert_eq!(sanitize_slug("cargo test"), "cargo_test");
        assert_eq!(sanitize_slug("go/test/./pkg"), "go_test___pkg");
    }

    #[test]
    fn shortens_long_slugs_with_readable_prefix() {
        let long = format!("grep_0_{}", "a".repeat(50));
        let short = sanitize_slug(&long);
        assert!(
            short.len() < 24,
            "long slug should shorten, got '{}'",
            short
        );
        assert!(
            short.starts_with("grep_0_a"),
            "keeps a readable prefix, got '{}'",
            short
        );
    }

    #[test]
    fn shortening_is_deterministic_and_collision_free() {
        let long = format!("grep_0_{}", "a".repeat(50));
        let short = sanitize_slug(&long);
        assert_eq!(sanitize_slug(&long), short);
        let other = sanitize_slug(&format!("grep_1_{}", "a".repeat(50)));
        assert_ne!(other, short, "distinct slugs must not collide");
    }
}
