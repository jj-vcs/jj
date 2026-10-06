---
type: fix
---
Date patterns such as `after:"2024-03-10 03:30"` are now resolved with the
correct UTC offset on days with a daylight savings transition. Previously, the
offset from the start of the day was used, so times after were off by an hour.
Likely a niche bug fix, but a bug fix nonetheless!
