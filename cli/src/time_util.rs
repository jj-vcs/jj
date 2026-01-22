use std::sync::LazyLock;

use chrono::format::StrftimeItems;
use jj_lib::backend::Timestamp;
use jj_lib::backend::TimestampOutOfRange;

/// Parsed formatting items which should never contain an error.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FormattingItems<'a> {
    items: Vec<chrono::format::Item<'a>>,
}

impl<'a> FormattingItems<'a> {
    /// Parses strftime-like format string.
    pub fn parse(format: &'a str) -> Option<Self> {
        // If the parsed format contained an error, format().to_string() would panic.
        let items = StrftimeItems::new(format)
            .map(|item| match item {
                chrono::format::Item::Error => None,
                _ => Some(item),
            })
            .collect::<Option<_>>()?;
        Some(FormattingItems { items })
    }

    pub fn into_owned(self) -> FormattingItems<'static> {
        use chrono::format::Item;
        let items = self
            .items
            .into_iter()
            .map(|item| match item {
                Item::Literal(s) => Item::OwnedLiteral(s.into()),
                Item::OwnedLiteral(s) => Item::OwnedLiteral(s),
                Item::Space(s) => Item::OwnedSpace(s.into()),
                Item::OwnedSpace(s) => Item::OwnedSpace(s),
                Item::Numeric(spec, pad) => Item::Numeric(spec, pad),
                Item::Fixed(spec) => Item::Fixed(spec),
                Item::Error => Item::Error, // shouldn't exist, but just copy
            })
            .collect();
        FormattingItems { items }
    }
}

pub fn format_absolute_timestamp(timestamp: &Timestamp) -> Result<String, TimestampOutOfRange> {
    static DEFAULT_FORMAT: LazyLock<FormattingItems> =
        LazyLock::new(|| FormattingItems::parse("%Y-%m-%d %H:%M:%S.%3f %:z").unwrap());
    format_absolute_timestamp_with(timestamp, &DEFAULT_FORMAT)
}

pub fn format_absolute_timestamp_with(
    timestamp: &Timestamp,
    format: &FormattingItems,
) -> Result<String, TimestampOutOfRange> {
    let datetime = timestamp.to_datetime()?;
    Ok(datetime.format_with_items(format.items.iter()).to_string())
}

pub fn format_duration(
    from: &Timestamp,
    to: &Timestamp,
    format: &timeago::Formatter,
) -> Result<String, TimestampOutOfRange> {
    let duration = to
        .to_datetime()?
        .signed_duration_since(from.to_datetime()?)
        .to_std()
        .map_err(|_: chrono::OutOfRangeError| TimestampOutOfRange)?;
    Ok(format.convert(duration))
}

/// Formats a duration between two timestamps using single-character units.
///
/// Returns strings like "5s", "3m", "2h", "4d", "2w", "3M", "1y".
pub fn format_duration_short(
    from: &Timestamp,
    to: &Timestamp,
) -> Result<String, TimestampOutOfRange> {
    let secs = to
        .to_datetime()?
        .signed_duration_since(from.to_datetime()?)
        .num_seconds()
        .unsigned_abs();

    const MINUTE: u64 = 60;
    const HOUR: u64 = 60 * MINUTE;
    const DAY: u64 = 24 * HOUR;
    const WEEK: u64 = 7 * DAY;
    const MONTH: u64 = 30 * DAY;
    const YEAR: u64 = 365 * DAY;

    let (value, unit) = if secs >= YEAR {
        (secs / YEAR, "y")
    } else if secs >= MONTH {
        (secs / MONTH, "M")
    } else if secs >= WEEK {
        (secs / WEEK, "w")
    } else if secs >= DAY {
        (secs / DAY, "d")
    } else if secs >= HOUR {
        (secs / HOUR, "h")
    } else if secs >= MINUTE {
        (secs / MINUTE, "m")
    } else {
        (secs, "s")
    };

    Ok(format!("{value}{unit}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_timestamp(secs: i64) -> Timestamp {
        Timestamp {
            timestamp: jj_lib::backend::MillisSinceEpoch(secs * 1000),
            tz_offset: 0,
        }
    }

    #[test]
    fn test_format_duration_short_seconds() {
        let from = make_timestamp(0);
        let to = make_timestamp(30);
        assert_eq!(format_duration_short(&from, &to).unwrap(), "30s");
    }

    #[test]
    fn test_format_duration_short_minutes() {
        let from = make_timestamp(0);
        let to = make_timestamp(60);
        assert_eq!(format_duration_short(&from, &to).unwrap(), "1m");

        let to = make_timestamp(5 * 60);
        assert_eq!(format_duration_short(&from, &to).unwrap(), "5m");
    }

    #[test]
    fn test_format_duration_short_hours() {
        let from = make_timestamp(0);
        let to = make_timestamp(60 * 60);
        assert_eq!(format_duration_short(&from, &to).unwrap(), "1h");

        let to = make_timestamp(3 * 60 * 60);
        assert_eq!(format_duration_short(&from, &to).unwrap(), "3h");
    }

    #[test]
    fn test_format_duration_short_days() {
        let from = make_timestamp(0);
        let to = make_timestamp(24 * 60 * 60);
        assert_eq!(format_duration_short(&from, &to).unwrap(), "1d");

        let to = make_timestamp(5 * 24 * 60 * 60);
        assert_eq!(format_duration_short(&from, &to).unwrap(), "5d");
    }

    #[test]
    fn test_format_duration_short_weeks() {
        let from = make_timestamp(0);
        let to = make_timestamp(7 * 24 * 60 * 60);
        assert_eq!(format_duration_short(&from, &to).unwrap(), "1w");

        let to = make_timestamp(3 * 7 * 24 * 60 * 60);
        assert_eq!(format_duration_short(&from, &to).unwrap(), "3w");
    }

    #[test]
    fn test_format_duration_short_months() {
        let from = make_timestamp(0);
        let to = make_timestamp(30 * 24 * 60 * 60);
        assert_eq!(format_duration_short(&from, &to).unwrap(), "1M");

        let to = make_timestamp(6 * 30 * 24 * 60 * 60);
        assert_eq!(format_duration_short(&from, &to).unwrap(), "6M");
    }

    #[test]
    fn test_format_duration_short_years() {
        let from = make_timestamp(0);
        let to = make_timestamp(365 * 24 * 60 * 60);
        assert_eq!(format_duration_short(&from, &to).unwrap(), "1y");

        let to = make_timestamp(3 * 365 * 24 * 60 * 60);
        assert_eq!(format_duration_short(&from, &to).unwrap(), "3y");
    }

    #[test]
    fn test_format_duration_short_zero() {
        let from = make_timestamp(0);
        let to = make_timestamp(0);
        assert_eq!(format_duration_short(&from, &to).unwrap(), "0s");
    }

    #[test]
    fn test_format_duration_short_negative() {
        // Negative durations should use absolute value
        let from = make_timestamp(100);
        let to = make_timestamp(0);
        assert_eq!(format_duration_short(&from, &to).unwrap(), "1m");
    }
}
