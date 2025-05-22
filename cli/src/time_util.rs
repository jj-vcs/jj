use jiff::fmt::strtime;
use jiff::tz::Offset;
use jiff::tz::TimeZone;
use jiff::Zoned;
use jj_lib::backend::Timestamp;

fn datetime_from_timestamp(context: &Timestamp) -> Result<Zoned, jiff::Error> {
    Ok(
        jiff::Timestamp::from_millisecond(context.timestamp.0)?.to_zoned(TimeZone::fixed(
            Offset::constant((context.tz_offset / 60).try_into().unwrap_or_default()),
        )),
    )
}

pub fn format_absolute_timestamp(timestamp: &Timestamp) -> Result<String, jiff::Error> {
    const DEFAULT_FORMAT: &str = "%Y-%m-%d %H:%M:%S.%3f %:z";
    format_absolute_timestamp_with(timestamp, DEFAULT_FORMAT)
}

pub fn format_absolute_timestamp_with(
    timestamp: &Timestamp,
    format: &str,
) -> Result<String, jiff::Error> {
    let datetime = datetime_from_timestamp(timestamp)?;
    strtime::format(format, &datetime)
}

pub fn format_duration(
    from: &Timestamp,
    to: &Timestamp,
    format: &timeago::Formatter,
) -> Result<String, jiff::Error> {
    let duration = datetime_from_timestamp(to)?
        .duration_since(&datetime_from_timestamp(from)?)
        .unsigned_abs();
    Ok(format.convert(duration))
}
