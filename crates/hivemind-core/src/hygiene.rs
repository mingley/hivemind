//! Write hygiene: secrets are rejected and invisible characters are stripped
//! before anything reaches the log.

use std::borrow::Cow;

/// Zero-width and bidirectional control characters can hide instructions from
/// human reviewers ("rules file backdoor" attacks). They are removed at ingest.
pub fn is_invisible(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{2069}'
            | '\u{FEFF}'
    )
}

pub fn strip_invisible(text: &str) -> Cow<'_, str> {
    if text.chars().any(is_invisible) {
        Cow::Owned(text.chars().filter(|c| !is_invisible(*c)).collect())
    } else {
        Cow::Borrowed(text)
    }
}

/// A secret pattern: prefix, minimum length of the token after it, the bytes
/// that token may contain, and the kind reported.
type Pattern = (&'static str, usize, fn(u8) -> bool, &'static str);

fn upper_or_digit(b: u8) -> bool {
    b.is_ascii_uppercase() || b.is_ascii_digit()
}

fn alphanumeric(b: u8) -> bool {
    b.is_ascii_alphanumeric()
}

fn token_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

const PATTERNS: &[Pattern] = &[
    ("AKIA", 16, upper_or_digit, "AWS access key"),
    ("ASIA", 16, upper_or_digit, "AWS access key"),
    ("ghp_", 36, alphanumeric, "GitHub token"),
    ("gho_", 36, alphanumeric, "GitHub token"),
    ("ghu_", 36, alphanumeric, "GitHub token"),
    ("ghs_", 36, alphanumeric, "GitHub token"),
    ("ghr_", 36, alphanumeric, "GitHub token"),
    ("github_pat_", 22, token_byte, "GitHub token"),
    ("glpat-", 20, token_byte, "GitLab token"),
    ("xoxb-", 10, token_byte, "Slack token"),
    ("xoxp-", 10, token_byte, "Slack token"),
    ("sk-ant-", 32, token_byte, "Anthropic API key"),
    ("sk-proj-", 32, token_byte, "OpenAI API key"),
    ("AIza", 35, token_byte, "Google API key"),
];

/// Returns the kind of the first likely secret in `text`. Patterns are
/// conservative to avoid rejecting ordinary text.
pub fn find_secret(text: &str) -> Option<&'static str> {
    if text.contains("-----BEGIN") && text.contains("PRIVATE KEY-----") {
        return Some("private key");
    }
    let bytes = text.as_bytes();
    for (prefix, min_len, allowed, kind) in PATTERNS {
        let mut start = 0;
        while let Some(found) = text[start..].find(prefix) {
            let at = start + found;
            let preceded_by_token = at > 0 && token_byte(bytes[at - 1]);
            let run = bytes[at + prefix.len()..]
                .iter()
                .take_while(|b| allowed(**b))
                .count();
            if !preceded_by_token && run >= *min_len {
                return Some(kind);
            }
            start = at + prefix.len();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_zero_width_and_bidi_controls() {
        let hidden = "run\u{200B} tests\u{202E}";
        assert_eq!(strip_invisible(hidden), "run tests");
        assert!(matches!(strip_invisible("plain"), Cow::Borrowed(_)));
    }

    #[test]
    fn detects_common_secrets() {
        assert_eq!(
            find_secret("key AKIAIOSFODNN7EXAMPLE here"),
            Some("AWS access key")
        );
        let gh = format!("token ghp_{}", "a".repeat(36));
        assert_eq!(find_secret(&gh), Some("GitHub token"));
        assert_eq!(
            find_secret("-----BEGIN OPENSSH PRIVATE KEY-----\nabc"),
            Some("private key")
        );
    }

    #[test]
    fn ignores_ordinary_text() {
        for text in [
            "cargo build fails with error[E0277]",
            "set OPENSSL_DIR to brew --prefix openssl@3",
            "the ghp_ prefix marks GitHub tokens",
            "AKIA is the prefix of AWS keys",
            "task-sk-proj-short",
        ] {
            assert_eq!(find_secret(text), None, "{text}");
        }
    }
}
