use jiff::fmt::strtime;
use jj_lib::backend::Timestamp;

pub fn format_absolute_timestamp(timestamp: &Timestamp) -> Result<String, jiff::Error> {
    const DEFAULT_FORMAT: &str = "%Y-%m-%d %H:%M:%S.%3f %:z";
    format_absolute_timestamp_with(timestamp, DEFAULT_FORMAT)
}

pub fn format_absolute_timestamp_with(
    timestamp: &Timestamp,
    format: &str,
) -> Result<String, jiff::Error> {
    let datetime = timestamp.to_zoned()?;
    strtime::format(format, &datetime)
}

pub fn format_duration(
    from: &Timestamp,
    to: &Timestamp,
    format: &timeago::Formatter,
) -> Result<String, jiff::Error> {
    let duration = to
        .to_zoned()?
        .duration_since(&from.to_zoned()?)
        .unsigned_abs();
    Ok(format.convert(duration))
}
