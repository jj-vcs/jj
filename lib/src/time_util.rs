// Copyright 2024 The Jujutsu Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// https://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Provides support for parsing and matching date ranges.

use interim::DateError;
use interim::Dialect;
use interim::parse_date_string;
use jiff::Zoned;
use thiserror::Error;

use crate::backend::MillisSinceEpoch;
use crate::backend::Timestamp;

/// Error occurred during date pattern parsing.
#[derive(Debug, Error)]
pub enum DatePatternParseError {
    /// Unknown pattern kind is specified.
    #[error("Invalid date pattern kind `{0}:`")]
    InvalidKind(String),
    /// Failed to parse timestamp.
    #[error(transparent)]
    ParseError(#[from] DateError),
}

/// Represents an range of dates that may be matched against.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum DatePattern {
    /// Represents all dates at or after the given instant.
    AtOrAfter(MillisSinceEpoch),
    /// Represents all dates before, but not including, the given instant.
    Before(MillisSinceEpoch),
}

impl DatePattern {
    /// Parses a string into a DatePattern.
    ///
    /// * `s` is the string to be parsed.
    ///
    /// * `kind` must be either "after" or "before". This determines whether the
    ///   pattern will match dates after or before the parsed date.
    ///
    /// * `now` is the user's current time as a [`jiff::Zoned`]. Knowledge of
    ///   offset changes is needed to correctly process relative times like
    ///   "today". For example, California entered DST on March 10, 2024,
    ///   shifting clocks from UTC-8 to UTC-7 at 2:00 AM. If the pattern "today"
    ///   was parsed at noon on that day, it should be interpreted as
    ///   2024-03-10T00:00:00-08:00 even though the current offset is -07:00.
    pub fn from_str_kind(s: &str, kind: &str, now: Zoned) -> Result<Self, DatePatternParseError> {
        let d =
            parse_date_string(s, now, Dialect::Us).map_err(DatePatternParseError::ParseError)?;
        let millis_since_epoch = MillisSinceEpoch(d.timestamp().as_millisecond());
        match kind {
            "after" => Ok(Self::AtOrAfter(millis_since_epoch)),
            "before" => Ok(Self::Before(millis_since_epoch)),
            kind => Err(DatePatternParseError::InvalidKind(kind.to_owned())),
        }
    }

    /// Determines whether a given timestamp is matched by the pattern.
    pub fn matches(&self, timestamp: &Timestamp) -> bool {
        match self {
            Self::AtOrAfter(earliest) => *earliest <= timestamp.timestamp,
            Self::Before(latest) => timestamp.timestamp < *latest,
        }
    }
}

// @TODO ideally we would have this unified with the other parsing code. However
// we use the interim crate which does not handle explicitly given time zone
// information
/// Parse a [`&str`] with time zone information into a `Timestamp`
///
/// Parsing occurs in three steps:
/// 1. First, try to parse an RFC 2822 timestamp.
/// 2. If step 1 fails, attempt to parse RFC3339/ISO8601 timestamp that include
///    an _explicit_ offset.
/// 3. if step 1 and 2 fail, fall back to parsing as
///    [`crate::backend::Timestamp`], which handles formats like
///    "2024-01-01T00:00:00") and assume UTC.
pub fn parse_datetime(s: &str) -> Result<Timestamp, jiff::Error> {
    if let Ok(zoned) = jiff::fmt::rfc2822::parse(s) {
        return Ok(Timestamp::from_zoned(zoned));
    }
    if let Some(zoned) = parse_temporal_datetime(s) {
        return Ok(Timestamp::from_zoned(zoned));
    }
    let ts: jiff::Timestamp = s.parse()?;
    Ok(Timestamp::from_zoned(ts.to_zoned(jiff::tz::TimeZone::UTC)))
}

fn parse_temporal_datetime(literal: &str) -> Option<Zoned> {
    use jiff::civil::Time;
    use jiff::fmt::temporal::Pieces;
    use jiff::tz::TimeZone;

    let trimmed = literal.trim();
    if let Ok(zoned) = trimmed.parse::<Zoned>() {
        return Some(zoned);
    }
    let pieces = Pieces::parse(trimmed).ok()?;
    let time = pieces.time().unwrap_or(Time::midnight());
    let dt = pieces.date().to_datetime(time);
    let offset = pieces.to_numeric_offset()?;
    let tz = TimeZone::fixed(offset);
    dt.to_zoned(tz).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_equal(now: &Zoned, expression: &str, should_equal_time: &str) {
        let expression = DatePattern::from_str_kind(expression, "after", now.clone()).unwrap();
        let expected_ts: jiff::Timestamp = should_equal_time.parse().unwrap();
        assert_eq!(
            expression,
            DatePattern::AtOrAfter(MillisSinceEpoch(expected_ts.as_millisecond()))
        );
    }

    #[test]
    fn test_date_pattern_parses_dates_without_times_as_the_date_at_local_midnight() {
        let ts: jiff::Timestamp = "2024-01-01T00:00:00-08:00".parse().unwrap();
        let tz = jiff::tz::TimeZone::fixed(jiff::tz::offset(-8));
        let now = ts.to_zoned(tz);
        test_equal(&now, "2023-03-25", "2023-03-25T08:00:00Z");
        test_equal(&now, "3/25/2023", "2023-03-25T08:00:00Z");
        test_equal(&now, "3/25/23", "2023-03-25T08:00:00Z");
    }

    #[test]
    fn test_date_pattern_parses_dates_with_times_without_specifying_an_offset() {
        let ts: jiff::Timestamp = "2024-01-01T00:00:00-08:00".parse().unwrap();
        let tz = jiff::tz::TimeZone::fixed(jiff::tz::offset(-8));
        let now = ts.to_zoned(tz);
        test_equal(&now, "2023-03-25T00:00:00", "2023-03-25T08:00:00Z");
        test_equal(&now, "2023-03-25 00:00:00", "2023-03-25T08:00:00Z");
    }

    #[test]
    fn test_date_pattern_parses_dates_with_a_specified_offset() {
        let ts: jiff::Timestamp = "2024-01-01T00:00:00-08:00".parse().unwrap();
        let tz = jiff::tz::TimeZone::fixed(jiff::tz::offset(-8));
        let now = ts.to_zoned(tz);
        test_equal(
            &now,
            "2023-03-25T00:00:00-05:00",
            "2023-03-25T00:00:00-05:00",
        );
    }

    #[test]
    fn test_date_pattern_parses_dates_with_the_z_offset() {
        let ts: jiff::Timestamp = "2024-01-01T00:00:00-08:00".parse().unwrap();
        let tz = jiff::tz::TimeZone::fixed(jiff::tz::offset(-8));
        let now = ts.to_zoned(tz);
        test_equal(&now, "2023-03-25T00:00:00Z", "2023-03-25T00:00:00Z");
    }

    #[test]
    fn test_date_pattern_parses_relative_durations() {
        let ts: jiff::Timestamp = "2024-01-01T00:00:00-08:00".parse().unwrap();
        let tz = jiff::tz::TimeZone::fixed(jiff::tz::offset(-8));
        let now = ts.to_zoned(tz);
        test_equal(&now, "2 hours ago", "2024-01-01T06:00:00Z");
        test_equal(&now, "5 minutes", "2024-01-01T08:05:00Z");
        test_equal(&now, "1 week ago", "2023-12-25T08:00:00Z");
        test_equal(&now, "yesterday", "2023-12-31T08:00:00Z");
        test_equal(&now, "tomorrow", "2024-01-02T08:00:00Z");
    }

    #[test]
    fn test_date_pattern_parses_relative_dates_with_times() {
        let ts: jiff::Timestamp = "2024-01-01T08:00:00-08:00".parse().unwrap();
        let tz = jiff::tz::TimeZone::fixed(jiff::tz::offset(-8));
        let now = ts.to_zoned(tz);
        test_equal(&now, "yesterday 5pm", "2024-01-01T01:00:00Z");
        test_equal(&now, "yesterday 10am", "2023-12-31T18:00:00Z");
        test_equal(&now, "yesterday 10:30", "2023-12-31T18:30:00Z");
    }

    #[test]
    fn test_parse_datetime_non_sense_yields_error() {
        let parse_error = parse_datetime("aaaaa").err().unwrap();
        // just verify that it's an error; jiff's error types are different
        assert!(parse_error.to_string().contains("invalid"));
    }

    #[test]
    fn test_parse_datetime_human_readable() {
        // this is the example given in the help text for `jj metaedit
        // --author-timestamp`
        let timestamp = parse_datetime("2000-01-23T01:23:45-08:00").unwrap();
        let human_readable_explicit = parse_datetime("Sun, 23 Jan 2000 01:23:45 -0800").unwrap();
        assert_eq!(timestamp, human_readable_explicit);
    }

    #[test]
    fn test_parse_datetime_preserves_explicit_offset() {
        let ts = parse_datetime("1995-12-19T16:39:57-08:00").unwrap();
        let expected: jiff::Timestamp = "1995-12-19T16:39:57-08:00".parse().unwrap();
        assert_eq!(ts.timestamp, MillisSinceEpoch(expected.as_millisecond()));
        assert_eq!(ts.tz_offset, -8 * 60);
    }

    #[test]
    fn test_parse_datetime_handles_z_suffix() {
        let ts = parse_datetime("1995-12-19T16:39:57Z").unwrap();
        let expected: jiff::Timestamp = "1995-12-19T16:39:57Z".parse().unwrap();
        assert_eq!(ts.timestamp, MillisSinceEpoch(expected.as_millisecond()));
        assert_eq!(ts.tz_offset, 0);
    }
}
