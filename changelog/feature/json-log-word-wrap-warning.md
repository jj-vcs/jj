Using the `json` template function with `ui.log-word-wrap` set to `true` now
emits a warning that the output may be broken due to potential wrapping in the
middle of JSON output. Users should set the config option to `false` to avoid
receiving invalid JSON objects on small terminals.
