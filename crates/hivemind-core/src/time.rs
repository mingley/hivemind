//! Time as milliseconds since the Unix epoch, UTC.

use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const MS_PER_DAY: i64 = 86_400_000;
/// 9999-12-31T23:59:59.999Z: the last instant RFC 3339 can express.
pub const MAX_MILLIS: i64 = 253_402_300_799_999;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "invalid time `{0}`; use RFC 3339 (2026-11-10T21:00:00Z), a date (2026-11-10), or a duration from now (90d)"
)]
pub struct TimeParseError(pub String);

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Timestamp(i64);

impl Timestamp {
    pub fn now() -> Self {
        let since_epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO);
        Self(i64::try_from(since_epoch.as_millis()).unwrap_or(i64::MAX))
    }

    pub const fn from_millis(ms: i64) -> Self {
        Self(ms)
    }

    pub const fn as_millis(self) -> i64 {
        self.0
    }

    /// Whether the timestamp falls between 1970 and the year 9999, the range
    /// that RFC 3339 output covers. Stored timestamps must be representable.
    pub const fn is_representable(self) -> bool {
        self.0 >= 0 && self.0 <= MAX_MILLIS
    }

    pub fn plus_days(self, days: i64) -> Self {
        Self(self.0.saturating_add(days.saturating_mul(MS_PER_DAY)))
    }

    /// Whole and fractional days from `earlier` to `self`; negative if `earlier` is later.
    pub fn days_since(self, earlier: Timestamp) -> f64 {
        (self.0 - earlier.0) as f64 / MS_PER_DAY as f64
    }

    fn to_system_time(self) -> SystemTime {
        if self.0 >= 0 {
            UNIX_EPOCH + Duration::from_millis(self.0.unsigned_abs())
        } else {
            UNIX_EPOCH - Duration::from_millis(self.0.unsigned_abs())
        }
    }

    pub fn to_rfc3339(self) -> String {
        if self.is_representable() {
            humantime::format_rfc3339_millis(self.to_system_time()).to_string()
        } else {
            format!("@{}ms", self.0)
        }
    }

    /// `YYYY-MM-DD`, or the raw form for unrepresentable timestamps.
    pub fn date(self) -> String {
        let text = self.to_rfc3339();
        if self.is_representable() {
            text[..10].to_owned()
        } else {
            text
        }
    }

    /// Parses RFC 3339, a bare date (midnight UTC), or a duration such as
    /// `90d` or `2h`, which is added to `now`. Results outside 1970–9999 are
    /// rejected.
    pub fn parse_relative(input: &str, now: Timestamp) -> Result<Self, TimeParseError> {
        let parsed = Self::parse_unchecked(input, now)?;
        if parsed.is_representable() {
            Ok(parsed)
        } else {
            Err(TimeParseError(input.to_owned()))
        }
    }

    fn parse_unchecked(input: &str, now: Timestamp) -> Result<Self, TimeParseError> {
        let s = input.trim();
        let rfc3339 = if s.len() == 10 && s.as_bytes().get(4) == Some(&b'-') {
            format!("{s}T00:00:00Z")
        } else {
            s.to_owned()
        };
        if let Ok(t) = humantime::parse_rfc3339_weak(&rfc3339) {
            let ms = match t.duration_since(UNIX_EPOCH) {
                Ok(d) => i64::try_from(d.as_millis()).unwrap_or(i64::MAX),
                Err(e) => -i64::try_from(e.duration().as_millis()).unwrap_or(i64::MAX),
            };
            return Ok(Self(ms));
        }
        if let Ok(d) = humantime::parse_duration(s) {
            let ms = i64::try_from(d.as_millis()).unwrap_or(i64::MAX);
            return Ok(Self(now.0.saturating_add(ms)));
        }
        Err(TimeParseError(input.to_owned()))
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_rfc3339())
    }
}

impl fmt::Debug for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_dates_timestamps_and_durations() {
        let now = Timestamp::from_millis(1_000_000);
        let date = Timestamp::parse_relative("2026-11-10", now).unwrap();
        assert_eq!(date.to_rfc3339(), "2026-11-10T00:00:00.000Z");
        let exact = Timestamp::parse_relative("2026-11-10T21:02:55.625Z", now).unwrap();
        assert_eq!(exact.to_rfc3339(), "2026-11-10T21:02:55.625Z");
        let later = Timestamp::parse_relative("90d", now).unwrap();
        assert_eq!(later, now.plus_days(90));
        assert!(Timestamp::parse_relative("next tuesday", now).is_err());
    }

    #[test]
    fn far_future_and_negative_times_are_rejected_and_never_panic() {
        let now = Timestamp::from_millis(1_000_000);
        assert!(Timestamp::parse_relative("9000y", now).is_err());
        let huge = Timestamp::from_millis(i64::MAX);
        assert!(!huge.is_representable());
        assert!(huge.to_rfc3339().starts_with('@'));
        assert!(Timestamp::from_millis(-5).date().starts_with('@'));
    }

    #[test]
    fn days_since_is_signed() {
        let a = Timestamp::from_millis(0);
        let b = a.plus_days(3);
        assert!((b.days_since(a) - 3.0).abs() < 1e-9);
        assert!(a.days_since(b) < 0.0);
    }
}
