//! Helper functions for rendering UI components.

use std::{collections::VecDeque, time::Duration};

use crate::{Event, RecordError, RecordInput, TerminalKind};

/// Generate a one-line description of a binary file change.
pub fn make_binary_description(hash: &str, num_bytes: u64) -> String {
    format!("{hash} ({num_bytes} bytes)")
}

/// Reads input events from the terminal using `crossterm`.
///
/// Its default implementation of `edit_commit_message` returns the provided
/// message unchanged.
pub struct CrosstermInput;

impl RecordInput for CrosstermInput {
    fn terminal_kind(&self) -> TerminalKind {
        TerminalKind::Crossterm
    }

    fn next_events(&mut self) -> Result<Vec<Event>, RecordError> {
        // Ensure we block for at least one event.
        let first_event = crossterm::event::read().map_err(RecordError::ReadInput)?;
        let mut events = vec![first_event.into()];
        // Some events, like scrolling, are generated more quickly than
        // we can render the UI. In those cases, batch up all available
        // events and process them before the next render.
        while crossterm::event::poll(Duration::ZERO).map_err(RecordError::ReadInput)? {
            let event = crossterm::event::read().map_err(RecordError::ReadInput)?;
            events.push(event.into());
        }
        Ok(events)
    }

    fn edit_commit_message(&mut self, message: &str) -> Result<String, RecordError> {
        Ok(message.to_owned())
    }
}

/// Reads events from the provided sequence of events.
pub struct TestingInput {
    /// The width of the virtual terminal in columns.
    pub width: usize,

    /// The height of the virtual terminal in columns.
    pub height: usize,

    /// The sequence of events to emit.
    pub events: Box<dyn Iterator<Item = Event>>,

    /// Commit messages to use when the commit editor is opened.
    pub commit_messages: VecDeque<String>,
}

impl TestingInput {
    /// Helper function to construct a `TestingInput`.
    pub fn new(
        width: usize,
        height: usize,
        events: impl IntoIterator<Item = Event> + 'static,
    ) -> Self {
        Self {
            width,
            height,
            events: Box::new(events.into_iter()),
            commit_messages: Default::default(),
        }
    }
}

impl RecordInput for TestingInput {
    fn terminal_kind(&self) -> TerminalKind {
        let Self {
            width,
            height,
            events: _,
            commit_messages: _,
        } = self;
        TerminalKind::Testing {
            width: *width,
            height: *height,
        }
    }

    fn next_events(&mut self) -> Result<Vec<Event>, RecordError> {
        Ok(vec![self.events.next().unwrap_or(Event::None)])
    }

    fn edit_commit_message(&mut self, _message: &str) -> Result<String, RecordError> {
        self.commit_messages
            .pop_front()
            .ok_or_else(|| RecordError::Other("No more commit messages available".to_string()))
    }
}
