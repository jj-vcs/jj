//! Special runtime variables.

/// Upon launch, write a serialized version of the UI state to the file named
/// [`DUMP_UI_STATE_FILENAME`] in the current directory. Only works if compiled
/// with the `debug` feature.
pub const ENV_VAR_DUMP_UI_STATE: &str = "SCM_RECORD_DUMP_UI_STATE";

/// The filename to write to for [`ENV_VAR_DUMP_UI_STATE`].
pub const DUMP_UI_STATE_FILENAME: &str = "scm_record_ui_state.json";

/// Render a debug pane over the file. Only works if compiled with the `debug`
/// feature.
pub const ENV_VAR_DEBUG_UI: &str = "SCM_RECORD_DEBUG_UI";
