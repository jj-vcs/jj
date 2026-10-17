use jj_lib::backend::Timestamp;
use jj_lib::backend::TimestampOutOfRange;

/// Validates the format string by trying to format a dummy timestamp.
pub fn validate_format(format: &str) -> Result<(), jiff::Error> {
    let dummy_ts = jiff::Zoned::UNIX_EPOCH;
    // note that the usage of a `String` is load-bearing; using an `io::sink`
    // will not result in `write!` returning an error even if the format string
    // is invalid.
    jiff::fmt::strtime::format(format, &dummy_ts)?;
    Ok(())
}

pub fn format_absolute_timestamp(timestamp: &Timestamp) -> Result<String, TimestampOutOfRange> {
    const DEFAULT_FORMAT: &str = "%Y-%m-%d %H:%M:%S.%3f %:z";
    format_absolute_timestamp_with(timestamp, DEFAULT_FORMAT)
}

pub fn format_absolute_timestamp_with(
    timestamp: &Timestamp,
    format: &str,
) -> Result<String, TimestampOutOfRange> {
    let zoned = timestamp.to_zoned()?;
    Ok(zoned.strftime(format).to_string())
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
