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
