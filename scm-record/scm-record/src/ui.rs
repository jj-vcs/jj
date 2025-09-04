//! UI implementation.

use std::any::Any;
use std::borrow::Cow;
use std::cell::RefCell;
use std::cmp::min;
use std::collections::{BTreeMap, HashSet};
use std::fmt::Write;
use std::fmt::{Debug, Display};
use std::hash::Hash;
use std::path::Path;
use std::rc::Rc;
use std::{io, iter, mem, panic};

use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
    MouseButton, MouseEvent, MouseEventKind,
};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, is_raw_mode_enabled, EnterAlternateScreen,
    LeaveAlternateScreen,
};
use ratatui::backend::{Backend, TestBackend};
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::{backend::CrosstermBackend, Terminal};
use tracing::warn;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::consts::ENV_VAR_DEBUG_UI;
use crate::render::{
    centered_rect, Component, DrawnRect, DrawnRects, Mask, Rect, RectSize, Viewport,
};
use crate::types::{ChangeType, Commit, RecordError, RecordState, Tristate};
use crate::util::{IsizeExt, UsizeExt};
use crate::{File, FileMode, Section, SectionChangedLine};

const NUM_CONTEXT_LINES: usize = 3;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, PartialOrd, Ord)]
struct FileKey {
    commit_idx: usize,
    file_idx: usize,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, PartialOrd, Ord)]
struct SectionKey {
    commit_idx: usize,
    file_idx: usize,
    section_idx: usize,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, PartialOrd, Ord)]
struct LineKey {
    commit_idx: usize,
    file_idx: usize,
    section_idx: usize,
    line_idx: usize,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, PartialOrd, Ord)]
enum QuitDialogButtonId {
    Quit,
    GoBack,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, PartialOrd, Ord)]
enum SelectionKey {
    None,
    File(FileKey),
    Section(SectionKey),
    Line(LineKey),
}

impl Default for SelectionKey {
    fn default() -> Self {
        Self::None
    }
}

/// A copy of the contents of the screen at a certain point in time.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TestingScreenshot {
    contents: Rc<RefCell<Option<String>>>,
}

impl TestingScreenshot {
    fn set(&self, new_contents: String) {
        let Self { contents } = self;
        *contents.borrow_mut() = Some(new_contents);
    }

    /// Produce an `Event` which will record the screenshot when it's handled.
    pub fn event(&self) -> Event {
        Event::TakeScreenshot(self.clone())
    }
}

impl Display for TestingScreenshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self { contents } = self;
        match contents.borrow().as_ref() {
            Some(contents) => write!(f, "{contents}"),
            None => write!(f, "<this screenshot was never assigned>"),
        }
    }
}

#[allow(missing_docs)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Event {
    None,
    QuitAccept,
    QuitCancel,
    QuitInterrupt,
    QuitEscape,
    TakeScreenshot(TestingScreenshot),
    Redraw,
    EnsureSelectionInViewport,
    ScrollUp,
    ScrollDown,
    PageUp,
    PageDown,
    FocusPrev,
    /// Move focus to the previous item of the same kind (i.e. file, section, line).
    FocusPrevSameKind,
    FocusPrevPage,
    FocusNext,
    /// Move focus to the next item of the same kind.
    FocusNextSameKind,
    FocusNextPage,
    FocusInner,
    /// If `fold_section` is true, and the current section is expanded, the
    /// section should be collapsed without moving focus. Otherwise, move the
    /// focus outwards.
    FocusOuter {
        fold_section: bool,
    },
    ToggleItem,
    ToggleItemAndAdvance,
    ToggleAll,
    ToggleAllUniform,
    ExpandItem,
    ExpandAll,
    Click {
        row: usize,
        column: usize,
    },
    ToggleCommitViewMode, // no key binding currently
    EditCommitMessage,
    Help,
}

impl From<crossterm::event::Event> for Event {
    fn from(event: crossterm::event::Event) -> Self {
        use crossterm::event::Event;
        match event {
            Event::Key(KeyEvent {
                code: KeyCode::Char('q'),
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: _,
            }) => Self::QuitCancel,

            Event::Key(KeyEvent {
                code: KeyCode::Esc,
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: _,
            }) => Self::QuitEscape,

            Event::Key(KeyEvent {
                code: KeyCode::Char('c'),
                modifiers: KeyModifiers::CONTROL,
                kind: KeyEventKind::Press,
                state: _,
            }) => Self::QuitInterrupt,

            Event::Key(KeyEvent {
                code: KeyCode::Char('c'),
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: _,
            }) => Self::QuitAccept,

            Event::Key(KeyEvent {
                code: KeyCode::Char('?'),
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: _,
            }) => Self::Help,

            Event::Key(KeyEvent {
                code: KeyCode::Up | KeyCode::Char('y'),
                modifiers: KeyModifiers::CONTROL,
                kind: KeyEventKind::Press,
                state: _,
            })
            | Event::Mouse(MouseEvent {
                kind: MouseEventKind::ScrollUp,
                column: _,
                row: _,
                modifiers: _,
            }) => Self::ScrollUp,
            Event::Key(KeyEvent {
                code: KeyCode::Down | KeyCode::Char('e'),
                modifiers: KeyModifiers::CONTROL,
                kind: KeyEventKind::Press,
                state: _,
            })
            | Event::Mouse(MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: _,
                row: _,
                modifiers: _,
            }) => Self::ScrollDown,

            Event::Key(KeyEvent {
                code: KeyCode::PageUp | KeyCode::Char('b'),
                modifiers: KeyModifiers::CONTROL,
                kind: KeyEventKind::Press,
                state: _,
            }) => Self::PageUp,
            Event::Key(KeyEvent {
                code: KeyCode::PageDown | KeyCode::Char('f'),
                modifiers: KeyModifiers::CONTROL,
                kind: KeyEventKind::Press,
                state: _,
            }) => Self::PageDown,

            Event::Key(KeyEvent {
                code: KeyCode::Up | KeyCode::Char('k'),
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: _,
            }) => Self::FocusPrev,
            Event::Key(KeyEvent {
                code: KeyCode::Down | KeyCode::Char('j'),
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: _,
            }) => Self::FocusNext,

            Event::Key(KeyEvent {
                code: KeyCode::PageUp,
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: _,
            }) => Self::FocusPrevSameKind,
            Event::Key(KeyEvent {
                code: KeyCode::PageDown,
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: _,
            }) => Self::FocusNextSameKind,

            Event::Key(KeyEvent {
                code: KeyCode::Left | KeyCode::Char('h'),
                modifiers: KeyModifiers::SHIFT,
                kind: KeyEventKind::Press,
                state: _,
            }) => Self::FocusOuter {
                fold_section: false,
            },
            Event::Key(KeyEvent {
                code: KeyCode::Left | KeyCode::Char('h'),
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: _,
            }) => Self::FocusOuter { fold_section: true },
            Event::Key(KeyEvent {
                code: KeyCode::Right | KeyCode::Char('l'),
                // The shift modifier is accepted for continuity with FocusOuter.
                modifiers: KeyModifiers::NONE | KeyModifiers::SHIFT,
                kind: KeyEventKind::Press,
                state: _,
            }) => Self::FocusInner,

            Event::Key(KeyEvent {
                code: KeyCode::Char('u'),
                modifiers: KeyModifiers::CONTROL,
                kind: KeyEventKind::Press,
                state: _,
            }) => Self::FocusPrevPage,
            Event::Key(KeyEvent {
                code: KeyCode::Char('d'),
                modifiers: KeyModifiers::CONTROL,
                kind: KeyEventKind::Press,
                state: _,
            }) => Self::FocusNextPage,

            Event::Key(KeyEvent {
                code: KeyCode::Char(' '),
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: _,
            }) => Self::ToggleItem,

            Event::Key(KeyEvent {
                code: KeyCode::Enter,
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: _,
            }) => Self::ToggleItemAndAdvance,

            Event::Key(KeyEvent {
                code: KeyCode::Char('a'),
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: _,
            }) => Self::ToggleAll,
            Event::Key(KeyEvent {
                code: KeyCode::Char('A'),
                modifiers: KeyModifiers::SHIFT,
                kind: KeyEventKind::Press,
                state: _,
            }) => Self::ToggleAllUniform,

            Event::Key(KeyEvent {
                code: KeyCode::Char('f'),
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: _,
            }) => Self::ExpandItem,
            Event::Key(KeyEvent {
                code: KeyCode::Char('F'),
                modifiers: KeyModifiers::SHIFT,
                kind: KeyEventKind::Press,
                state: _,
            }) => Self::ExpandAll,

            Event::Key(KeyEvent {
                code: KeyCode::Char('e'),
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: _event,
            }) => Self::EditCommitMessage,

            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column,
                row,
                modifiers: _,
            }) => Self::Click {
                row: row.into(),
                column: column.into(),
            },

            _event => Self::None,
        }
    }
}

/// The terminal backend to use.
pub enum TerminalKind {
    /// Use the `CrosstermBackend` backend.
    Crossterm,

    /// Use the `TestingBackend` backend.
    Testing {
        /// The width of the virtual terminal.
        width: usize,

        /// The height of the virtual terminal.
        height: usize,
    },
}

/// Get user input.
pub trait RecordInput {
    /// Return the kind of terminal to use.
    fn terminal_kind(&self) -> TerminalKind;

    /// Get all available user events. This should block until there is at least
    /// one available event.
    fn next_events(&mut self) -> Result<Vec<Event>, RecordError>;

    /// Open a commit editor and interactively edit the given message.
    ///
    /// This function will only be invoked if one of the provided `Commit`s had
    /// a non-`None` commit message.
    fn edit_commit_message(&mut self, message: &str) -> Result<String, RecordError>;
}

/// Copied from internal implementation of `tui`.
fn buffer_view(buffer: &Buffer) -> String {
    let mut view =
        String::with_capacity(buffer.content.len() + usize::from(buffer.area.height) * 3);
    for cells in buffer.content.chunks(buffer.area.width.into()) {
        let mut overwritten = vec![];
        let mut skip: usize = 0;
        view.push('"');
        for (x, c) in cells.iter().enumerate() {
            if skip == 0 {
                view.push_str(c.symbol());
            } else {
                overwritten.push((x, c.symbol()))
            }
            skip = std::cmp::max(skip, c.symbol().width()).saturating_sub(1);
        }
        view.push('"');
        if !overwritten.is_empty() {
            write!(&mut view, " Hidden by multi-width symbols: {overwritten:?}").unwrap();
        }
        view.push('\n');
    }
    view
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum StateUpdate {
    None,
    SetQuitDialog(Option<QuitDialog>),
    QuitAccept,
    QuitCancel,
    SetHelpDialog(Option<HelpDialog>),
    TakeScreenshot(TestingScreenshot),
    Redraw,
    EnsureSelectionInViewport,
    ScrollTo(isize),
    SelectItem {
        selection_key: SelectionKey,
        ensure_in_viewport: bool,
    },
    ToggleItem(SelectionKey),
    ToggleItemAndAdvance(SelectionKey, SelectionKey),
    ToggleAll,
    ToggleAllUniform,
    SetExpandItem(SelectionKey, bool),
    ToggleExpandItem(SelectionKey),
    ToggleExpandAll,
    UnfocusMenuBar,
    ClickMenu {
        menu_idx: usize,
    },
    ClickMenuItem(Event),
    ToggleCommitViewMode,
    EditCommitMessage {
        commit_idx: usize,
    },
}

#[derive(Clone, Copy, Debug)]
enum CommitViewMode {
    Inline,
    Adjacent,
}

#[allow(clippy::enum_variant_names)]
enum ToggleSideEffects {
    ToggledModeChangeSection(SectionKey, FileMode, FileMode, bool),
    ToggledChangedSection(SectionKey, bool),
    ToggledChangedLine(LineKey, bool),
}

/// UI component to record the user's changes.
pub struct Recorder<'state, 'input> {
    state: RecordState<'state>,
    input: &'input mut dyn RecordInput,
    pending_events: Vec<Event>,
    use_unicode: bool,
    commit_view_mode: CommitViewMode,
    expanded_items: HashSet<SelectionKey>,
    expanded_menu_idx: Option<usize>,
    selection_key: SelectionKey,
    focused_commit_idx: usize,
    quit_dialog: Option<QuitDialog>,
    help_dialog: Option<HelpDialog>,
    scroll_offset_y: isize,
}

impl<'state, 'input> Recorder<'state, 'input> {
    /// Constructor.
    pub fn new(mut state: RecordState<'state>, input: &'input mut dyn RecordInput) -> Self {
        // Ensure that there are at least two commits.
        state.commits.extend(
            iter::repeat_with(Commit::default).take(2_usize.saturating_sub(state.commits.len())),
        );
        if state.commits.len() > 2 {
            unimplemented!("more than two commits");
        }

        let mut recorder = Self {
            state,
            input,
            pending_events: Default::default(),
            use_unicode: true,
            commit_view_mode: CommitViewMode::Inline,
            expanded_items: Default::default(),
            expanded_menu_idx: Default::default(),
            selection_key: SelectionKey::None,
            focused_commit_idx: 0,
            quit_dialog: None,
            help_dialog: None,
            scroll_offset_y: 0,
        };
        recorder.expand_initial_items();
        recorder
    }

    /// Run the terminal user interface and have the user interactively select
    /// changes.
    pub fn run(self) -> Result<RecordState<'state>, RecordError> {
        #[cfg(feature = "debug")]
        if std::env::var_os(crate::consts::ENV_VAR_DUMP_UI_STATE).is_some() {
            let ui_state =
                serde_json::to_string_pretty(&self.state).map_err(RecordError::SerializeJson)?;
            std::fs::write(crate::consts::DUMP_UI_STATE_FILENAME, ui_state)
                .map_err(RecordError::WriteFile)?;
        }

        match self.input.terminal_kind() {
            TerminalKind::Crossterm => self.run_crossterm(),
            TerminalKind::Testing { width, height } => self.run_testing(width, height),
        }
    }

    /// Run the recorder UI using `crossterm` as the backend connected to stdout.
    fn run_crossterm(self) -> Result<RecordState<'state>, RecordError> {
        Self::set_up_crossterm()?;
        Self::install_panic_hook();
        let backend = CrosstermBackend::new(io::stdout());
        let mut term = Terminal::new(backend).map_err(RecordError::SetUpTerminal)?;
        term.clear().map_err(RecordError::RenderFrame)?;
        let result = self.run_inner(&mut term);
        Self::clean_up_crossterm()?;
        result
    }

    fn install_panic_hook() {
        // HACK: installing a global hook here. This could be installed multiple
        // times, and there's no way to uninstall it once we return.
        //
        // The idea is
        // taken from
        // https://github.com/fdehau/tui-rs/blob/fafad6c96109610825aad89c4bba5253e01101ed/examples/panic.rs.
        //
        // For some reason, simply catching the panic, cleaning up, and
        // reraising the panic loses information about where the panic was
        // originally raised, which is frustrating.
        let original_hook = panic::take_hook();
        panic::set_hook(Box::new(move |panic| {
            Self::clean_up_crossterm().unwrap();
            original_hook(panic);
        }));
    }

    fn set_up_crossterm() -> Result<(), RecordError> {
        if !is_raw_mode_enabled().map_err(RecordError::SetUpTerminal)? {
            crossterm::execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)
                .map_err(RecordError::SetUpTerminal)?;
            enable_raw_mode().map_err(RecordError::SetUpTerminal)?;
        }
        Ok(())
    }

    fn clean_up_crossterm() -> Result<(), RecordError> {
        if is_raw_mode_enabled().map_err(RecordError::CleanUpTerminal)? {
            disable_raw_mode().map_err(RecordError::CleanUpTerminal)?;
            crossterm::execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture)
                .map_err(RecordError::CleanUpTerminal)?;
        }
        Ok(())
    }

    fn run_testing(self, width: usize, height: usize) -> Result<RecordState<'state>, RecordError> {
        let backend = TestBackend::new(width.clamp_into_u16(), height.clamp_into_u16());
        let mut term = Terminal::new(backend).map_err(RecordError::SetUpTerminal)?;
        self.run_inner(&mut term)
    }

    fn run_inner(
        mut self,
        term: &mut Terminal<impl Backend + Any>,
    ) -> Result<RecordState<'state>, RecordError> {
        self.selection_key = self.first_selection_key();
        let debug = if cfg!(feature = "debug") {
            std::env::var_os(ENV_VAR_DEBUG_UI).is_some()
        } else {
            false
        };

        'outer: loop {
            let menu_bar = self.make_menu_bar();
            let app = self.make_app(menu_bar.clone(), None);
            let term_height = usize::from(term.get_frame().area().height);

            let mut drawn_rects: Option<DrawnRects<ComponentId>> = None;
            term.draw(|frame| {
                drawn_rects = Some(Viewport::<ComponentId>::render_top_level(
                    frame,
                    0,
                    self.scroll_offset_y,
                    &app,
                ));
            })
            .map_err(RecordError::RenderFrame)?;
            let drawn_rects = drawn_rects.unwrap();

            // Dump debug info. We may need to use information about the
            // rendered app, so we perform a re-render here.
            if debug {
                let debug_info = AppDebugInfo {
                    term_height,
                    scroll_offset_y: self.scroll_offset_y,
                    selection_key: self.selection_key,
                    selection_key_y: self.selection_key_y(&drawn_rects, self.selection_key),
                    drawn_rects: drawn_rects.clone().into_iter().collect(),
                };
                let debug_app = AppView {
                    debug_info: Some(debug_info),
                    ..app.clone()
                };
                term.draw(|frame| {
                    Viewport::<ComponentId>::render_top_level(
                        frame,
                        0,
                        self.scroll_offset_y,
                        &debug_app,
                    );
                })
                .map_err(RecordError::RenderFrame)?;
            }

            let events = if self.pending_events.is_empty() {
                self.input.next_events()?
            } else {
                // FIXME: the pending events should be applied without redrawing
                // the screen, as otherwise there may be a flash of content
                // containing the screen contents before the event is applied.
                mem::take(&mut self.pending_events)
            };
            for event in events {
                match self.handle_event(event, term_height, &drawn_rects, &menu_bar)? {
                    StateUpdate::None => {}
                    StateUpdate::SetQuitDialog(quit_dialog) => {
                        self.quit_dialog = quit_dialog;
                    }
                    StateUpdate::SetHelpDialog(help_dialog) => {
                        self.help_dialog = help_dialog;
                    }
                    StateUpdate::QuitAccept => {
                        if self.help_dialog.is_some() {
                            self.help_dialog = None;
                        } else {
                            break 'outer;
                        }
                    }
                    StateUpdate::QuitCancel => return Err(RecordError::Cancelled),
                    StateUpdate::TakeScreenshot(screenshot) => {
                        let backend: &dyn Any = term.backend();
                        let test_backend = backend
                            .downcast_ref::<TestBackend>()
                            .expect("TakeScreenshot event generated for non-testing backend");
                        screenshot.set(buffer_view(test_backend.buffer()));
                    }
                    StateUpdate::Redraw => {
                        term.clear().map_err(RecordError::RenderFrame)?;
                    }
                    StateUpdate::EnsureSelectionInViewport => {
                        if let Some(scroll_offset_y) =
                            self.ensure_in_viewport(term_height, &drawn_rects, self.selection_key)
                        {
                            self.scroll_offset_y = scroll_offset_y;
                        }
                    }
                    StateUpdate::ScrollTo(scroll_offset_y) => {
                        self.scroll_offset_y = scroll_offset_y.clamp(0, {
                            let DrawnRect { rect, timestamp: _ } = drawn_rects[&ComponentId::App];
                            rect.height.unwrap_isize() - 1
                        });
                    }
                    StateUpdate::SelectItem {
                        selection_key,
                        ensure_in_viewport,
                    } => {
                        self.selection_key = selection_key;
                        self.expand_item_ancestors(selection_key);
                        if ensure_in_viewport {
                            self.pending_events.push(Event::EnsureSelectionInViewport);
                        }
                    }
                    StateUpdate::ToggleItem(selection_key) => {
                        self.toggle_item(selection_key)?;
                    }
                    StateUpdate::ToggleItemAndAdvance(selection_key, new_key) => {
                        self.toggle_item(selection_key)?;
                        self.selection_key = new_key;
                        self.pending_events.push(Event::EnsureSelectionInViewport);
                    }
                    StateUpdate::ToggleAll => {
                        self.toggle_all();
                    }
                    StateUpdate::ToggleAllUniform => {
                        self.toggle_all_uniform();
                    }
                    StateUpdate::SetExpandItem(selection_key, is_expanded) => {
                        self.set_expand_item(selection_key, is_expanded);
                        self.pending_events.push(Event::EnsureSelectionInViewport);
                    }
                    StateUpdate::ToggleExpandItem(selection_key) => {
                        self.toggle_expand_item(selection_key)?;
                        self.pending_events.push(Event::EnsureSelectionInViewport);
                    }
                    StateUpdate::ToggleExpandAll => {
                        self.toggle_expand_all()?;
                        self.pending_events.push(Event::EnsureSelectionInViewport);
                    }
                    StateUpdate::UnfocusMenuBar => {
                        self.unfocus_menu_bar();
                    }
                    StateUpdate::ClickMenu { menu_idx } => {
                        self.click_menu_header(menu_idx);
                    }
                    StateUpdate::ClickMenuItem(event) => {
                        self.click_menu_item(event);
                    }
                    StateUpdate::ToggleCommitViewMode => {
                        self.commit_view_mode = match self.commit_view_mode {
                            CommitViewMode::Inline => CommitViewMode::Adjacent,
                            CommitViewMode::Adjacent => CommitViewMode::Inline,
                        };
                    }
                    StateUpdate::EditCommitMessage { commit_idx } => {
                        self.pending_events.push(Event::Redraw);
                        self.edit_commit_message(commit_idx)?;
                    }
                }
            }
        }

        Ok(self.state)
    }

    fn make_menu_bar(&self) -> MenuBar<'static> {
        MenuBar {
            menus: vec![
                Menu {
                    label: Cow::Borrowed("File"),
                    items: vec![
                        MenuItem {
                            label: Cow::Borrowed("Confirm (c)"),
                            event: Event::QuitAccept,
                        },
                        MenuItem {
                            label: Cow::Borrowed("Quit (q)"),
                            event: Event::QuitCancel,
                        },
                    ],
                },
                Menu {
                    label: Cow::Borrowed("Edit"),
                    items: vec![
                        MenuItem {
                            label: Cow::Borrowed("Edit message (e)"),
                            event: Event::EditCommitMessage,
                        },
                        MenuItem {
                            label: Cow::Borrowed("Toggle current (space)"),
                            event: Event::ToggleItem,
                        },
                        MenuItem {
                            label: Cow::Borrowed("Toggle current and advance (enter)"),
                            event: Event::ToggleItemAndAdvance,
                        },
                        MenuItem {
                            label: Cow::Borrowed("Invert all items (a)"),
                            event: Event::ToggleAll,
                        },
                        MenuItem {
                            label: Cow::Borrowed("Invert all items uniformly (A)"),
                            event: Event::ToggleAllUniform,
                        },
                    ],
                },
                Menu {
                    label: Cow::Borrowed("Select"),
                    items: vec![
                        MenuItem {
                            label: Cow::Borrowed("Previous item (up, k)"),
                            event: Event::FocusPrev,
                        },
                        MenuItem {
                            label: Cow::Borrowed("Next item (down, j)"),
                            event: Event::FocusNext,
                        },
                        MenuItem {
                            label: Cow::Borrowed("Previous item of the same kind (page-up)"),
                            event: Event::FocusPrevSameKind,
                        },
                        MenuItem {
                            label: Cow::Borrowed("Next item of the same kind (page-down)"),
                            event: Event::FocusNextSameKind,
                        },
                        MenuItem {
                            label: Cow::Borrowed(
                                "Outer item without folding (shift-left, shift-h)",
                            ),
                            event: Event::FocusOuter {
                                fold_section: false,
                            },
                        },
                        MenuItem {
                            label: Cow::Borrowed("Outer item with folding (left, h)"),
                            event: Event::FocusOuter { fold_section: true },
                        },
                        MenuItem {
                            label: Cow::Borrowed("Inner item with unfolding (right, l)"),
                            event: Event::FocusInner,
                        },
                        MenuItem {
                            label: Cow::Borrowed("Previous page (ctrl-u)"),
                            event: Event::FocusPrevPage,
                        },
                        MenuItem {
                            label: Cow::Borrowed("Next page (ctrl-d)"),
                            event: Event::FocusNextPage,
                        },
                    ],
                },
                Menu {
                    label: Cow::Borrowed("View"),
                    items: vec![
                        MenuItem {
                            label: Cow::Borrowed("Fold/unfold current (f)"),
                            event: Event::ExpandItem,
                        },
                        MenuItem {
                            label: Cow::Borrowed("Fold/unfold all (F)"),
                            event: Event::ExpandAll,
                        },
                        MenuItem {
                            label: Cow::Borrowed("Scroll up (ctrl-up, ctrl-y)"),
                            event: Event::ScrollUp,
                        },
                        MenuItem {
                            label: Cow::Borrowed("Scroll down (ctrl-down, ctrl-e)"),
                            event: Event::ScrollDown,
                        },
                        MenuItem {
                            label: Cow::Borrowed("Previous page (ctrl-page-up, ctrl-b)"),
                            event: Event::PageUp,
                        },
                        MenuItem {
                            label: Cow::Borrowed("Next page (ctrl-page-down, ctrl-f)"),
                            event: Event::PageDown,
                        },
                    ],
                },
            ],
            expanded_menu_idx: self.expanded_menu_idx,
        }
    }

    fn make_app(
        &'state self,
        menu_bar: MenuBar<'static>,
        debug_info: Option<AppDebugInfo>,
    ) -> AppView<'state> {
        let RecordState {
            is_read_only,
            commits,
            files,
        } = &self.state;
        let commit_views = match self.commit_view_mode {
            CommitViewMode::Inline => {
                vec![CommitView {
                    debug_info: None,
                    commit_message_view: CommitMessageView {
                        commit_idx: self.focused_commit_idx,
                        commit: &commits[self.focused_commit_idx],
                    },
                    file_views: self.make_file_views(
                        self.focused_commit_idx,
                        files,
                        &debug_info,
                        *is_read_only,
                    ),
                }]
            }

            CommitViewMode::Adjacent => commits
                .iter()
                .enumerate()
                .map(|(commit_idx, commit)| CommitView {
                    debug_info: None,
                    commit_message_view: CommitMessageView { commit_idx, commit },
                    file_views: self.make_file_views(commit_idx, files, &debug_info, *is_read_only),
                })
                .collect(),
        };
        AppView {
            debug_info: None,
            menu_bar,
            commit_view_mode: self.commit_view_mode,
            commit_views,
            quit_dialog: self.quit_dialog.clone(),
            help_dialog: self.help_dialog.clone(),
        }
    }

    fn make_file_views(
        &'state self,
        commit_idx: usize,
        files: &'state [File<'state>],
        debug_info: &Option<AppDebugInfo>,
        is_read_only: bool,
    ) -> Vec<FileView<'state>> {
        files
            .iter()
            .enumerate()
            .map(|(file_idx, file)| {
                let file_key = FileKey {
                    commit_idx,
                    file_idx,
                };
                let file_toggled = self.file_tristate(file_key).unwrap();
                let file_expanded = self.file_expanded(file_key);
                let is_focused = match self.selection_key {
                    SelectionKey::None | SelectionKey::Section(_) | SelectionKey::Line(_) => false,
                    SelectionKey::File(selected_file_key) => file_key == selected_file_key,
                };
                FileView {
                    debug: debug_info.is_some(),
                    file_key,
                    toggle_box: TristateBox {
                        use_unicode: self.use_unicode,
                        id: ComponentId::ToggleBox(SelectionKey::File(file_key)),
                        icon_style: TristateIconStyle::Check,
                        tristate: file_toggled,
                        is_focused,
                        is_read_only,
                    },
                    expand_box: TristateBox {
                        use_unicode: self.use_unicode,
                        id: ComponentId::ExpandBox(SelectionKey::File(file_key)),
                        icon_style: TristateIconStyle::Expand,
                        tristate: file_expanded,
                        is_focused,
                        is_read_only: false,
                    },
                    is_header_selected: is_focused,
                    old_path: file.old_path.as_deref(),
                    path: &file.path,
                    section_views: {
                        let mut section_views = Vec::new();
                        let total_num_sections = file.sections.len();
                        let total_num_editable_sections = file
                            .sections
                            .iter()
                            .filter(|section| section.is_editable())
                            .count();

                        let mut line_num = 1;
                        let mut editable_section_num = 0;
                        for (section_idx, section) in file.sections.iter().enumerate() {
                            let section_key = SectionKey {
                                commit_idx,
                                file_idx,
                                section_idx,
                            };
                            let section_toggled = self.section_tristate(section_key).unwrap();
                            let section_expanded = Tristate::from(
                                self.expanded_items
                                    .contains(&SelectionKey::Section(section_key)),
                            );
                            let is_focused = match self.selection_key {
                                SelectionKey::None
                                | SelectionKey::File(_)
                                | SelectionKey::Line(_) => false,
                                SelectionKey::Section(selection_section_key) => {
                                    selection_section_key == section_key
                                }
                            };
                            if section.is_editable() {
                                editable_section_num += 1;
                            }
                            section_views.push(SectionView {
                                use_unicode: self.use_unicode,
                                is_read_only,
                                section_key,
                                toggle_box: TristateBox {
                                    use_unicode: self.use_unicode,
                                    is_read_only,
                                    id: ComponentId::ToggleBox(SelectionKey::Section(section_key)),
                                    tristate: section_toggled,
                                    icon_style: TristateIconStyle::Check,
                                    is_focused,
                                },
                                expand_box: TristateBox {
                                    use_unicode: self.use_unicode,
                                    is_read_only: false,
                                    id: ComponentId::ExpandBox(SelectionKey::Section(section_key)),
                                    tristate: section_expanded,
                                    icon_style: TristateIconStyle::Expand,
                                    is_focused,
                                },
                                selection: match self.selection_key {
                                    SelectionKey::None | SelectionKey::File(_) => None,
                                    SelectionKey::Section(selected_section_key) => {
                                        if selected_section_key == section_key {
                                            Some(SectionSelection::SectionHeader)
                                        } else {
                                            None
                                        }
                                    }
                                    SelectionKey::Line(LineKey {
                                        commit_idx,
                                        file_idx,
                                        section_idx,
                                        line_idx,
                                    }) => {
                                        let selected_section_key = SectionKey {
                                            commit_idx,
                                            file_idx,
                                            section_idx,
                                        };
                                        if selected_section_key == section_key {
                                            Some(SectionSelection::ChangedLine(line_idx))
                                        } else {
                                            None
                                        }
                                    }
                                },
                                total_num_sections,
                                editable_section_num,
                                total_num_editable_sections,
                                section,
                                line_start_num: line_num,
                            });

                            line_num += match section {
                                Section::Unchanged { lines } => lines.len(),
                                Section::Changed { lines } => lines
                                    .iter()
                                    .filter(|changed_line| match changed_line.change_type {
                                        ChangeType::Added => false,
                                        ChangeType::Removed => true,
                                    })
                                    .count(),
                                Section::FileMode { .. } | Section::Binary { .. } => 0,
                            };
                        }
                        section_views
                    },
                }
            })
            .collect()
    }

    fn handle_event(
        &self,
        event: Event,
        term_height: usize,
        drawn_rects: &DrawnRects<ComponentId>,
        menu_bar: &MenuBar,
    ) -> Result<StateUpdate, RecordError> {
        let state_update = match (&self.quit_dialog, event) {
            (_, Event::None) => StateUpdate::None,
            (_, Event::Redraw) => StateUpdate::Redraw,
            (_, Event::EnsureSelectionInViewport) => StateUpdate::EnsureSelectionInViewport,

            (
                _,
                Event::Help
                | Event::QuitEscape
                | Event::QuitCancel
                | Event::ToggleItem
                | Event::ToggleItemAndAdvance,
            ) if self.help_dialog.is_some() => {
                // there is only one button in the help dialog, so 'toggle*' means "click close"
                StateUpdate::SetHelpDialog(None)
            }
            (_, Event::Help) => StateUpdate::SetHelpDialog(Some(HelpDialog())),

            // Confirm the changes.
            (None, Event::QuitAccept) => StateUpdate::QuitAccept,
            // Ignore the confirm action if the quit dialog is open.
            (Some(_), Event::QuitAccept) => StateUpdate::None,

            // Render quit dialog if the user made changes.
            (None, Event::QuitCancel | Event::QuitInterrupt) => {
                let num_commit_messages = self.num_user_commit_messages()?;
                let num_changed_files = self.num_user_file_changes()?;
                if num_commit_messages > 0 || num_changed_files > 0 {
                    StateUpdate::SetQuitDialog(Some(QuitDialog {
                        num_commit_messages,
                        num_changed_files,
                        focused_button: QuitDialogButtonId::Quit,
                    }))
                } else {
                    StateUpdate::QuitCancel
                }
            }
            // If pressing quit again, or escape, while the dialog is open, close it.
            (Some(_), Event::QuitCancel | Event::QuitEscape) => StateUpdate::SetQuitDialog(None),
            // If pressing ctrl-c again wile the dialog is open, force quit.
            (Some(_), Event::QuitInterrupt) => StateUpdate::QuitCancel,
            // Select left quit dialog button.
            (Some(quit_dialog), Event::FocusOuter { .. }) => {
                StateUpdate::SetQuitDialog(Some(QuitDialog {
                    focused_button: QuitDialogButtonId::GoBack,
                    ..quit_dialog.clone()
                }))
            }
            // Select right quit dialog button.
            (Some(quit_dialog), Event::FocusInner) => {
                StateUpdate::SetQuitDialog(Some(QuitDialog {
                    focused_button: QuitDialogButtonId::Quit,
                    ..quit_dialog.clone()
                }))
            }
            // Press the appropriate dialog button.
            (Some(quit_dialog), Event::ToggleItem | Event::ToggleItemAndAdvance) => {
                let QuitDialog {
                    num_commit_messages: _,
                    num_changed_files: _,
                    focused_button,
                } = quit_dialog;
                match focused_button {
                    QuitDialogButtonId::Quit => StateUpdate::QuitCancel,
                    QuitDialogButtonId::GoBack => StateUpdate::SetQuitDialog(None),
                }
            }

            // Disable most keyboard shortcuts while the quit dialog is open.
            (
                Some(_),
                Event::ScrollUp
                | Event::ScrollDown
                | Event::PageUp
                | Event::PageDown
                | Event::FocusPrev
                | Event::FocusNext
                | Event::FocusPrevSameKind
                | Event::FocusNextSameKind
                | Event::FocusPrevPage
                | Event::FocusNextPage
                | Event::ToggleAll
                | Event::ToggleAllUniform
                | Event::ExpandItem
                | Event::ExpandAll
                | Event::EditCommitMessage,
            ) => StateUpdate::None,

            (Some(_) | None, Event::TakeScreenshot(screenshot)) => {
                StateUpdate::TakeScreenshot(screenshot)
            }
            (None, Event::ScrollUp) => {
                StateUpdate::ScrollTo(self.scroll_offset_y.saturating_sub(1))
            }
            (None, Event::ScrollDown) => {
                StateUpdate::ScrollTo(self.scroll_offset_y.saturating_add(1))
            }
            (None, Event::PageUp) => StateUpdate::ScrollTo(
                self.scroll_offset_y
                    .saturating_sub(term_height.unwrap_isize()),
            ),
            (None, Event::PageDown) => StateUpdate::ScrollTo(
                self.scroll_offset_y
                    .saturating_add(term_height.unwrap_isize()),
            ),
            (None, Event::FocusPrev) => {
                let (keys, index) = self.find_selection();
                let selection_key = self.select_prev(&keys, index);
                StateUpdate::SelectItem {
                    selection_key,
                    ensure_in_viewport: true,
                }
            }
            (None, Event::FocusNext) => {
                let (keys, index) = self.find_selection();
                let selection_key = self.select_next(&keys, index);
                StateUpdate::SelectItem {
                    selection_key,
                    ensure_in_viewport: true,
                }
            }
            (None, Event::FocusPrevSameKind) => {
                let selection_key =
                    self.select_prev_or_next_of_same_kind(/*select_previous=*/ true);
                StateUpdate::SelectItem {
                    selection_key,
                    ensure_in_viewport: true,
                }
            }
            (None, Event::FocusNextSameKind) => {
                let selection_key =
                    self.select_prev_or_next_of_same_kind(/*select_previous=*/ false);
                StateUpdate::SelectItem {
                    selection_key,
                    ensure_in_viewport: true,
                }
            }
            (None, Event::FocusPrevPage) => {
                let selection_key = self.select_prev_page(term_height, drawn_rects);
                StateUpdate::SelectItem {
                    selection_key,
                    ensure_in_viewport: true,
                }
            }
            (None, Event::FocusNextPage) => {
                let selection_key = self.select_next_page(term_height, drawn_rects);
                StateUpdate::SelectItem {
                    selection_key,
                    ensure_in_viewport: true,
                }
            }
            (None, Event::FocusOuter { fold_section }) => self.select_outer(fold_section),
            (None, Event::FocusInner) => {
                let selection_key = self.select_inner();
                StateUpdate::SelectItem {
                    selection_key,
                    ensure_in_viewport: true,
                }
            }
            (None, Event::ToggleItem) => StateUpdate::ToggleItem(self.selection_key),
            (None, Event::ToggleItemAndAdvance) => {
                let advanced_key = self.advance_to_next_of_kind();
                StateUpdate::ToggleItemAndAdvance(self.selection_key, advanced_key)
            }
            (None, Event::ToggleAll) => StateUpdate::ToggleAll,
            (None, Event::ToggleAllUniform) => StateUpdate::ToggleAllUniform,
            (None, Event::ExpandItem) => StateUpdate::ToggleExpandItem(self.selection_key),
            (None, Event::ExpandAll) => StateUpdate::ToggleExpandAll,
            (None, Event::EditCommitMessage) => StateUpdate::EditCommitMessage {
                commit_idx: self.focused_commit_idx,
            },

            (_, Event::Click { row, column }) => {
                let component_id = self.find_component_at(drawn_rects, row, column);
                self.click_component(menu_bar, component_id)
            }
            (_, Event::ToggleCommitViewMode) => StateUpdate::ToggleCommitViewMode,

            // generally ignore escape key
            (_, Event::QuitEscape) => StateUpdate::None,
        };
        Ok(state_update)
    }

    fn first_selection_key(&self) -> SelectionKey {
        match self.state.files.iter().enumerate().next() {
            Some((file_idx, _)) => SelectionKey::File(FileKey {
                commit_idx: self.focused_commit_idx,
                file_idx,
            }),
            None => SelectionKey::None,
        }
    }

    fn num_user_commit_messages(&self) -> Result<usize, RecordError> {
        let RecordState {
            files: _,
            commits,
            is_read_only: _,
        } = &self.state;
        Ok(commits
            .iter()
            .map(|commit| {
                let Commit { message } = commit;
                match message {
                    Some(message) if !message.is_empty() => 1,
                    _ => 0,
                }
            })
            .sum())
    }

    fn num_user_file_changes(&self) -> Result<usize, RecordError> {
        let RecordState {
            files,
            commits: _,
            is_read_only: _,
        } = &self.state;
        let mut result = 0;
        for (file_idx, _file) in files.iter().enumerate() {
            match self.file_tristate(FileKey {
                commit_idx: self.focused_commit_idx,
                file_idx,
            })? {
                Tristate::False => {}
                Tristate::Partial | Tristate::True => {
                    result += 1;
                }
            }
        }
        Ok(result)
    }

    fn all_selection_keys(&self) -> Vec<SelectionKey> {
        let mut result = Vec::new();
        for (commit_idx, _) in self.state.commits.iter().enumerate() {
            if commit_idx > 0 {
                // TODO: implement adjacent `CommitView s.
                continue;
            }
            for (file_idx, file) in self.state.files.iter().enumerate() {
                result.push(SelectionKey::File(FileKey {
                    commit_idx,
                    file_idx,
                }));
                for (section_idx, section) in file.sections.iter().enumerate() {
                    match section {
                        Section::Unchanged { .. } => {}
                        Section::Changed { lines } => {
                            result.push(SelectionKey::Section(SectionKey {
                                commit_idx,
                                file_idx,
                                section_idx,
                            }));
                            for (line_idx, _line) in lines.iter().enumerate() {
                                result.push(SelectionKey::Line(LineKey {
                                    commit_idx,
                                    file_idx,
                                    section_idx,
                                    line_idx,
                                }));
                            }
                        }
                        Section::FileMode {
                            is_checked: _,
                            mode: _,
                        }
                        | Section::Binary { .. } => {
                            result.push(SelectionKey::Section(SectionKey {
                                commit_idx,
                                file_idx,
                                section_idx,
                            }));
                        }
                    }
                }
            }
        }
        result
    }

    fn find_selection(&self) -> (Vec<SelectionKey>, Option<usize>) {
        // FIXME: finding the selected key is an O(n) algorithm (instead of O(log(n)) or O(1)).
        let visible_keys: Vec<_> = self
            .all_selection_keys()
            .iter()
            .cloned()
            .filter(|key| match key {
                SelectionKey::None => false,
                SelectionKey::File(_) => true,
                SelectionKey::Section(section_key) => {
                    let file_key = FileKey {
                        commit_idx: section_key.commit_idx,
                        file_idx: section_key.file_idx,
                    };
                    match self.file_expanded(file_key) {
                        Tristate::False => false,
                        Tristate::Partial | Tristate::True => true,
                    }
                }
                SelectionKey::Line(line_key) => {
                    let file_key = FileKey {
                        commit_idx: line_key.commit_idx,
                        file_idx: line_key.file_idx,
                    };
                    let section_key = SectionKey {
                        commit_idx: line_key.commit_idx,
                        file_idx: line_key.file_idx,
                        section_idx: line_key.section_idx,
                    };
                    self.expanded_items.contains(&SelectionKey::File(file_key))
                        && self
                            .expanded_items
                            .contains(&SelectionKey::Section(section_key))
                }
            })
            .collect();
        let index = visible_keys.iter().enumerate().find_map(|(k, v)| {
            if v == &self.selection_key {
                Some(k)
            } else {
                None
            }
        });
        (visible_keys, index)
    }

    fn select_prev(&self, keys: &[SelectionKey], index: Option<usize>) -> SelectionKey {
        match index {
            None => self.first_selection_key(),
            Some(index) => match index.checked_sub(1) {
                Some(index) => keys[index],
                None => {
                    // TODO: this behavior will be wrong if we have keys for each `Commit` (which currently isn't the case).
                    *keys.last().unwrap()
                }
            },
        }
    }

    fn select_next(&self, keys: &[SelectionKey], index: Option<usize>) -> SelectionKey {
        match index {
            None => self.first_selection_key(),
            Some(index) => match keys.get(index + 1) {
                Some(key) => *key,
                None => keys[0],
            },
        }
    }

    // Returns the previous or next SelectionKey of the same kind as the current
    // selection key. If there are no other keys of the same kind, the current
    // key is returned instead. If `select_previous` is true, the previous key
    // is returned. Otherwise, the next key is returned.
    fn select_prev_or_next_of_same_kind(&self, select_previous: bool) -> SelectionKey {
        let (keys, index) = self.find_selection();
        let iterate_keys_with_wrap_around = |i| -> Box<dyn DoubleEndedIterator<Item = _>> {
            let forward_iter = keys[i + 1..] // Skip the current key
                .iter()
                .chain(keys[..i].iter());
            if select_previous {
                Box::new(forward_iter.rev())
            } else {
                Box::new(forward_iter)
            }
        };
        match index {
            None => self.first_selection_key(),
            Some(index) => {
                match iterate_keys_with_wrap_around(index)
                    .find(|k| std::mem::discriminant(*k) == std::mem::discriminant(&keys[index]))
                {
                    None => keys[index],
                    Some(key) => *key,
                }
            }
        }
    }

    fn select_prev_page(
        &self,
        term_height: usize,
        drawn_rects: &DrawnRects<ComponentId>,
    ) -> SelectionKey {
        let (keys, index) = self.find_selection();
        let mut index = match index {
            Some(index) => index,
            None => return SelectionKey::None,
        };

        let original_y = match self.selection_key_y(drawn_rects, self.selection_key) {
            Some(original_y) => original_y,
            None => {
                return SelectionKey::None;
            }
        };
        let target_y = original_y.saturating_sub(term_height.unwrap_isize() / 2);
        while index > 0 {
            index -= 1;
            let selection_key_y = self.selection_key_y(drawn_rects, keys[index]);
            if let Some(selection_key_y) = selection_key_y {
                if selection_key_y <= target_y {
                    break;
                }
            }
        }
        keys[index]
    }

    fn select_next_page(
        &self,
        term_height: usize,
        drawn_rects: &DrawnRects<ComponentId>,
    ) -> SelectionKey {
        let (keys, index) = self.find_selection();
        let mut index = match index {
            Some(index) => index,
            None => return SelectionKey::None,
        };

        let original_y = match self.selection_key_y(drawn_rects, self.selection_key) {
            Some(original_y) => original_y,
            None => return SelectionKey::None,
        };
        let target_y = original_y.saturating_add(term_height.unwrap_isize() / 2);
        while index + 1 < keys.len() {
            index += 1;
            let selection_key_y = self.selection_key_y(drawn_rects, keys[index]);
            if let Some(selection_key_y) = selection_key_y {
                if selection_key_y >= target_y {
                    break;
                }
            }
        }
        keys[index]
    }

    fn select_inner(&self) -> SelectionKey {
        self.all_selection_keys()
            .into_iter()
            .skip_while(|selection_key| selection_key != &self.selection_key)
            .skip(1)
            .find(|selection_key| {
                match (self.selection_key, selection_key) {
                    (SelectionKey::None, _) => true,
                    (_, SelectionKey::None) => false, // shouldn't happen

                    (SelectionKey::File(_), SelectionKey::File(_)) => false,
                    (SelectionKey::File(_), SelectionKey::Section(_)) => true,
                    (SelectionKey::File(_), SelectionKey::Line(_)) => false, // shouldn't happen

                    (SelectionKey::Section(_), SelectionKey::File(_))
                    | (SelectionKey::Section(_), SelectionKey::Section(_)) => false,
                    (SelectionKey::Section(_), SelectionKey::Line(_)) => true,

                    (SelectionKey::Line(_), _) => false,
                }
            })
            .unwrap_or(self.selection_key)
    }

    fn select_outer(&self, fold_section: bool) -> StateUpdate {
        match self.selection_key {
            SelectionKey::None => StateUpdate::None,
            selection_key @ SelectionKey::File(_) => {
                StateUpdate::SetExpandItem(selection_key, false)
            }
            selection_key @ SelectionKey::Section(SectionKey {
                commit_idx,
                file_idx,
                section_idx: _,
            }) => {
                // If folding is requested and the selection is expanded,
                // collapse it. Otherwise, move the selection to the file.
                if fold_section && self.expanded_items.contains(&selection_key) {
                    StateUpdate::SetExpandItem(selection_key, false)
                } else {
                    StateUpdate::SelectItem {
                        selection_key: SelectionKey::File(FileKey {
                            commit_idx,
                            file_idx,
                        }),
                        ensure_in_viewport: true,
                    }
                }
            }
            SelectionKey::Line(LineKey {
                commit_idx,
                file_idx,
                section_idx,
                line_idx: _,
            }) => StateUpdate::SelectItem {
                selection_key: SelectionKey::Section(SectionKey {
                    commit_idx,
                    file_idx,
                    section_idx,
                }),
                ensure_in_viewport: true,
            },
        }
    }

    fn advance_to_next_of_kind(&self) -> SelectionKey {
        let (keys, index) = self.find_selection();
        let index = match index {
            Some(index) => index,
            None => return SelectionKey::None,
        };
        keys.iter()
            .skip(index + 1)
            .copied()
            .find(|key| match (self.selection_key, key) {
                (SelectionKey::None, _)
                | (SelectionKey::File(_), SelectionKey::File(_))
                | (SelectionKey::Section(_), SelectionKey::Section(_))
                | (SelectionKey::Line(_), SelectionKey::Line(_)) => true,
                (
                    SelectionKey::File(_),
                    SelectionKey::None | SelectionKey::Section(_) | SelectionKey::Line(_),
                )
                | (
                    SelectionKey::Section(_),
                    SelectionKey::None | SelectionKey::File(_) | SelectionKey::Line(_),
                )
                | (
                    SelectionKey::Line(_),
                    SelectionKey::None | SelectionKey::File(_) | SelectionKey::Section(_),
                ) => false,
            })
            .unwrap_or(self.selection_key)
    }

    fn selection_key_y(
        &self,
        drawn_rects: &DrawnRects<ComponentId>,
        selection_key: SelectionKey,
    ) -> Option<isize> {
        let rect = self.selection_rect(drawn_rects, selection_key)?;
        Some(rect.y)
    }

    fn selection_rect(
        &self,
        drawn_rects: &DrawnRects<ComponentId>,
        selection_key: SelectionKey,
    ) -> Option<Rect> {
        let id = match selection_key {
            SelectionKey::None => return None,
            SelectionKey::File(_) | SelectionKey::Section(_) | SelectionKey::Line(_) => {
                ComponentId::SelectableItem(selection_key)
            }
        };
        match drawn_rects.get(&id) {
            Some(DrawnRect { rect, timestamp: _ }) => Some(*rect),
            None => {
                if cfg!(debug_assertions) {
                    panic!(
                        "could not look up drawn rect for component with ID {id:?}; was it drawn?"
                    )
                } else {
                    warn!(component_id = ?id, "could not look up drawn rect for component; was it drawn?");
                    None
                }
            }
        }
    }

    fn ensure_in_viewport(
        &self,
        term_height: usize,
        drawn_rects: &DrawnRects<ComponentId>,
        selection_key: SelectionKey,
    ) -> Option<isize> {
        let menu_bar_height = 1;
        let sticky_file_header_height = match selection_key {
            SelectionKey::None | SelectionKey::File(_) => 0,
            SelectionKey::Section(_) | SelectionKey::Line(_) => 1,
        };
        let top_margin = sticky_file_header_height + menu_bar_height;

        let viewport_top_y = self.scroll_offset_y + top_margin;
        let viewport_height = term_height.unwrap_isize() - top_margin;
        let viewport_bottom_y = viewport_top_y + viewport_height;

        let selection_rect = self.selection_rect(drawn_rects, selection_key)?;
        let selection_top_y = selection_rect.y;
        let selection_height = selection_rect.height.unwrap_isize();
        let selection_bottom_y = selection_top_y + selection_height;

        // Idea: scroll the entire component into the viewport, not just the
        // first line, if possible. If the entire component is smaller than
        // the viewport, then we scroll only enough so that the entire
        // component becomes visible, i.e. align the component's bottom edge
        // with the viewport's bottom edge. Otherwise, we scroll such that
        // the component's top edge is aligned with the viewport's top edge.
        //
        // FIXME: if we scroll up from below, we would want to align the top
        // edge of the component, not the bottom edge. Thus, we should also
        // accept the previous `SelectionKey` and use that when making the
        // decision of where to scroll.
        let result = if viewport_top_y <= selection_top_y && selection_bottom_y < viewport_bottom_y
        {
            // Component is completely within the viewport, no need to scroll.
            self.scroll_offset_y
        } else if (
            // Component doesn't fit in the viewport; just render the top.
            selection_height >= viewport_height
        ) || (
            // Component is at least partially above the viewport.
            selection_top_y < viewport_top_y
        ) {
            selection_top_y - top_margin
        } else {
            // Component is at least partially below the viewport. Want to satisfy:
            // scroll_offset_y + term_height == rect_bottom_y
            selection_bottom_y - top_margin - viewport_height
        };
        Some(result)
    }

    fn find_component_at(
        &self,
        drawn_rects: &DrawnRects<ComponentId>,
        row: usize,
        column: usize,
    ) -> ComponentId {
        let x = column.unwrap_isize();
        let y = row.unwrap_isize() + self.scroll_offset_y;
        drawn_rects
            .iter()
            .filter(|(id, drawn_rect)| {
                let DrawnRect { rect, timestamp: _ } = drawn_rect;
                rect.contains_point(x, y)
                    && match id {
                        ComponentId::App
                        | ComponentId::AppFiles
                        | ComponentId::MenuHeader
                        | ComponentId::CommitMessageView => false,
                        ComponentId::MenuBar
                        | ComponentId::MenuItem(_)
                        | ComponentId::Menu(_)
                        | ComponentId::CommitEditMessageButton(_)
                        | ComponentId::FileViewHeader(_)
                        | ComponentId::SelectableItem(_)
                        | ComponentId::ToggleBox(_)
                        | ComponentId::ExpandBox(_)
                        | ComponentId::HelpDialog
                        | ComponentId::HelpDialogQuitButton
                        | ComponentId::QuitDialog
                        | ComponentId::QuitDialogButton(_) => true,
                    }
            })
            .max_by_key(|(id, rect)| {
                let DrawnRect { rect: _, timestamp } = rect;
                (timestamp, *id)
            })
            .map(|(id, _rect)| *id)
            .unwrap_or(ComponentId::App)
    }

    fn click_component(&self, menu_bar: &MenuBar, component_id: ComponentId) -> StateUpdate {
        match component_id {
            ComponentId::App
            | ComponentId::AppFiles
            | ComponentId::MenuHeader
            | ComponentId::CommitMessageView
            | ComponentId::QuitDialog => StateUpdate::None,
            ComponentId::MenuBar => StateUpdate::UnfocusMenuBar,
            ComponentId::Menu(section_idx) => StateUpdate::ClickMenu {
                menu_idx: section_idx,
            },
            ComponentId::MenuItem(item_idx) => {
                StateUpdate::ClickMenuItem(self.get_menu_item_event(menu_bar, item_idx))
            }
            ComponentId::CommitEditMessageButton(commit_idx) => {
                StateUpdate::EditCommitMessage { commit_idx }
            }
            ComponentId::FileViewHeader(file_key) => StateUpdate::SelectItem {
                selection_key: SelectionKey::File(file_key),
                ensure_in_viewport: false,
            },
            ComponentId::SelectableItem(selection_key) => StateUpdate::SelectItem {
                selection_key,
                ensure_in_viewport: false,
            },
            ComponentId::ToggleBox(selection_key) => {
                if self.selection_key == selection_key {
                    StateUpdate::ToggleItem(selection_key)
                } else {
                    StateUpdate::SelectItem {
                        selection_key,
                        ensure_in_viewport: false,
                    }
                }
            }
            ComponentId::ExpandBox(selection_key) => {
                if self.selection_key == selection_key {
                    StateUpdate::ToggleExpandItem(selection_key)
                } else {
                    StateUpdate::SelectItem {
                        selection_key,
                        ensure_in_viewport: false,
                    }
                }
            }
            ComponentId::QuitDialogButton(QuitDialogButtonId::GoBack) => {
                StateUpdate::SetQuitDialog(None)
            }
            ComponentId::QuitDialogButton(QuitDialogButtonId::Quit) => StateUpdate::QuitCancel,
            ComponentId::HelpDialog => StateUpdate::None,
            ComponentId::HelpDialogQuitButton => StateUpdate::SetHelpDialog(None),
        }
    }

    fn get_menu_item_event(&self, menu_bar: &MenuBar, item_idx: usize) -> Event {
        let MenuBar {
            menus,
            expanded_menu_idx,
        } = menu_bar;
        let menu_idx = match expanded_menu_idx {
            Some(section_idx) => section_idx,
            None => {
                warn!(?item_idx, "Clicking menu item when no menu is expanded");
                return Event::None;
            }
        };
        let menu = match menus.get(*menu_idx) {
            Some(menu) => menu,
            None => {
                warn!(?menu_idx, "Clicking out-of-bounds menu");
                return Event::None;
            }
        };
        let item = match menu.items.get(item_idx) {
            Some(item) => item,
            None => {
                warn!(
                    ?menu_idx,
                    ?item_idx,
                    "Clicking menu bar section item that is out of bounds"
                );
                return Event::None;
            }
        };
        item.event.clone()
    }

    fn toggle_item(&mut self, selection: SelectionKey) -> Result<(), RecordError> {
        if self.state.is_read_only {
            return Ok(());
        }

        let side_effects = match selection {
            SelectionKey::None => None,
            SelectionKey::File(file_key) => {
                let tristate = self.file_tristate(file_key)?;
                let is_checked_new = match tristate {
                    Tristate::False => true,
                    Tristate::Partial | Tristate::True => false,
                };
                self.visit_file(file_key, |file| {
                    file.set_checked(is_checked_new);
                })?;

                None
            }
            SelectionKey::Section(section_key) => {
                let tristate = self.section_tristate(section_key)?;
                let is_checked_new = match tristate {
                    Tristate::False => true,
                    Tristate::Partial | Tristate::True => false,
                };

                let old_file_mode = self.visit_file_for_section(section_key, |f| f.file_mode)?;

                self.visit_section(section_key, |section| {
                    section.set_checked(is_checked_new);

                    if let Section::FileMode { mode, .. } = section {
                        return Some(ToggleSideEffects::ToggledModeChangeSection(
                            section_key,
                            old_file_mode,
                            *mode,
                            is_checked_new,
                        ));
                    }

                    if let Section::Changed { .. } = section {
                        return Some(ToggleSideEffects::ToggledChangedSection(
                            section_key,
                            is_checked_new,
                        ));
                    }

                    None
                })?
            }
            SelectionKey::Line(line_key) => self.visit_line(line_key, |line| {
                line.is_checked = !line.is_checked;

                Some(ToggleSideEffects::ToggledChangedLine(
                    line_key,
                    line.is_checked,
                ))
            })?,
        };

        if let Some(side_effects) = side_effects {
            match side_effects {
                ToggleSideEffects::ToggledModeChangeSection(
                    section_key,
                    old_mode,
                    new_mode,
                    toggled_to,
                ) => {
                    // If we check a deletion, all lines in the file must be deleted
                    if toggled_to && new_mode == FileMode::Absent {
                        self.visit_file_for_section(section_key, |file| {
                            for section in &mut file.sections {
                                if matches!(section, Section::Changed { .. }) {
                                    section.set_checked(true);
                                }
                            }
                        })?;
                    }

                    // If we uncheck a creation, no lines in the file can be added
                    if !toggled_to && old_mode == FileMode::Absent {
                        self.visit_file_for_section(section_key, |file| {
                            for section in &mut file.sections {
                                section.set_checked(false);
                            }
                        })?;
                    }
                }
                ToggleSideEffects::ToggledChangedSection(section_key, toggled_to) => {
                    self.visit_file_for_section(section_key, |file| {
                        for section in &mut file.sections {
                            if let Section::FileMode { mode, is_checked } = section {
                                // If we removed a line and the file was being deleted, it can no longer
                                // be deleted as it needs to contain that line
                                if !toggled_to && *mode == FileMode::Absent {
                                    *is_checked = false;
                                }

                                // If we added a line and the file was not being created, it must be created
                                // in order to contain that line
                                if toggled_to && file.file_mode == FileMode::Absent {
                                    *is_checked = true;
                                }
                            }
                        }
                    })?;
                }
                ToggleSideEffects::ToggledChangedLine(line_key, toggled_to) => {
                    self.visit_file_for_line(line_key, |file| {
                        for section in &mut file.sections {
                            if let Section::FileMode { mode, is_checked } = section {
                                // If we removed a line and the file was being deleted, it can no longer
                                // be deleted as it needs to contain that line
                                if !toggled_to && *mode == FileMode::Absent {
                                    *is_checked = false;
                                }

                                // If we added a line and the file was not being created, it must be created
                                // in order to contain that line
                                if toggled_to && file.file_mode == FileMode::Absent {
                                    *is_checked = true;
                                }
                            }
                        }
                    })?;
                }
            }
        };

        Ok(())
    }

    fn toggle_all(&mut self) {
        if self.state.is_read_only {
            return;
        }

        for file in &mut self.state.files {
            file.toggle_all();
        }
    }

    fn toggle_all_uniform(&mut self) {
        if self.state.is_read_only {
            return;
        }

        let checked = {
            let tristate = self
                .state
                .files
                .iter()
                .map(|file| file.tristate())
                .fold(None, |acc, elem| match (acc, elem) {
                    (None, tristate) => Some(tristate),
                    (Some(acc_tristate), tristate) if acc_tristate == tristate => Some(tristate),
                    _ => Some(Tristate::Partial),
                })
                .unwrap_or(Tristate::False);
            match tristate {
                Tristate::False | Tristate::Partial => true,
                Tristate::True => false,
            }
        };
        for file in &mut self.state.files {
            file.set_checked(checked);
        }
    }

    fn expand_item_ancestors(&mut self, selection: SelectionKey) {
        match selection {
            SelectionKey::None | SelectionKey::File(_) => {}
            SelectionKey::Section(SectionKey {
                commit_idx,
                file_idx,
                section_idx: _,
            }) => {
                self.expanded_items.insert(SelectionKey::File(FileKey {
                    commit_idx,
                    file_idx,
                }));
            }
            SelectionKey::Line(LineKey {
                commit_idx,
                file_idx,
                section_idx,
                line_idx: _,
            }) => {
                self.expanded_items.insert(SelectionKey::File(FileKey {
                    commit_idx,
                    file_idx,
                }));
                self.expanded_items
                    .insert(SelectionKey::Section(SectionKey {
                        commit_idx,
                        file_idx,
                        section_idx,
                    }));
            }
        }
    }

    fn set_expand_item(&mut self, selection: SelectionKey, is_expanded: bool) {
        if is_expanded {
            self.expanded_items.insert(selection);
        } else {
            self.expanded_items.remove(&selection);
        }
    }

    fn toggle_expand_item(&mut self, selection: SelectionKey) -> Result<(), RecordError> {
        match selection {
            SelectionKey::None => {}
            SelectionKey::File(file_key) => {
                if !self.expanded_items.insert(SelectionKey::File(file_key)) {
                    self.expanded_items.remove(&SelectionKey::File(file_key));
                }
            }
            SelectionKey::Section(section_key) => {
                if !self
                    .expanded_items
                    .insert(SelectionKey::Section(section_key))
                {
                    self.expanded_items
                        .remove(&SelectionKey::Section(section_key));
                }
            }
            SelectionKey::Line(_) => {
                // Do nothing.
            }
        }
        Ok(())
    }

    fn expand_initial_items(&mut self) {
        self.expanded_items = self
            .all_selection_keys()
            .into_iter()
            .filter(|selection_key| match selection_key {
                SelectionKey::None | SelectionKey::File(_) | SelectionKey::Line(_) => false,
                SelectionKey::Section(_) => true,
            })
            .collect();
    }

    fn toggle_expand_all(&mut self) -> Result<(), RecordError> {
        let all_selection_keys: HashSet<_> = self.all_selection_keys().into_iter().collect();
        self.expanded_items = if self.expanded_items == all_selection_keys {
            // Select an ancestor file key that will still be visible.
            self.selection_key = match self.selection_key {
                selection_key @ (SelectionKey::None | SelectionKey::File(_)) => selection_key,
                SelectionKey::Section(SectionKey {
                    commit_idx,
                    file_idx,
                    section_idx: _,
                })
                | SelectionKey::Line(LineKey {
                    commit_idx,
                    file_idx,
                    section_idx: _,
                    line_idx: _,
                }) => SelectionKey::File(FileKey {
                    commit_idx,
                    file_idx,
                }),
            };
            Default::default()
        } else {
            all_selection_keys
        };
        Ok(())
    }

    fn unfocus_menu_bar(&mut self) {
        self.expanded_menu_idx = None;
    }

    fn click_menu_header(&mut self, menu_idx: usize) {
        let menu_idx = Some(menu_idx);
        self.expanded_menu_idx = if self.expanded_menu_idx == menu_idx {
            None
        } else {
            menu_idx
        };
    }

    fn click_menu_item(&mut self, event: Event) {
        self.expanded_menu_idx = None;
        self.pending_events.push(event);
    }

    fn edit_commit_message(&mut self, commit_idx: usize) -> Result<(), RecordError> {
        let message = &mut self.state.commits[commit_idx].message;
        let message_str = match message.as_ref() {
            Some(message) => message,
            None => return Ok(()),
        };
        let new_message = {
            match self.input.terminal_kind() {
                TerminalKind::Testing { .. } => {}
                TerminalKind::Crossterm => {
                    Self::clean_up_crossterm()?;
                }
            }
            let result = self.input.edit_commit_message(message_str);
            match self.input.terminal_kind() {
                TerminalKind::Testing { .. } => {}
                TerminalKind::Crossterm => {
                    Self::set_up_crossterm()?;
                }
            }
            result?
        };
        *message = Some(new_message);
        Ok(())
    }

    fn file(&self, file_key: FileKey) -> Result<&File, RecordError> {
        let FileKey {
            commit_idx: _,
            file_idx,
        } = file_key;
        match self.state.files.get(file_idx) {
            Some(file) => Ok(file),
            None => Err(RecordError::Bug(format!(
                "Out-of-bounds file key: {file_key:?}"
            ))),
        }
    }

    fn section(&self, section_key: SectionKey) -> Result<&Section, RecordError> {
        let SectionKey {
            commit_idx,
            file_idx,
            section_idx,
        } = section_key;
        let file = self.file(FileKey {
            commit_idx,
            file_idx,
        })?;
        match file.sections.get(section_idx) {
            Some(section) => Ok(section),
            None => Err(RecordError::Bug(format!(
                "Out-of-bounds section key: {section_key:?}"
            ))),
        }
    }

    fn visit_file_for_section<T>(
        &mut self,
        section_key: SectionKey,
        f: impl Fn(&mut File) -> T,
    ) -> Result<T, RecordError> {
        let SectionKey {
            commit_idx: _,
            file_idx,
            section_idx: _,
        } = section_key;

        match self.state.files.get_mut(file_idx) {
            Some(file) => Ok(f(file)),
            None => Err(RecordError::Bug(format!(
                "Out-of-bounds file key: {file_idx:?}"
            ))),
        }
    }

    fn visit_file_for_line<T>(
        &mut self,
        line_key: LineKey,
        f: impl Fn(&mut File) -> T,
    ) -> Result<T, RecordError> {
        let LineKey {
            commit_idx: _,
            file_idx,
            section_idx: _,
            line_idx: _,
        } = line_key;

        match self.state.files.get_mut(file_idx) {
            Some(file) => Ok(f(file)),
            None => Err(RecordError::Bug(format!(
                "Out-of-bounds file key: {file_idx:?}"
            ))),
        }
    }

    fn visit_file<T>(
        &mut self,
        file_key: FileKey,
        f: impl Fn(&mut File) -> T,
    ) -> Result<T, RecordError> {
        let FileKey {
            commit_idx: _,
            file_idx,
        } = file_key;
        match self.state.files.get_mut(file_idx) {
            Some(file) => Ok(f(file)),
            None => Err(RecordError::Bug(format!(
                "Out-of-bounds file key: {file_key:?}"
            ))),
        }
    }

    fn file_tristate(&self, file_key: FileKey) -> Result<Tristate, RecordError> {
        let file = self.file(file_key)?;
        Ok(file.tristate())
    }

    fn file_expanded(&self, file_key: FileKey) -> Tristate {
        let is_expanded = self.expanded_items.contains(&SelectionKey::File(file_key));
        if !is_expanded {
            Tristate::False
        } else {
            let any_section_unexpanded = self
                .file(file_key)
                .unwrap()
                .sections
                .iter()
                .enumerate()
                .any(|(section_idx, section)| {
                    match section {
                        Section::Unchanged { .. }
                        | Section::FileMode { .. }
                        | Section::Binary { .. } => {
                            // Not collapsible/expandable.
                            false
                        }
                        Section::Changed { .. } => {
                            let section_key = SectionKey {
                                commit_idx: file_key.commit_idx,
                                file_idx: file_key.file_idx,
                                section_idx,
                            };
                            !self
                                .expanded_items
                                .contains(&SelectionKey::Section(section_key))
                        }
                    }
                });
            if any_section_unexpanded {
                Tristate::Partial
            } else {
                Tristate::True
            }
        }
    }

    fn visit_section<T>(
        &mut self,
        section_key: SectionKey,
        f: impl Fn(&mut Section) -> T,
    ) -> Result<T, RecordError> {
        let SectionKey {
            commit_idx: _,
            file_idx,
            section_idx,
        } = section_key;
        let file = match self.state.files.get_mut(file_idx) {
            Some(file) => file,
            None => {
                return Err(RecordError::Bug(format!(
                    "Out-of-bounds file for section key: {section_key:?}"
                )));
            }
        };
        match file.sections.get_mut(section_idx) {
            Some(section) => Ok(f(section)),
            None => Err(RecordError::Bug(format!(
                "Out-of-bounds section key: {section_key:?}"
            ))),
        }
    }

    fn section_tristate(&self, section_key: SectionKey) -> Result<Tristate, RecordError> {
        let section = self.section(section_key)?;
        Ok(section.tristate())
    }

    fn visit_line<T>(
        &mut self,
        line_key: LineKey,
        f: impl FnOnce(&mut SectionChangedLine) -> Option<T>,
    ) -> Result<Option<T>, RecordError> {
        let LineKey {
            commit_idx: _,
            file_idx,
            section_idx,
            line_idx,
        } = line_key;
        let section = &mut self.state.files[file_idx].sections[section_idx];
        match section {
            Section::Changed { lines } => {
                let line = &mut lines[line_idx];
                Ok(f(line))
            }
            Section::Unchanged { .. } | Section::FileMode { .. } | Section::Binary { .. } => {
                // Do nothing.
                Ok(None)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, PartialOrd, Ord)]
enum ComponentId {
    App,
    AppFiles,
    MenuBar,
    MenuHeader,
    Menu(usize),
    MenuItem(usize),
    CommitMessageView,
    CommitEditMessageButton(usize),
    FileViewHeader(FileKey),
    SelectableItem(SelectionKey),
    ToggleBox(SelectionKey),
    ExpandBox(SelectionKey),
    QuitDialog,
    QuitDialogButton(QuitDialogButtonId),
    HelpDialog,
    HelpDialogQuitButton,
}

#[derive(Clone, Debug)]
enum TristateIconStyle {
    Check,
    Expand,
}

#[derive(Clone, Debug)]
struct TristateBox<Id> {
    use_unicode: bool,
    id: Id,
    tristate: Tristate,
    icon_style: TristateIconStyle,
    is_focused: bool,
    is_read_only: bool,
}

impl<Id> TristateBox<Id> {
    fn text(&self) -> String {
        let Self {
            use_unicode,
            id: _,
            tristate,
            icon_style,
            is_focused,
            is_read_only,
        } = self;

        let (l, r) = match (is_read_only, is_focused) {
            (true, _) => ("<", ">"),
            (false, false) => ("[", "]"),
            (false, true) => ("(", ")"),
        };

        let inner = match (icon_style, tristate, use_unicode) {
            (TristateIconStyle::Expand, Tristate::False, _) => "+",
            (TristateIconStyle::Expand, Tristate::True, _) => "-",
            (TristateIconStyle::Expand, Tristate::Partial, false) => "~",
            (TristateIconStyle::Expand, Tristate::Partial, true) => "±",

            (TristateIconStyle::Check, Tristate::False, false) => " ",
            (TristateIconStyle::Check, Tristate::True, false) => "*",
            (TristateIconStyle::Check, Tristate::Partial, false) => "~",

            (TristateIconStyle::Check, Tristate::False, true) => " ",
            (TristateIconStyle::Check, Tristate::True, true) => "●",
            (TristateIconStyle::Check, Tristate::Partial, true) => "◐",
        };
        format!("{l}{inner}{r}")
    }
}

impl<Id: Clone + Debug + Eq + Hash> Component for TristateBox<Id> {
    type Id = Id;

    fn id(&self) -> Self::Id {
        self.id.clone()
    }

    fn draw(&self, viewport: &mut Viewport<Self::Id>, x: isize, y: isize) {
        let style = if self.is_read_only {
            Style::default().fg(Color::Gray).add_modifier(Modifier::DIM)
        } else {
            Style::default().add_modifier(Modifier::BOLD)
        };
        let span = Span::styled(self.text(), style);
        viewport.draw_span(x, y, &span);
    }
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
struct AppDebugInfo {
    term_height: usize,
    scroll_offset_y: isize,
    selection_key: SelectionKey,
    selection_key_y: Option<isize>,
    drawn_rects: BTreeMap<ComponentId, DrawnRect>, // sorted for determinism
}

#[derive(Clone, Debug)]
struct AppView<'a> {
    debug_info: Option<AppDebugInfo>,
    menu_bar: MenuBar<'a>,
    commit_view_mode: CommitViewMode,
    commit_views: Vec<CommitView<'a>>,
    quit_dialog: Option<QuitDialog>,
    help_dialog: Option<HelpDialog>,
}

impl Component for AppView<'_> {
    type Id = ComponentId;

    fn id(&self) -> Self::Id {
        ComponentId::App
    }

    fn draw(&self, viewport: &mut Viewport<Self::Id>, x: isize, _y: isize) {
        let Self {
            debug_info,
            menu_bar,
            commit_view_mode,
            commit_views,
            quit_dialog,
            help_dialog,
        } = self;

        if let Some(debug_info) = debug_info {
            viewport.debug(format!("app debug info: {debug_info:#?}"));
        }

        let viewport_rect = viewport.mask_rect();

        let menu_bar_height = 1usize;
        let commit_view_width = match commit_view_mode {
            CommitViewMode::Inline => viewport.rect().width,
            CommitViewMode::Adjacent => {
                const MAX_COMMIT_VIEW_WIDTH: usize = 120;
                MAX_COMMIT_VIEW_WIDTH
                    .min(viewport.rect().width.saturating_sub(CommitView::MARGIN) / 2)
            }
        };
        let commit_views_mask = Mask {
            x: viewport_rect.x,
            y: viewport_rect.y + menu_bar_height.unwrap_isize(),
            width: Some(viewport_rect.width),
            height: None,
        };
        viewport.with_mask(commit_views_mask, |viewport| {
            let mut commit_view_x = 0;
            for commit_view in commit_views {
                let commit_view_mask = Mask {
                    x: commit_views_mask.x + commit_view_x,
                    y: commit_views_mask.y,
                    width: Some(commit_view_width),
                    height: None,
                };
                let commit_view_rect = viewport.with_mask(commit_view_mask, |viewport| {
                    viewport.draw_component(
                        commit_view_x,
                        menu_bar_height.unwrap_isize(),
                        commit_view,
                    )
                });
                commit_view_x += (CommitView::MARGIN
                    + commit_view_mask.apply(commit_view_rect).width)
                    .unwrap_isize();
            }
        });

        viewport.draw_component(x, viewport_rect.y, menu_bar);

        if let Some(quit_dialog) = quit_dialog {
            viewport.draw_component(0, 0, quit_dialog);
        }
        if let Some(help_dialog) = help_dialog {
            viewport.draw_component(0, 0, help_dialog);
        }
    }
}

#[derive(Clone, Debug)]
struct CommitMessageView<'a> {
    commit_idx: usize,
    commit: &'a Commit,
}

impl Component for CommitMessageView<'_> {
    type Id = ComponentId;

    fn id(&self) -> Self::Id {
        ComponentId::CommitMessageView
    }

    fn draw(&self, viewport: &mut Viewport<Self::Id>, x: isize, y: isize) {
        let Self { commit_idx, commit } = self;
        match commit {
            Commit { message: None } => {}
            Commit {
                message: Some(message),
            } => {
                viewport.draw_blank(Rect {
                    x,
                    y,
                    width: viewport.mask_rect().width,
                    height: 1,
                });
                let y = y + 1;

                let style = Style::default();
                let button_rect = viewport.draw_component(
                    x,
                    y,
                    &Button {
                        id: ComponentId::CommitEditMessageButton(*commit_idx),
                        label: Cow::Borrowed("Edit message"),
                        style,
                        is_focused: false,
                    },
                );
                let divider_rect =
                    viewport.draw_span(button_rect.end_x() + 1, y, &Span::raw(" • "));
                viewport.draw_text(
                    divider_rect.end_x() + 1,
                    y,
                    Span::styled(
                        Cow::Borrowed({
                            let first_line = match message.split_once('\n') {
                                Some((before, _after)) => before,
                                None => message,
                            };
                            let first_line = first_line.trim();
                            if first_line.is_empty() {
                                "(no message)"
                            } else {
                                first_line
                            }
                        }),
                        style.add_modifier(Modifier::UNDERLINED),
                    ),
                );
                let y = y + 1;

                viewport.draw_blank(Rect {
                    x,
                    y,
                    width: viewport.mask_rect().width,
                    height: 1,
                });
            }
        }
    }
}

#[derive(Clone, Debug)]
struct CommitView<'a> {
    debug_info: Option<&'a AppDebugInfo>,
    commit_message_view: CommitMessageView<'a>,
    file_views: Vec<FileView<'a>>,
}

impl CommitView<'_> {
    const MARGIN: usize = 1;
}

impl Component for CommitView<'_> {
    type Id = ComponentId;

    fn id(&self) -> Self::Id {
        ComponentId::AppFiles
    }

    fn draw(&self, viewport: &mut Viewport<Self::Id>, x: isize, y: isize) {
        let Self {
            debug_info,
            commit_message_view,
            file_views,
        } = self;

        let commit_message_view_rect = viewport.draw_component(x, y, commit_message_view);
        if file_views.is_empty() {
            let message = "There are no changes to view.";
            let message_rect = centered_rect(
                Rect {
                    x,
                    y,
                    width: viewport.mask_rect().width,
                    height: viewport.mask_rect().height,
                },
                RectSize {
                    width: message.len(),
                    height: 1,
                },
                50,
                50,
            );
            viewport.draw_text(message_rect.x, message_rect.y, Span::raw(message));
            return;
        }

        let mut y = y;
        y += commit_message_view_rect.height.unwrap_isize();
        for file_view in file_views {
            let file_view_rect = {
                let file_view_mask = Mask {
                    x,
                    y,
                    width: viewport.mask().width,
                    height: None,
                };
                viewport.with_mask(file_view_mask, |viewport| {
                    viewport.draw_component(x, y, file_view)
                })
            };

            // Render a sticky header if necessary.
            let mask = viewport.mask();
            if file_view_rect.y < mask.y
                && mask.y < file_view_rect.y + file_view_rect.height.unwrap_isize()
            {
                viewport.with_mask(
                    Mask {
                        x,
                        y: mask.y,
                        width: Some(viewport.mask_rect().width),
                        height: Some(1),
                    },
                    |viewport| {
                        viewport.draw_component(
                            x,
                            mask.y,
                            &FileViewHeader {
                                file_key: file_view.file_key,
                                path: file_view.path,
                                old_path: file_view.old_path,
                                is_selected: file_view.is_header_selected,
                                toggle_box: file_view.toggle_box.clone(),
                                expand_box: file_view.expand_box.clone(),
                            },
                        );
                    },
                );
            }

            y += file_view_rect.height.unwrap_isize();

            if debug_info.is_some() {
                viewport.debug(format!(
                    "file {} dims: {file_view_rect:?}",
                    file_view.path.to_string_lossy()
                ));
            }
        }
    }
}

#[derive(Clone, Debug)]
struct MenuItem<'a> {
    label: Cow<'a, str>,
    event: Event,
}

#[derive(Clone, Debug)]
struct Menu<'a> {
    label: Cow<'a, str>,
    items: Vec<MenuItem<'a>>,
}

impl Component for Menu<'_> {
    type Id = ComponentId;

    fn id(&self) -> Self::Id {
        ComponentId::MenuHeader
    }

    fn draw(&self, viewport: &mut Viewport<Self::Id>, x: isize, y: isize) {
        let Self { label: _, items } = self;

        let buttons = items
            .iter()
            .enumerate()
            .map(|(i, item)| Button {
                id: ComponentId::MenuItem(i),
                label: Cow::Borrowed(&item.label),
                style: Style::default(),
                is_focused: false,
            })
            .collect::<Vec<_>>();
        let max_width = buttons
            .iter()
            .map(|button| button.width())
            .max()
            .unwrap_or_default();
        let mut y = y;
        for button in buttons {
            viewport.draw_span(
                x,
                y,
                &Span::styled(
                    " ".repeat(max_width),
                    Style::reset().add_modifier(Modifier::REVERSED),
                ),
            );
            viewport.draw_component(x, y, &button);
            y += 1;
        }
    }
}

#[derive(Clone, Debug)]
struct MenuBar<'a> {
    menus: Vec<Menu<'a>>,
    expanded_menu_idx: Option<usize>,
}

impl Component for MenuBar<'_> {
    type Id = ComponentId;

    fn id(&self) -> Self::Id {
        ComponentId::MenuBar
    }

    fn draw(&self, viewport: &mut Viewport<Self::Id>, x: isize, y: isize) {
        let Self {
            menus,
            expanded_menu_idx,
        } = self;

        viewport.draw_blank(viewport.rect().top_row());
        highlight_rect(viewport, viewport.rect().top_row());
        let mut x = x;
        for (i, menu) in menus.iter().enumerate() {
            let menu_header = Button {
                id: ComponentId::Menu(i),
                label: Cow::Borrowed(&menu.label),
                style: Style::default(),
                is_focused: false,
            };
            let rect = viewport.draw_component(x, y, &menu_header);
            if expanded_menu_idx == &Some(i) {
                viewport.draw_component(x, y + 1, menu);
            }
            x += rect.width.unwrap_isize() + 1;
        }
    }
}

#[derive(Clone, Debug)]
struct FileView<'a> {
    debug: bool,
    file_key: FileKey,
    toggle_box: TristateBox<ComponentId>,
    expand_box: TristateBox<ComponentId>,
    is_header_selected: bool,
    old_path: Option<&'a Path>,
    path: &'a Path,
    section_views: Vec<SectionView<'a>>,
}

impl FileView<'_> {
    fn is_expanded(&self) -> bool {
        match self.expand_box.tristate {
            Tristate::False => false,
            Tristate::Partial | Tristate::True => true,
        }
    }
}

impl Component for FileView<'_> {
    type Id = ComponentId;

    fn id(&self) -> Self::Id {
        ComponentId::SelectableItem(SelectionKey::File(self.file_key))
    }

    fn draw(&self, viewport: &mut Viewport<Self::Id>, x: isize, y: isize) {
        let Self {
            debug,
            file_key,
            toggle_box,
            expand_box,
            old_path,
            path,
            section_views,
            is_header_selected,
        } = self;

        let file_view_header_rect = viewport.draw_component(
            x,
            y,
            &FileViewHeader {
                file_key: *file_key,
                path,
                old_path: *old_path,
                is_selected: *is_header_selected,
                toggle_box: toggle_box.clone(),
                expand_box: expand_box.clone(),
            },
        );
        if self.is_expanded() {
            let x = x + 2;
            let mut section_y = y + file_view_header_rect.height.unwrap_isize();
            let expanded_sections: HashSet<usize> = section_views
                .iter()
                .enumerate()
                .filter_map(|(i, view)| {
                    if view.is_expanded() && view.section.is_editable() {
                        return Some(i);
                    }
                    None
                })
                .collect();
            for (i, section_view) in section_views.iter().enumerate() {
                // Skip this section if it is an un-editable context section and
                // none of the editable sections surrounding it are expanded.
                let context_section = !section_view.section.is_editable();
                let prev_is_collapsed = i == 0 || !expanded_sections.contains(&(i - 1));
                let next_is_collapsed = !expanded_sections.contains(&(i + 1));
                if context_section && prev_is_collapsed && next_is_collapsed {
                    continue;
                }

                let section_rect = viewport.draw_component(x, section_y, section_view);
                section_y += section_rect.height.unwrap_isize();

                if *debug {
                    viewport.debug(format!("section dims: {section_rect:?}",));
                }
            }
        }
    }
}

struct FileViewHeader<'a> {
    file_key: FileKey,
    path: &'a Path,
    old_path: Option<&'a Path>,
    is_selected: bool,
    toggle_box: TristateBox<ComponentId>,
    expand_box: TristateBox<ComponentId>,
}

impl Component for FileViewHeader<'_> {
    type Id = ComponentId;

    fn id(&self) -> Self::Id {
        let Self {
            file_key,
            path: _,
            old_path: _,
            is_selected: _,
            toggle_box: _,
            expand_box: _,
        } = self;
        ComponentId::FileViewHeader(*file_key)
    }

    fn draw(&self, viewport: &mut Viewport<Self::Id>, x: isize, y: isize) {
        let Self {
            file_key: _,
            path,
            old_path,
            is_selected,
            toggle_box,
            expand_box,
        } = self;

        // Draw expand box at end of line.
        let expand_box_width = expand_box.text().width().unwrap_isize();
        let expand_box_rect = viewport.draw_component(
            viewport.mask_rect().end_x() - expand_box_width,
            y,
            expand_box,
        );

        viewport.with_mask(
            Mask {
                x,
                y,
                width: Some((expand_box_rect.x - x).clamp_into_usize()),
                height: Some(1),
            },
            |viewport| {
                viewport.draw_blank(Rect {
                    x,
                    y,
                    width: viewport.mask_rect().width,
                    height: 1,
                });
                let toggle_box_rect = viewport.draw_component(x, y, toggle_box);
                viewport.draw_text(
                    x + toggle_box_rect.width.unwrap_isize() + 1,
                    y,
                    Span::styled(
                        format!(
                            "{}{}",
                            match old_path {
                                Some(old_path) => format!("{} => ", old_path.to_string_lossy()),
                                None => String::new(),
                            },
                            path.to_string_lossy(),
                        ),
                        if *is_selected {
                            Style::default().fg(Color::Blue)
                        } else {
                            Style::default()
                        },
                    ),
                );
            },
        );

        if *is_selected {
            highlight_rect(
                viewport,
                Rect {
                    x: viewport.mask_rect().x,
                    y,
                    width: viewport.mask_rect().width,
                    height: 1,
                },
            );
        }
    }
}

#[derive(Clone, Debug)]
enum SectionSelection {
    SectionHeader,
    ChangedLine(usize),
}

#[derive(Clone, Debug)]
struct SectionView<'a> {
    use_unicode: bool,
    is_read_only: bool,
    section_key: SectionKey,
    toggle_box: TristateBox<ComponentId>,
    expand_box: TristateBox<ComponentId>,
    selection: Option<SectionSelection>,
    total_num_sections: usize,
    editable_section_num: usize,
    total_num_editable_sections: usize,
    section: &'a Section<'a>,
    line_start_num: usize,
}

impl SectionView<'_> {
    fn is_expanded(&self) -> bool {
        match self.expand_box.tristate {
            Tristate::False => false,
            Tristate::Partial => {
                // Shouldn't happen.
                true
            }
            Tristate::True => true,
        }
    }
}

impl Component for SectionView<'_> {
    type Id = ComponentId;

    fn id(&self) -> Self::Id {
        ComponentId::SelectableItem(SelectionKey::Section(self.section_key))
    }

    fn draw(&self, viewport: &mut Viewport<Self::Id>, x: isize, y: isize) {
        let Self {
            use_unicode,
            is_read_only,
            section_key,
            toggle_box,
            expand_box,
            selection,
            total_num_sections,
            editable_section_num,
            total_num_editable_sections,
            section,
            line_start_num,
        } = self;
        viewport.draw_blank(Rect {
            x,
            y,
            width: viewport.mask_rect().width,
            height: 1,
        });

        let SectionKey {
            commit_idx,
            file_idx,
            section_idx,
        } = *section_key;
        match section {
            Section::Unchanged { lines } => {
                if lines.is_empty() {
                    return;
                }

                let lines: Vec<_> = lines.iter().enumerate().collect();
                let is_first_section = section_idx == 0;
                let is_last_section = section_idx + 1 == *total_num_sections;
                let before_ellipsis_lines = &lines[..min(NUM_CONTEXT_LINES, lines.len())];
                let after_ellipsis_lines = &lines[lines.len().saturating_sub(NUM_CONTEXT_LINES)..];

                match (before_ellipsis_lines, after_ellipsis_lines) {
                    ([.., (last_before_idx, _)], [(first_after_idx, _), ..])
                        if *last_before_idx + 1 >= *first_after_idx
                            && !is_first_section
                            && !is_last_section =>
                    {
                        let first_before_idx = before_ellipsis_lines.first().unwrap().0;
                        let last_after_idx = after_ellipsis_lines.last().unwrap().0;
                        let overlapped_lines = &lines[first_before_idx..=last_after_idx];
                        let overlapped_lines = if is_first_section {
                            &overlapped_lines
                                [overlapped_lines.len().saturating_sub(NUM_CONTEXT_LINES)..]
                        } else if is_last_section {
                            &overlapped_lines[..lines.len().min(NUM_CONTEXT_LINES)]
                        } else {
                            overlapped_lines
                        };
                        for (dy, (line_idx, line)) in overlapped_lines.iter().enumerate() {
                            let line_view = SectionLineView {
                                line_key: LineKey {
                                    commit_idx,
                                    file_idx,
                                    section_idx,
                                    line_idx: *line_idx,
                                },
                                inner: SectionLineViewInner::Unchanged {
                                    line: line.as_ref(),
                                    line_num: line_start_num + line_idx,
                                },
                            };
                            viewport.draw_component(x + 2, y + dy.unwrap_isize(), &line_view);
                        }
                        return;
                    }
                    _ => {}
                };

                let mut dy = 0;
                if !is_first_section {
                    for (line_idx, line) in before_ellipsis_lines {
                        let line_view = SectionLineView {
                            line_key: LineKey {
                                commit_idx,
                                file_idx,
                                section_idx,
                                line_idx: *line_idx,
                            },
                            inner: SectionLineViewInner::Unchanged {
                                line: line.as_ref(),
                                line_num: line_start_num + line_idx,
                            },
                        };
                        viewport.draw_component(x + 2, y + dy, &line_view);
                        dy += 1;
                    }
                }

                let should_render_ellipsis = lines.len() > NUM_CONTEXT_LINES;
                if should_render_ellipsis {
                    let ellipsis = if *use_unicode {
                        "\u{22EE}" // Vertical Ellipsis
                    } else {
                        ":"
                    };
                    viewport.draw_span(
                        x + 6, // align with line numbering
                        y + dy,
                        &Span::styled(ellipsis, Style::default().add_modifier(Modifier::DIM)),
                    );
                    dy += 1;
                }

                if !is_last_section {
                    for (line_idx, line) in after_ellipsis_lines {
                        let line_view = SectionLineView {
                            line_key: LineKey {
                                commit_idx,
                                file_idx,
                                section_idx,
                                line_idx: *line_idx,
                            },
                            inner: SectionLineViewInner::Unchanged {
                                line: line.as_ref(),
                                line_num: line_start_num + line_idx,
                            },
                        };
                        viewport.draw_component(x + 2, y + dy, &line_view);
                        dy += 1;
                    }
                }
            }

            Section::Changed { lines } => {
                // Draw expand box at end of line.
                let expand_box_width = expand_box.text().width().unwrap_isize();
                let expand_box_rect = viewport.draw_component(
                    viewport.mask_rect().width.unwrap_isize() - expand_box_width,
                    y,
                    expand_box,
                );

                // Draw section header.
                viewport.with_mask(
                    Mask {
                        x,
                        y,
                        width: Some((expand_box_rect.x - x).clamp_into_usize()),
                        height: Some(1),
                    },
                    |viewport| {
                        let toggle_box_rect = viewport.draw_component(x, y, toggle_box);
                        viewport.draw_text(
                            x + toggle_box_rect.width.unwrap_isize() + 1,
                            y,
                            Span::styled(
                                format!(
                                    "Section {editable_section_num}/{total_num_editable_sections}"
                                ),
                                Style::default(),
                            ),
                        )
                    },
                );

                match selection {
                    Some(SectionSelection::SectionHeader) => {
                        highlight_rect(
                            viewport,
                            Rect {
                                x: viewport.mask_rect().x,
                                y,
                                width: viewport.mask_rect().width,
                                height: 1,
                            },
                        );
                    }
                    Some(SectionSelection::ChangedLine(_)) | None => {}
                }

                if self.is_expanded() {
                    // Draw changed lines.
                    let y = y + 1;
                    for (line_idx, line) in lines.iter().enumerate() {
                        let SectionChangedLine {
                            is_checked,
                            change_type,
                            line,
                        } = line;
                        let is_focused = match selection {
                            Some(SectionSelection::ChangedLine(selected_line_idx)) => {
                                line_idx == *selected_line_idx
                            }
                            Some(SectionSelection::SectionHeader) | None => false,
                        };
                        let line_key = LineKey {
                            commit_idx,
                            file_idx,
                            section_idx,
                            line_idx,
                        };
                        let toggle_box = TristateBox {
                            use_unicode: *use_unicode,
                            id: ComponentId::ToggleBox(SelectionKey::Line(line_key)),
                            icon_style: TristateIconStyle::Check,
                            tristate: Tristate::from(*is_checked),
                            is_focused,
                            is_read_only: *is_read_only,
                        };
                        let line_view = SectionLineView {
                            line_key,
                            inner: SectionLineViewInner::Changed {
                                toggle_box,
                                change_type: *change_type,
                                line: line.as_ref(),
                            },
                        };
                        let y = y + line_idx.unwrap_isize();
                        viewport.draw_component(x + 2, y, &line_view);
                        if is_focused {
                            highlight_rect(
                                viewport,
                                Rect {
                                    x: viewport.mask_rect().x,
                                    y,
                                    width: viewport.mask_rect().width,
                                    height: 1,
                                },
                            );
                        }
                    }
                }
            }

            Section::FileMode { is_checked, mode } => {
                let is_focused = match selection {
                    Some(SectionSelection::SectionHeader) => true,
                    Some(SectionSelection::ChangedLine(_)) | None => false,
                };
                let section_key = SectionKey {
                    commit_idx,
                    file_idx,
                    section_idx,
                };
                let selection_key = SelectionKey::Section(section_key);
                let toggle_box = TristateBox {
                    use_unicode: *use_unicode,
                    id: ComponentId::ToggleBox(selection_key),
                    icon_style: TristateIconStyle::Check,
                    tristate: Tristate::from(*is_checked),
                    is_focused,
                    is_read_only: *is_read_only,
                };
                let toggle_box_rect = viewport.draw_component(x, y, &toggle_box);
                let x = x + toggle_box_rect.width.unwrap_isize() + 1;

                let text = match mode {
                    // TODO: It would be nice to render this as 'file was created with mode x' but we don't have access
                    // to the file's mode to see if it was absent before here.
                    FileMode::Unix(mode) => format!("File mode set to {mode:o}"),
                    FileMode::Absent => "File deleted".to_owned(),
                };

                viewport.draw_text(x, y, Span::styled(text, Style::default().fg(Color::Blue)));
                if is_focused {
                    highlight_rect(
                        viewport,
                        Rect {
                            x: viewport.mask_rect().x,
                            y,
                            width: viewport.mask_rect().width,
                            height: 1,
                        },
                    );
                }
            }

            Section::Binary {
                is_checked,
                old_description,
                new_description,
            } => {
                let is_focused = match selection {
                    Some(SectionSelection::SectionHeader) => true,
                    Some(SectionSelection::ChangedLine(_)) | None => false,
                };
                let section_key = SectionKey {
                    commit_idx,
                    file_idx,
                    section_idx,
                };
                let toggle_box = TristateBox {
                    use_unicode: *use_unicode,
                    id: ComponentId::ToggleBox(SelectionKey::Section(section_key)),
                    icon_style: TristateIconStyle::Check,
                    tristate: Tristate::from(*is_checked),
                    is_focused,
                    is_read_only: *is_read_only,
                };
                let toggle_box_rect = viewport.draw_component(x, y, &toggle_box);
                let x = x + toggle_box_rect.width.unwrap_isize() + 1;

                let text = {
                    let mut result =
                        vec![if old_description.is_some() || new_description.is_some() {
                            "binary contents:"
                        } else {
                            "binary contents"
                        }
                        .to_string()];
                    let description: Vec<_> = [old_description, new_description]
                        .iter()
                        .copied()
                        .flatten()
                        .map(|s| s.as_ref())
                        .collect();
                    result.push(description.join(" -> "));
                    format!("({})", result.join(" "))
                };
                viewport.draw_text(x, y, Span::styled(text, Style::default().fg(Color::Blue)));

                if is_focused {
                    highlight_rect(
                        viewport,
                        Rect {
                            x: viewport.mask_rect().x,
                            y,
                            width: viewport.mask_rect().width,
                            height: 1,
                        },
                    );
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
enum SectionLineViewInner<'a> {
    Unchanged {
        line: &'a str,
        line_num: usize,
    },
    Changed {
        toggle_box: TristateBox<ComponentId>,
        change_type: ChangeType,
        line: &'a str,
    },
}

fn replace_control_character(character: char) -> Option<&'static str> {
    match character {
        // Characters end up writing over each-other and end up
        // displaying incorrectly if ignored. Replacing tabs
        // with a known length string fixes the issue for now.
        '\t' => Some("→   "),
        '\n' => Some("⏎"),
        '\r' => Some("␍"),

        '\x00' => Some("␀"),
        '\x01' => Some("␁"),
        '\x02' => Some("␂"),
        '\x03' => Some("␃"),
        '\x04' => Some("␄"),
        '\x05' => Some("␅"),
        '\x06' => Some("␆"),
        '\x07' => Some("␇"),
        '\x08' => Some("␈"),
        // '\x09' ('\t') handled above
        // '\x0A' ('\n') handled above
        '\x0B' => Some("␋"),
        '\x0C' => Some("␌"),
        // '\x0D' ('\r') handled above
        '\x0E' => Some("␎"),
        '\x0F' => Some("␏"),
        '\x10' => Some("␐"),
        '\x11' => Some("␑"),
        '\x12' => Some("␒"),
        '\x13' => Some("␓"),
        '\x14' => Some("␔"),
        '\x15' => Some("␕"),
        '\x16' => Some("␖"),
        '\x17' => Some("␗"),
        '\x18' => Some("␘"),
        '\x19' => Some("␙"),
        '\x1A' => Some("␚"),
        '\x1B' => Some("␛"),
        '\x1C' => Some("␜"),
        '\x1D' => Some("␝"),
        '\x1E' => Some("␞"),
        '\x1F' => Some("␟"),

        '\x7F' => Some("␡"),

        c if c.width().unwrap_or_default() == 0 => Some("�"),

        _ => None,
    }
}

/// Split the line into a sequence of [`Span`]s where control characters are
/// replaced with styled [`Span`]'s and push them to the [`spans`] argument.
fn push_spans_from_line<'line>(line: &'line str, spans: &mut Vec<Span<'line>>) {
    const CONTROL_CHARACTER_STYLE: Style = Style::new().fg(Color::DarkGray);

    let mut last_index = 0;
    // Find index of the start of each character to replace
    for (idx, char) in line.match_indices(|char| replace_control_character(char).is_some()) {
        // Push the string leading up to the character and the styled replacement string
        if let Some(replacement_string) = char.chars().next().and_then(replace_control_character) {
            spans.push(Span::raw(&line[last_index..idx]));
            spans.push(Span::styled(replacement_string, CONTROL_CHARACTER_STYLE));
            // Move the "cursor" to just after the character we're replacing
            last_index = idx + char.len();
        }
    }
    // Append anything remaining after the last replacement
    let remaining_line = &line[last_index..];
    if !remaining_line.is_empty() {
        spans.push(Span::raw(remaining_line));
    }
}

#[derive(Clone, Debug)]
struct SectionLineView<'a> {
    line_key: LineKey,
    inner: SectionLineViewInner<'a>,
}

impl Component for SectionLineView<'_> {
    type Id = ComponentId;

    fn id(&self) -> Self::Id {
        ComponentId::SelectableItem(SelectionKey::Line(self.line_key))
    }

    fn draw(&self, viewport: &mut Viewport<Self::Id>, x: isize, y: isize) {
        viewport.draw_blank(Rect {
            x: viewport.mask_rect().x,
            y,
            width: viewport.mask_rect().width,
            height: 1,
        });

        match &self.inner {
            SectionLineViewInner::Unchanged { line, line_num } => {
                // Pad the number in 5 columns because that will align the
                // beginning of the actual text with the `+`/`-` of the changed
                // lines.
                let line_number = Span::raw(format!("{line_num:5} "));
                let mut spans = vec![line_number];
                push_spans_from_line(line, &mut spans);

                const UI_UNCHANGED_STYLE: Style = Style::new().add_modifier(Modifier::DIM);
                viewport.draw_text(x, y, Line::from(spans).style(UI_UNCHANGED_STYLE));
            }

            SectionLineViewInner::Changed {
                toggle_box,
                change_type,
                line,
            } => {
                let toggle_box_rect = viewport.draw_component(x, y, toggle_box);
                let x = toggle_box_rect.end_x() + 1;

                let (change_type_text, changed_line_style) = match change_type {
                    ChangeType::Added => ("+ ", Style::default().fg(Color::Green)),
                    ChangeType::Removed => ("- ", Style::default().fg(Color::Red)),
                };

                let mut spans = vec![Span::raw(change_type_text)];
                push_spans_from_line(line, &mut spans);

                viewport.draw_text(x, y, Line::from(spans).style(changed_line_style));
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct QuitDialog {
    num_commit_messages: usize,
    num_changed_files: usize,
    focused_button: QuitDialogButtonId,
}

impl Component for QuitDialog {
    type Id = ComponentId;

    fn id(&self) -> Self::Id {
        ComponentId::QuitDialog
    }

    fn draw(&self, viewport: &mut Viewport<Self::Id>, _x: isize, _y: isize) {
        let Self {
            num_commit_messages,
            num_changed_files,
            focused_button,
        } = self;
        let title = "Quit";
        let alert_items = {
            let mut result = Vec::new();
            if *num_commit_messages > 0 {
                result.push(format!(
                    "{num_commit_messages} {}",
                    if *num_commit_messages == 1 {
                        "message"
                    } else {
                        "messages"
                    }
                ));
            }
            if *num_changed_files > 0 {
                result.push(format!(
                    "{num_changed_files} {}",
                    if *num_changed_files == 1 {
                        "file"
                    } else {
                        "files"
                    }
                ));
            }
            result
        };
        let alert = if alert_items.is_empty() {
            // Shouldn't happen.
            "".to_string()
        } else {
            format!("You have changes to {}. ", alert_items.join(" and "))
        };
        let body = format!("{alert}Are you sure you want to quit?",);

        let quit_button = Button {
            id: ComponentId::QuitDialogButton(QuitDialogButtonId::Quit),
            label: Cow::Borrowed("Quit"),
            style: Style::default(),
            is_focused: match focused_button {
                QuitDialogButtonId::Quit => true,
                QuitDialogButtonId::GoBack => false,
            },
        };
        let go_back_button = Button {
            id: ComponentId::QuitDialogButton(QuitDialogButtonId::GoBack),
            label: Cow::Borrowed("Go Back"),
            style: Style::default(),
            is_focused: match focused_button {
                QuitDialogButtonId::GoBack => true,
                QuitDialogButtonId::Quit => false,
            },
        };
        let buttons = [quit_button, go_back_button];

        let dialog = Dialog {
            id: ComponentId::QuitDialog,
            title: Cow::Borrowed(title),
            body: Cow::Owned(body),
            buttons: &buttons,
        };
        viewport.draw_component(0, 0, &dialog);
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct HelpDialog();

impl Component for HelpDialog {
    type Id = ComponentId;

    fn id(&self) -> Self::Id {
        ComponentId::HelpDialog
    }

    fn draw(&self, viewport: &mut Viewport<Self::Id>, _: isize, _: isize) {
        let title = "Help";
        let body = "You can click the menus with the mouse to view keyboard shortcuts.";

        let quit_button = Button {
            id: ComponentId::HelpDialogQuitButton,
            label: Cow::Borrowed("Close"),
            style: Style::default(),
            is_focused: true,
        };

        let buttons = [quit_button];
        let dialog = Dialog {
            id: self.id(),
            title: Cow::Borrowed(title),
            body: Cow::Borrowed(body),
            buttons: &buttons,
        };
        viewport.draw_component(0, 0, &dialog);
    }
}

struct Button<'a, Id> {
    id: Id,
    label: Cow<'a, str>,
    style: Style,
    is_focused: bool,
}

impl<Id> Button<'_, Id> {
    fn span(&self) -> Span {
        let Self {
            id: _,
            label,
            style,
            is_focused,
        } = self;
        if *is_focused {
            Span::styled(format!("({label})"), style.add_modifier(Modifier::REVERSED))
        } else {
            Span::styled(format!("[{label}]"), *style)
        }
    }

    fn width(&self) -> usize {
        self.span().width()
    }
}

impl<Id: Clone + Debug + Eq + Hash> Component for Button<'_, Id> {
    type Id = Id;

    fn id(&self) -> Self::Id {
        self.id.clone()
    }

    fn draw(&self, viewport: &mut Viewport<Self::Id>, x: isize, y: isize) {
        let span = self.span();
        viewport.draw_span(x, y, &span);
    }
}

struct Dialog<'a, Id> {
    id: Id,
    title: Cow<'a, str>,
    body: Cow<'a, str>,
    buttons: &'a [Button<'a, Id>],
}

impl<Id: Clone + Debug + Eq + Hash> Component for Dialog<'_, Id> {
    type Id = Id;

    fn id(&self) -> Self::Id {
        self.id.clone()
    }

    fn draw(&self, viewport: &mut Viewport<Self::Id>, _x: isize, _y: isize) {
        let Self {
            id: _,
            title,
            body,
            buttons,
        } = self;
        let rect = {
            let border_size = 2;
            let body_lines = body.lines().count();
            let rect = centered_rect(
                viewport.rect(),
                RectSize {
                    // FIXME: we might want to limit the width of the text and
                    // let `Paragraph` wrap it.
                    width: body.width() + border_size,
                    height: body_lines + border_size,
                },
                60,
                20,
            );

            let paragraph = Paragraph::new(body.as_ref()).block(
                Block::default()
                    .title(title.as_ref())
                    .borders(Borders::all()),
            );
            let tui_rect = viewport.translate_rect(rect);
            viewport.draw_widget(tui_rect, Clear);
            viewport.draw_widget(tui_rect, paragraph);

            rect
        };

        let mut bottom_x = rect.x + rect.width.unwrap_isize() - 1;
        let bottom_y = rect.y + rect.height.unwrap_isize() - 1;
        for button in buttons.iter() {
            bottom_x -= button.width().unwrap_isize();
            let button_rect = viewport.draw_component(bottom_x, bottom_y, button);
            bottom_x = button_rect.x - 1;
        }
    }
}

fn highlight_rect<Id: Clone + Debug + Eq + Hash>(viewport: &mut Viewport<Id>, rect: Rect) {
    viewport.set_style(rect, Style::default().add_modifier(Modifier::REVERSED));
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use crate::helpers::TestingInput;

    use super::*;

    use assert_matches::assert_matches;

    #[test]
    fn test_event_source_testing() {
        let mut event_source = TestingInput::new(80, 24, [Event::QuitCancel]);
        assert_matches!(
            event_source.next_events().unwrap().as_slice(),
            &[Event::QuitCancel]
        );
        assert_matches!(
            event_source.next_events().unwrap().as_slice(),
            &[Event::None]
        );
    }

    #[test]
    fn test_quit_returns_error() {
        let state = RecordState::default();
        let mut input = TestingInput::new(80, 24, [Event::QuitCancel]);
        let recorder = Recorder::new(state, &mut input);
        assert_matches!(recorder.run(), Err(RecordError::Cancelled));

        let state = RecordState {
            is_read_only: false,
            commits: vec![Commit::default(), Commit::default()],
            files: vec![File {
                old_path: None,
                path: Cow::Borrowed(Path::new("foo/bar")),
                file_mode: FileMode::FILE_DEFAULT,
                sections: Default::default(),
            }],
        };
        let mut input = TestingInput::new(80, 24, [Event::QuitAccept]);
        let recorder = Recorder::new(state.clone(), &mut input);
        assert_eq!(recorder.run().unwrap(), state);
    }

    fn test_push_lines_from_span_impl(line: &str) {
        let mut spans = Vec::new();
        push_spans_from_line(line, &mut spans); // assert no panic
    }

    proptest::proptest! {
        #[test]
        fn test_push_lines_from_span(line in ".*") {
            test_push_lines_from_span_impl(line.as_str());
        }
    }
}
