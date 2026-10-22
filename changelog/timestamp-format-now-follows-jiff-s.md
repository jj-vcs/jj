---
breaking_note: |
  `Timestamp.format()` now follows
  [jiff's strftime syntax](https://docs.rs/jiff/latest/jiff/fmt/strtime/index.html)
  instead of chrono's. Most directives, including the default format, are
  unchanged, but:

  - `%v` and `%+` are no longer supported. Use `%e-%b-%Y` and
    `%Y-%m-%dT%H:%M:%S%.3f%:z` (which always includes milliseconds) instead.
  - `%c`, `%x`, and `%r` produce different output. For example, `%x` now formats
    as `2001 M07 8` rather than `07/08/01`.
  - `%f` now prints only as many digits as needed rather than always 9. Use
    `%9f` or `%N` for the old behavior. Similarly, `%.f` no longer pads to 3, 6,
    or 9 digits, so it formats as `.12` rather than `.120`.
  - `%Z` now prints a time zone abbreviation. Use `%:z` to get an offset.
  - `%:::z` now includes the minutes of an offset when they are non-zero.
  - New directives and flags such as `%N`, `%Q`, `%^a`, and `%5Y` are now
    accepted. `%#z` no longer panics.
  - `json(timestamp)` now prints a UTC time zone offset as `+00:00` instead of
    `Z`.
---
