use std::fmt::Write as _;
use std::sync::LazyLock;

use jj_lib::backend::Timestamp;
use jj_lib::backend::TimestampOutOfRange;

/// Parsed formatting items which should never contain an error.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FormattingItems {
    format: String,
}

impl FormattingItems {
    /// Parses strftime-like format string.
    pub fn parse(format: &str) -> Option<Self> {
        // validate the format string by trying to format a dummy timestamp
        let dummy_ts = jiff::Timestamp::UNIX_EPOCH;
        let dummy_zoned = dummy_ts.to_zoned(jiff::tz::TimeZone::UTC);
        // note that the usage of a `String` is load-bearing; using an `io::sink` will
        // not result in `write!` returning an error even if the format string
        // is invalid.
        let mut buf = String::new();
        write!(buf, "{}", dummy_zoned.strftime(format)).ok()?;

        Some(Self {
            format: format.to_owned(),
        })
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
    let zoned = timestamp.to_zoned()?;
    // jj's Timestamp only carries a numeric UTC offset, not an IANA timezone
    // name. Replace %Z (timezone abbreviation) with %:z (numeric offset) to
    // match chrono's FixedOffset behavior, where %Z always output the numeric
    // offset like "+00:00" rather than an abbreviation like "UTC".
    let format = format.format.replace("%Z", "%:z");
    Ok(zoned.strftime(&format).to_string())
}

pub fn format_duration(
    from: &Timestamp,
    to: &Timestamp,
    format: &timeago::Formatter,
) -> Result<String, TimestampOutOfRange> {
    let from_zoned = from.to_zoned()?;
    let to_zoned = to.to_zoned()?;
    let span = to_zoned
        .since(&from_zoned)
        .map_err(|_| TimestampOutOfRange)?;
    // convert jiff::Span to std::time::Duration via jiff::SignedDuration
    let signed_duration = span
        .to_duration(&to_zoned)
        .map_err(|_| TimestampOutOfRange)?;
    let duration: std::time::Duration = signed_duration
        .try_into()
        .map_err(|_| TimestampOutOfRange)?;
    Ok(format.convert(duration))
}
