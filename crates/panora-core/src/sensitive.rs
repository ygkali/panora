// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Heuristics for text that is probably a secret: private keys, tokens,
//! card and account numbers. A hit proves nothing; it only decides whether
//! an entry is masked in the list and expires early
//! (`privacy.sensitive_policy`, `privacy.sensitive_ttl_minutes`).
//!
//! The rules err on the side of ordinary text: a sentence, a URL, a hash, a
//! path, a file name or an identifier made of words must not be flagged,
//! because a false positive hides something the user wanted to see again.

/// What a piece of text was taken for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SensitiveKind {
    /// A PEM, OpenSSH or PGP private key block.
    PrivateKey,
    /// A JSON Web Token.
    Jwt,
    /// A token with a well-known vendor prefix (GitHub, AWS, Stripe, ...).
    ApiToken,
    /// A payment card number that passes the Luhn check.
    CardNumber,
    /// An IBAN that passes the mod-97 check.
    Iban,
    /// A single high-entropy token of no known shape: a password or a key.
    HighEntropy,
}

impl SensitiveKind {
    /// Short English label for masked previews and logs.
    pub fn label(self) -> &'static str {
        match self {
            SensitiveKind::PrivateKey => "private key",
            SensitiveKind::Jwt => "JWT",
            SensitiveKind::ApiToken => "API token",
            SensitiveKind::CardNumber => "card number",
            SensitiveKind::Iban => "IBAN",
            SensitiveKind::HighEntropy => "secret",
        }
    }
}

/// Vendor prefixes of tokens that are secrets by construction. Each must be
/// followed by at least `TOKEN_TAIL_MIN` token characters.
const TOKEN_PREFIXES: &[&str] = &[
    "sk-",
    "sk_live_",
    "sk_test_",
    "rk_live_",
    "rk_test_",
    "ghp_",
    "gho_",
    "ghu_",
    "ghs_",
    "ghr_",
    "github_pat_",
    "glpat-",
    "xoxb-",
    "xoxp-",
    "xoxa-",
    "xoxr-",
    "xapp-",
    "AKIA",
    "ASIA",
    "AIza",
    "npm_",
    "dop_v1_",
    "pypi-",
    "hf_",
    "shpat_",
    "shpss_",
    "sq0atp-",
    "sq0csp-",
    "SG.",
    "lin_api_",
    "ya29.",
];

/// Characters after a vendor prefix before the token counts.
const TOKEN_TAIL_MIN: usize = 16;

/// Longest text the heuristics look at; a document is not a secret.
const MAX_LEN: usize = 8 * 1024;

/// Classify `text` as sensitive, or `None` when it reads as ordinary text.
pub fn detect(text: &str) -> Option<SensitiveKind> {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.len() > MAX_LEN {
        return None;
    }
    if is_private_key(trimmed) {
        return Some(SensitiveKind::PrivateKey);
    }
    if is_card_number(trimmed) {
        return Some(SensitiveKind::CardNumber);
    }
    if is_iban(trimmed) {
        return Some(SensitiveKind::Iban);
    }
    // Everything below is a single token.
    if trimmed.chars().any(char::is_whitespace) {
        return None;
    }
    if is_jwt(trimmed) {
        return Some(SensitiveKind::Jwt);
    }
    if has_token_prefix(trimmed) {
        return Some(SensitiveKind::ApiToken);
    }
    if is_high_entropy(trimmed) {
        return Some(SensitiveKind::HighEntropy);
    }
    None
}

fn is_private_key(text: &str) -> bool {
    text.contains("-----BEGIN") && text.contains("PRIVATE KEY")
}

fn is_token_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.'
}

/// Three base64url segments, the first two JSON objects (`{"` encodes as
/// `eyJ`).
fn is_jwt(token: &str) -> bool {
    let parts: Vec<&str> = token.split('.').collect();
    parts.len() == 3
        && parts[0].starts_with("eyJ")
        && parts[1].starts_with("eyJ")
        && parts.iter().all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        })
}

fn has_token_prefix(token: &str) -> bool {
    TOKEN_PREFIXES.iter().any(|prefix| {
        token
            .strip_prefix(prefix)
            .is_some_and(|tail| tail.len() >= TOKEN_TAIL_MIN && tail.bytes().all(is_token_char))
    })
}

/// Digits, optionally grouped by spaces or dashes: the lengths and leading
/// digits of the card networks, and the Luhn check digit.
fn is_card_number(text: &str) -> bool {
    if !text
        .bytes()
        .all(|b| b.is_ascii_digit() || b == b' ' || b == b'-')
    {
        return false;
    }
    let digits: Vec<u8> = text
        .bytes()
        .filter(u8::is_ascii_digit)
        .map(|b| b - b'0')
        .collect();
    if !(14..=19).contains(&digits.len()) || !(2..=6).contains(&digits[0]) {
        return false;
    }
    if digits.iter().all(|&d| d == digits[0]) {
        return false;
    }
    luhn(&digits)
}

fn luhn(digits: &[u8]) -> bool {
    let sum: u32 = digits
        .iter()
        .rev()
        .enumerate()
        .map(|(i, &d)| {
            let d = u32::from(d);
            if i % 2 == 1 {
                let doubled = d * 2;
                if doubled > 9 {
                    doubled - 9
                } else {
                    doubled
                }
            } else {
                d
            }
        })
        .sum();
    sum.is_multiple_of(10)
}

/// Country code, two check digits, up to thirty alphanumerics, mod 97 == 1
/// after moving the first four characters to the end.
fn is_iban(text: &str) -> bool {
    let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    let bytes = compact.as_bytes();
    if !(15..=34).contains(&bytes.len())
        || !bytes[..2].iter().all(u8::is_ascii_uppercase)
        || !bytes[2..4].iter().all(u8::is_ascii_digit)
        || !bytes
            .iter()
            .all(|b| b.is_ascii_digit() || b.is_ascii_uppercase())
    {
        return false;
    }
    let mut remainder: u32 = 0;
    for &b in bytes[4..].iter().chain(&bytes[..4]) {
        let value = if b.is_ascii_digit() {
            u32::from(b - b'0')
        } else {
            u32::from(b - b'A') + 10
        };
        remainder = if value >= 10 {
            (remainder * 100 + value) % 97
        } else {
            (remainder * 10 + value) % 97
        };
    }
    remainder == 1
}

/// A single token that looks generated rather than written: long enough,
/// drawn from several character classes, with digits, and with an entropy
/// close to that of random characters. URLs, paths, e-mail addresses, file
/// names, hashes and word-built identifiers are excluded first.
fn is_high_entropy(token: &str) -> bool {
    let len = token.chars().count();
    if !(20..=128).contains(&len) {
        return false;
    }
    if token.contains("://")
        || token.starts_with('/')
        || token.starts_with("~/")
        || token.starts_with("./")
        || token.contains('@')
        || looks_like_filename(token)
        || token.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
        || has_word_like_run(token)
    {
        return false;
    }
    let (mut lower, mut upper, mut digit, mut other) = (0usize, 0usize, 0usize, 0usize);
    for c in token.chars() {
        if c.is_ascii_lowercase() {
            lower += 1;
        } else if c.is_ascii_uppercase() {
            upper += 1;
        } else if c.is_ascii_digit() {
            digit += 1;
        } else {
            other += 1;
        }
    }
    let classes = [lower, upper, digit, other]
        .iter()
        .filter(|&&n| n > 0)
        .count();
    if classes < 3 || digit < 2 {
        return false;
    }
    let needed = if len >= 24 { 4.0 } else { 3.9 };
    shannon_entropy(token) >= needed
}

/// `name.ext` with a short alphanumeric extension.
fn looks_like_filename(token: &str) -> bool {
    token.rsplit_once('.').is_some_and(|(stem, ext)| {
        !stem.is_empty()
            && (2..=5).contains(&ext.len())
            && ext.chars().all(|c| c.is_ascii_alphanumeric())
            && ext.chars().any(|c| c.is_ascii_alphabetic())
    })
}

/// A run of five or more letters with the vowel share and consonant runs of
/// a word. Random keys seldom produce one; identifiers and file names are
/// made of them.
fn has_word_like_run(token: &str) -> bool {
    const VOWELS: &str = "aeiouyAEIOUY";
    token.split(|c: char| !c.is_ascii_alphabetic()).any(|run| {
        if run.len() < 5 {
            return false;
        }
        let vowels = run.chars().filter(|c| VOWELS.contains(*c)).count();
        let mut longest = 0;
        let mut current = 0;
        for c in run.chars() {
            if VOWELS.contains(c) {
                current = 0;
            } else {
                current += 1;
                longest = longest.max(current);
            }
        }
        vowels * 4 >= run.len() && longest <= 4
    })
}

/// Bits per character of the token's own character distribution.
fn shannon_entropy(token: &str) -> f64 {
    let mut counts = std::collections::HashMap::new();
    let mut total = 0f64;
    for c in token.chars() {
        *counts.entry(c).or_insert(0f64) += 1.0;
        total += 1.0;
    }
    counts
        .values()
        .map(|&n| {
            let p = n / total;
            -p * p.log2()
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_shapes_are_recognised() {
        let cases = [
            ("AKIAIOSFODNN7EXAMPLE", SensitiveKind::ApiToken),
            ("ghp_A1b2C3d4E5f6G7h8I9j0K1l2M3n4O5p6Q7r8", SensitiveKind::ApiToken),
            (
                "sk-proj-abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJ",
                SensitiveKind::ApiToken,
            ),
            (
                "xoxb-000000000000-not-a-real-token-at-all-000",
                SensitiveKind::ApiToken,
            ),
            (
                "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c",
                SensitiveKind::Jwt,
            ),
            ("4111 1111 1111 1111", SensitiveKind::CardNumber),
            ("5555-5555-5555-4444", SensitiveKind::CardNumber),
            ("378282246310005", SensitiveKind::CardNumber),
            ("GB82 WEST 1234 5698 7654 32", SensitiveKind::Iban),
            ("DE89370400440532013000", SensitiveKind::Iban),
            ("TR330006100519786457841326", SensitiveKind::Iban),
            (
                "-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKCAQEA\n-----END RSA PRIVATE KEY-----",
                SensitiveKind::PrivateKey,
            ),
            (
                "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAA\n-----END OPENSSH PRIVATE KEY-----",
                SensitiveKind::PrivateKey,
            ),
            ("xK9#mP2$vL8-qR4!wN6%zT1&yH5^bJ3*", SensitiveKind::HighEntropy),
            ("a8F3kQ9zL2mX7vB4nC6pD1sG5hJ0wE", SensitiveKind::HighEntropy),
        ];
        for (text, expected) in cases {
            assert_eq!(detect(text), Some(expected), "{text:?}");
        }
    }

    #[test]
    fn ordinary_text_is_left_alone() {
        let cases = [
            "",
            "   ",
            "Hello world, this is a normal sentence.",
            "https://example.com/very/long/path?with=query&and=params123",
            "3b1f0e1c9d8a7b6c5d4e3f2a1b0c9d8e7f6a5b4c3d2e1f0a9b8c7d6e5f4a3b2c",
            "550e8400-e29b-41d4-a716-446655440000",
            "getUserAccountBalanceById",
            "MyProject_Build_2026_Final_v3",
            "/home/user/Documents/report-final-v2.pdf",
            "~/.config/panora/config.toml",
            "user.name+tag@example.com",
            "Screenshot_2026-09-20_12-34-56.png",
            "0000 0000 0000 0000",
            "4111 1111 1111 1112",
            "1726790400000",
            "+90 532 123 45 67",
            "GB82 WEST 1234 5698 7654 33",
            "sk-learn",
            "eyJ.eyJ",
            "ThisIsAPerfectlyNormalCamelCaseName",
            "Merhaba dünya, bu bir düz metin kaydı.",
            "v1.3.0-rc1+build.2026.09.20",
        ];
        for text in cases {
            assert_eq!(detect(text), None, "{text:?}");
        }
    }

    #[test]
    fn a_document_is_never_a_secret() {
        let long = "x9".repeat(MAX_LEN);
        assert_eq!(detect(&long), None);
    }

    #[test]
    fn luhn_and_iban_arithmetic() {
        assert!(luhn(&[4, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1]));
        assert!(!luhn(&[4, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2]));
        assert!(is_iban("GB82WEST12345698765432"));
        assert!(!is_iban("GB82WEST12345698765433"));
        assert!(!is_iban("gb82west12345698765432"));
    }

    #[test]
    fn entropy_of_a_uniform_token_is_its_bit_width() {
        let entropy = shannon_entropy("abcdefghijklmnop");
        assert!((entropy - 4.0).abs() < 1e-9);
        assert!(shannon_entropy("aaaaaaaa") < 1e-9);
    }
}
