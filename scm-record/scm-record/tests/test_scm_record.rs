use std::path::Path;
use std::{borrow::Cow, iter};

use assert_matches::assert_matches;
use insta::{assert_debug_snapshot, assert_snapshot};
use scm_record::helpers::{make_binary_description, TestingInput};
use scm_record::{
    ChangeType, Commit, Event, File, FileMode, RecordError, RecordState, Recorder, Section,
    SectionChangedLine, TestingScreenshot,
};

type TestResult = Result<(), scm_record::RecordError>;

fn example_contents() -> RecordState<'static> {
    RecordState {
        is_read_only: false,
        commits: Default::default(),
        files: vec![
            File {
                old_path: None,
                path: Cow::Borrowed(Path::new("foo/bar")),
                file_mode: FileMode::FILE_DEFAULT,
                sections: vec![
                    Section::Unchanged {
                        lines: iter::repeat(Cow::Borrowed("this is some text\n"))
                            .take(20)
                            .collect(),
                    },
                    Section::Changed {
                        lines: vec![
                            SectionChangedLine {
                                is_checked: true,
                                change_type: ChangeType::Removed,
                                line: Cow::Borrowed("before text 1\n"),
                            },
                            SectionChangedLine {
                                is_checked: true,
                                change_type: ChangeType::Removed,
                                line: Cow::Borrowed("before text 2\n"),
                            },
                            SectionChangedLine {
                                is_checked: true,
                                change_type: ChangeType::Added,
                                line: Cow::Borrowed("after text 1\n"),
                            },
                            SectionChangedLine {
                                is_checked: false,
                                change_type: ChangeType::Added,
                                line: Cow::Borrowed("after text 2\n"),
                            },
                        ],
                    },
                    Section::Unchanged {
                        lines: vec![Cow::Borrowed("this is some trailing text\n")],
                    },
                ],
            },
            File {
                old_path: None,
                path: Cow::Borrowed(Path::new("baz")),
                file_mode: FileMode::FILE_DEFAULT,
                sections: vec![
                    Section::Unchanged {
                        lines: vec![
                            Cow::Borrowed("Some leading text 1\n"),
                            Cow::Borrowed("Some leading text 2\n"),
                        ],
                    },
                    Section::Changed {
                        lines: vec![
                            SectionChangedLine {
                                is_checked: true,
                                change_type: ChangeType::Removed,
                                line: Cow::Borrowed("before text 1\n"),
                            },
                            SectionChangedLine {
                                is_checked: true,
                                change_type: ChangeType::Removed,
                                line: Cow::Borrowed("before text 2\n"),
                            },
                            SectionChangedLine {
                                is_checked: true,
                                change_type: ChangeType::Added,
                                line: Cow::Borrowed("after text 1\n"),
                            },
                            SectionChangedLine {
                                is_checked: true,
                                change_type: ChangeType::Added,
                                line: Cow::Borrowed("after text 2\n"),
                            },
                        ],
                    },
                    Section::Unchanged {
                        lines: vec![Cow::Borrowed("this is some trailing text\n")],
                    },
                ],
            },
        ],
    }
}

#[test]
fn test_select_scroll_into_view() -> TestResult {
    let initial = TestingScreenshot::default();
    let scroll_to_first_section = TestingScreenshot::default();
    let scroll_to_second_file = TestingScreenshot::default();
    let mut input = TestingInput::new(
        80,
        6,
        [
            Event::ExpandAll,
            initial.event(),
            // Scroll to first section (off-screen).
            Event::FocusNext,
            scroll_to_first_section.event(),
            // Scroll to second file (off-screen). It should display the entire
            // file contents, since they all fit in the viewport.
            Event::FocusNext,
            Event::FocusNext,
            Event::FocusNext,
            Event::FocusNext,
            Event::FocusNext,
            scroll_to_second_file.event(),
            Event::QuitAccept,
        ],
    );
    let state = example_contents();
    let recorder = Recorder::new(state, &mut input);
    recorder.run()?;

    insta::assert_snapshot!(initial, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "(◐) foo/bar                                                                  (-)"
    "        ⋮                                                                       "
    "       18 this is some text⏎                                                    "
    "       19 this is some text⏎                                                    "
    "       20 this is some text⏎                                                    "
    "###);
    insta::assert_snapshot!(scroll_to_first_section, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "[◐] foo/bar                                                                  [-]"
    "  (◐) Section 1/1                                                            (-)"
    "    [●] - before text 1⏎                                                        "
    "    [●] - before text 2⏎                                                        "
    "    [●] + after text 1⏎                                                         "
    "###);
    insta::assert_snapshot!(scroll_to_second_file, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "(●) baz                                                                      (-)"
    "        1 Some leading text 1⏎                                                  "
    "        2 Some leading text 2⏎                                                  "
    "  [●] Section 1/1                                                            [-]"
    "    [●] - before text 1⏎                                                        "
    "###);
    Ok(())
}

#[test]
fn test_toggle_all() -> TestResult {
    let before = TestingScreenshot::default();
    let after = TestingScreenshot::default();
    let mut input = TestingInput::new(
        80,
        20,
        [
            Event::ExpandAll,
            before.event(),
            Event::ToggleAll,
            after.event(),
            Event::QuitAccept,
        ],
    );
    let state = example_contents();
    let recorder = Recorder::new(state, &mut input);
    recorder.run()?;

    insta::assert_snapshot!(before, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "(◐) foo/bar                                                                  (-)"
    "        ⋮                                                                       "
    "       18 this is some text⏎                                                    "
    "       19 this is some text⏎                                                    "
    "       20 this is some text⏎                                                    "
    "  [◐] Section 1/1                                                            [-]"
    "    [●] - before text 1⏎                                                        "
    "    [●] - before text 2⏎                                                        "
    "    [●] + after text 1⏎                                                         "
    "    [ ] + after text 2⏎                                                         "
    "       23 this is some trailing text⏎                                           "
    "[●] baz                                                                      [-]"
    "        1 Some leading text 1⏎                                                  "
    "        2 Some leading text 2⏎                                                  "
    "  [●] Section 1/1                                                            [-]"
    "    [●] - before text 1⏎                                                        "
    "    [●] - before text 2⏎                                                        "
    "    [●] + after text 1⏎                                                         "
    "    [●] + after text 2⏎                                                         "
    "###);
    insta::assert_snapshot!(after, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "(◐) foo/bar                                                                  (-)"
    "        ⋮                                                                       "
    "       18 this is some text⏎                                                    "
    "       19 this is some text⏎                                                    "
    "       20 this is some text⏎                                                    "
    "  [◐] Section 1/1                                                            [-]"
    "    [ ] - before text 1⏎                                                        "
    "    [ ] - before text 2⏎                                                        "
    "    [ ] + after text 1⏎                                                         "
    "    [●] + after text 2⏎                                                         "
    "       23 this is some trailing text⏎                                           "
    "[ ] baz                                                                      [-]"
    "        1 Some leading text 1⏎                                                  "
    "        2 Some leading text 2⏎                                                  "
    "  [ ] Section 1/1                                                            [-]"
    "    [ ] - before text 1⏎                                                        "
    "    [ ] - before text 2⏎                                                        "
    "    [ ] + after text 1⏎                                                         "
    "    [ ] + after text 2⏎                                                         "
    "###);
    Ok(())
}

#[test]
fn test_toggle_all_uniform() -> TestResult {
    let initial = TestingScreenshot::default();
    let first_toggle = TestingScreenshot::default();
    let second_toggle = TestingScreenshot::default();
    let third_toggle = TestingScreenshot::default();
    let mut input = TestingInput::new(
        80,
        10,
        [
            Event::ExpandAll,
            initial.event(),
            Event::ToggleAllUniform,
            first_toggle.event(),
            Event::ToggleAllUniform,
            second_toggle.event(),
            Event::ToggleAllUniform,
            third_toggle.event(),
            Event::QuitAccept,
        ],
    );
    let state = example_contents();
    let recorder = Recorder::new(state, &mut input);
    recorder.run()?;

    insta::assert_snapshot!(initial, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "(◐) foo/bar                                                                  (-)"
    "        ⋮                                                                       "
    "       18 this is some text⏎                                                    "
    "       19 this is some text⏎                                                    "
    "       20 this is some text⏎                                                    "
    "  [◐] Section 1/1                                                            [-]"
    "    [●] - before text 1⏎                                                        "
    "    [●] - before text 2⏎                                                        "
    "    [●] + after text 1⏎                                                         "
    "###);
    insta::assert_snapshot!(first_toggle, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "(●) foo/bar                                                                  (-)"
    "        ⋮                                                                       "
    "       18 this is some text⏎                                                    "
    "       19 this is some text⏎                                                    "
    "       20 this is some text⏎                                                    "
    "  [●] Section 1/1                                                            [-]"
    "    [●] - before text 1⏎                                                        "
    "    [●] - before text 2⏎                                                        "
    "    [●] + after text 1⏎                                                         "
    "###);
    insta::assert_snapshot!(second_toggle, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "( ) foo/bar                                                                  (-)"
    "        ⋮                                                                       "
    "       18 this is some text⏎                                                    "
    "       19 this is some text⏎                                                    "
    "       20 this is some text⏎                                                    "
    "  [ ] Section 1/1                                                            [-]"
    "    [ ] - before text 1⏎                                                        "
    "    [ ] - before text 2⏎                                                        "
    "    [ ] + after text 1⏎                                                         "
    "###);
    insta::assert_snapshot!(third_toggle, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "(●) foo/bar                                                                  (-)"
    "        ⋮                                                                       "
    "       18 this is some text⏎                                                    "
    "       19 this is some text⏎                                                    "
    "       20 this is some text⏎                                                    "
    "  [●] Section 1/1                                                            [-]"
    "    [●] - before text 1⏎                                                        "
    "    [●] - before text 2⏎                                                        "
    "    [●] + after text 1⏎                                                         "
    "###);

    Ok(())
}

#[test]
fn test_quit_dialog_size() -> TestResult {
    let expect_quit_dialog_to_be_centered = TestingScreenshot::default();
    let mut input = TestingInput::new(
        100,
        40,
        [
            Event::ExpandAll,
            Event::QuitInterrupt,
            expect_quit_dialog_to_be_centered.event(),
            Event::QuitInterrupt,
        ],
    );
    let state = example_contents();
    let recorder = Recorder::new(state, &mut input);
    let result = recorder.run();
    assert_matches!(result, Err(RecordError::Cancelled));
    insta::assert_snapshot!(expect_quit_dialog_to_be_centered, @r###"
    "[File] [Edit] [Select] [View]                                                                       "
    "(◐) foo/bar                                                                                      (-)"
    "        ⋮                                                                                           "
    "       18 this is some text⏎                                                                        "
    "       19 this is some text⏎                                                                        "
    "       20 this is some text⏎                                                                        "
    "  [◐] Section 1/1                                                                                [-]"
    "    [●] - before text 1⏎                                                                            "
    "    [●] - before text 2⏎                                                                            "
    "    [●] + after text 1⏎                                                                             "
    "    [ ] + after text 2⏎                                                                             "
    "       23 this is some trailing text⏎                                                               "
    "[●] baz                                                                                          [-]"
    "        1 Some leading text 1⏎                                                                      "
    "        2 Some leading text 2⏎                                                                      "
    "  [●] Section 1/1                                                                                [-]"
    "    [●] - before te┌Quit───────────────────────────────────────────────────────┐                    "
    "    [●] - before te│You have changes to 2 files. Are you sure you want to quit?│                    "
    "    [●] + after tex│                                                           │                    "
    "    [●] + after tex│                                                           │                    "
    "        5 this is s│                                                           │                    "
    "                   │                                                           │                    "
    "                   │                                                           │                    "
    "                   └───────────────────────────────────────────[Go Back]─(Quit)┘                    "
    "                                                                                                    "
    "                                                                                                    "
    "                                                                                                    "
    "                                                                                                    "
    "                                                                                                    "
    "                                                                                                    "
    "                                                                                                    "
    "                                                                                                    "
    "                                                                                                    "
    "                                                                                                    "
    "                                                                                                    "
    "                                                                                                    "
    "                                                                                                    "
    "                                                                                                    "
    "                                                                                                    "
    "                                                                                                    "
    "###);
    Ok(())
}

#[test]
fn test_quit_dialog_keyboard_navigation() -> TestResult {
    let expect_q_opens_quit_dialog = TestingScreenshot::default();
    let expect_c_does_nothing = TestingScreenshot::default();
    let expect_q_closes_quit_dialog = TestingScreenshot::default();
    let expect_ctrl_c_opens_quit_dialog = TestingScreenshot::default();
    let expect_exited = TestingScreenshot::default();
    let mut input = TestingInput::new(
        80,
        6,
        [
            Event::ExpandAll,
            // Pressing 'q' should display the quit dialog.
            Event::QuitCancel,
            expect_q_opens_quit_dialog.event(),
            // Pressing 'c' now should do nothing.
            Event::QuitAccept,
            expect_c_does_nothing.event(),
            // Pressing 'q' now should close the quit dialog.
            Event::QuitCancel,
            expect_q_closes_quit_dialog.event(),
            // Pressing ctrl-c should display the quit dialog.
            Event::QuitInterrupt,
            expect_ctrl_c_opens_quit_dialog.event(),
            // Pressing ctrl-c again should exit.
            Event::QuitInterrupt,
            expect_exited.event(),
        ],
    );
    let state = example_contents();
    let recorder = Recorder::new(state, &mut input);
    assert_matches!(recorder.run(), Err(RecordError::Cancelled));
    insta::assert_snapshot!(expect_q_opens_quit_dialog, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "(◐) foo/b┌Quit───────────────────────────────────────────────────────┐       (-)"
    "        ⋮│You have changes to 2 files. Are you sure you want to quit?│          "
    "       18└───────────────────────────────────────────[Go Back]─(Quit)┘          "
    "       19 this is some text⏎                                                    "
    "       20 this is some text⏎                                                    "
    "###);
    insta::assert_snapshot!(expect_c_does_nothing, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "(◐) foo/b┌Quit───────────────────────────────────────────────────────┐       (-)"
    "        ⋮│You have changes to 2 files. Are you sure you want to quit?│          "
    "       18└───────────────────────────────────────────[Go Back]─(Quit)┘          "
    "       19 this is some text⏎                                                    "
    "       20 this is some text⏎                                                    "
    "###);
    insta::assert_snapshot!(expect_q_closes_quit_dialog, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "(◐) foo/bar                                                                  (-)"
    "        ⋮                                                                       "
    "       18 this is some text⏎                                                    "
    "       19 this is some text⏎                                                    "
    "       20 this is some text⏎                                                    "
    "###);
    insta::assert_snapshot!(expect_ctrl_c_opens_quit_dialog, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "(◐) foo/b┌Quit───────────────────────────────────────────────────────┐       (-)"
    "        ⋮│You have changes to 2 files. Are you sure you want to quit?│          "
    "       18└───────────────────────────────────────────[Go Back]─(Quit)┘          "
    "       19 this is some text⏎                                                    "
    "       20 this is some text⏎                                                    "
    "###);
    insta::assert_snapshot!(expect_exited, @"<this screenshot was never assigned>");
    Ok(())
}

#[test]
fn test_quit_dialog_buttons() -> TestResult {
    let expect_quit_button_focused_initially = TestingScreenshot::default();
    let expect_left_focuses_go_back_button = TestingScreenshot::default();
    let expect_left_again_does_not_wrap = TestingScreenshot::default();
    let expect_back_button_closes_quit_dialog = TestingScreenshot::default();
    let expect_right_focuses_quit_button = TestingScreenshot::default();
    let expect_right_again_does_not_wrap = TestingScreenshot::default();
    let expect_ctrl_left_focuses_go_back_button = TestingScreenshot::default();
    let expect_exited = TestingScreenshot::default();
    let mut input = TestingInput::new(
        80,
        6,
        [
            Event::ExpandAll,
            Event::QuitCancel,
            expect_quit_button_focused_initially.event(),
            // Pressing left should select the back button.
            Event::FocusOuter { fold_section: true },
            expect_left_focuses_go_back_button.event(),
            // Pressing left again should do nothing.
            Event::FocusOuter { fold_section: true },
            expect_left_again_does_not_wrap.event(),
            // Selecting the back button should close the dialog.
            Event::ToggleItem,
            expect_back_button_closes_quit_dialog.event(),
            Event::QuitCancel,
            // Pressing right should select the quit button.
            Event::FocusOuter { fold_section: true },
            Event::FocusInner,
            expect_right_focuses_quit_button.event(),
            // Pressing right again should do nothing.
            Event::FocusInner,
            expect_right_again_does_not_wrap.event(),
            // We have two ways to focus outer, with and without folding.
            // Both should navigate properly in this menu.
            Event::FocusOuter {
                fold_section: false,
            },
            expect_ctrl_left_focuses_go_back_button.event(),
            // Selecting the quit button should quit.
            Event::FocusInner,
            Event::ToggleItem,
            expect_exited.event(),
        ],
    );
    let state = example_contents();
    let recorder = Recorder::new(state, &mut input);
    assert_matches!(recorder.run(), Err(RecordError::Cancelled));
    insta::assert_snapshot!(expect_quit_button_focused_initially, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "(◐) foo/b┌Quit───────────────────────────────────────────────────────┐       (-)"
    "        ⋮│You have changes to 2 files. Are you sure you want to quit?│          "
    "       18└───────────────────────────────────────────[Go Back]─(Quit)┘          "
    "       19 this is some text⏎                                                    "
    "       20 this is some text⏎                                                    "
    "###);
    insta::assert_snapshot!(expect_left_focuses_go_back_button, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "(◐) foo/b┌Quit───────────────────────────────────────────────────────┐       (-)"
    "        ⋮│You have changes to 2 files. Are you sure you want to quit?│          "
    "       18└───────────────────────────────────────────(Go Back)─[Quit]┘          "
    "       19 this is some text⏎                                                    "
    "       20 this is some text⏎                                                    "
    "###);
    insta::assert_snapshot!(expect_left_again_does_not_wrap, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "(◐) foo/b┌Quit───────────────────────────────────────────────────────┐       (-)"
    "        ⋮│You have changes to 2 files. Are you sure you want to quit?│          "
    "       18└───────────────────────────────────────────(Go Back)─[Quit]┘          "
    "       19 this is some text⏎                                                    "
    "       20 this is some text⏎                                                    "
    "###);
    insta::assert_snapshot!(expect_back_button_closes_quit_dialog, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "(◐) foo/bar                                                                  (-)"
    "        ⋮                                                                       "
    "       18 this is some text⏎                                                    "
    "       19 this is some text⏎                                                    "
    "       20 this is some text⏎                                                    "
    "###);
    insta::assert_snapshot!(expect_right_focuses_quit_button, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "(◐) foo/b┌Quit───────────────────────────────────────────────────────┐       (-)"
    "        ⋮│You have changes to 2 files. Are you sure you want to quit?│          "
    "       18└───────────────────────────────────────────[Go Back]─(Quit)┘          "
    "       19 this is some text⏎                                                    "
    "       20 this is some text⏎                                                    "
    "###);
    insta::assert_snapshot!(expect_right_again_does_not_wrap, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "(◐) foo/b┌Quit───────────────────────────────────────────────────────┐       (-)"
    "        ⋮│You have changes to 2 files. Are you sure you want to quit?│          "
    "       18└───────────────────────────────────────────[Go Back]─(Quit)┘          "
    "       19 this is some text⏎                                                    "
    "       20 this is some text⏎                                                    "
    "###);
    insta::assert_snapshot!(expect_ctrl_left_focuses_go_back_button, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "(◐) foo/b┌Quit───────────────────────────────────────────────────────┐       (-)"
    "        ⋮│You have changes to 2 files. Are you sure you want to quit?│          "
    "       18└───────────────────────────────────────────(Go Back)─[Quit]┘          "
    "       19 this is some text⏎                                                    "
    "       20 this is some text⏎                                                    "
    "###);
    insta::assert_snapshot!(expect_exited, @"<this screenshot was never assigned>");
    Ok(())
}

#[test]
fn test_enter_next() -> TestResult {
    let state = RecordState {
        is_read_only: false,
        commits: Default::default(),
        files: vec![
            File {
                old_path: None,
                path: Cow::Borrowed(Path::new("foo")),
                file_mode: FileMode::FILE_DEFAULT,
                sections: vec![Section::Changed {
                    lines: vec![
                        SectionChangedLine {
                            is_checked: false,
                            change_type: ChangeType::Added,
                            line: Cow::Borrowed("world\n"),
                        },
                        SectionChangedLine {
                            is_checked: false,
                            change_type: ChangeType::Removed,
                            line: Cow::Borrowed("hello\n"),
                        },
                    ],
                }],
            },
            File {
                old_path: None,
                path: Cow::Borrowed(Path::new("bar")),
                file_mode: FileMode::FILE_DEFAULT,
                sections: vec![Section::Changed {
                    lines: vec![
                        SectionChangedLine {
                            is_checked: false,
                            change_type: ChangeType::Added,
                            line: Cow::Borrowed("world\n"),
                        },
                        SectionChangedLine {
                            is_checked: false,
                            change_type: ChangeType::Removed,
                            line: Cow::Borrowed("hello\n"),
                        },
                    ],
                }],
            },
        ],
    };

    let first_file_selected = TestingScreenshot::default();
    let second_file_selected = TestingScreenshot::default();
    let mut input = TestingInput::new(
        80,
        7,
        [
            Event::ExpandAll,
            Event::ToggleItemAndAdvance,
            first_file_selected.event(),
            Event::ToggleItemAndAdvance,
            second_file_selected.event(),
            Event::QuitCancel,
            Event::ToggleItemAndAdvance,
        ],
    );
    let recorder = Recorder::new(state, &mut input);
    assert_matches!(recorder.run(), Err(RecordError::Cancelled));
    insta::assert_snapshot!(first_file_selected, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "[●] foo                                                                      [-]"
    "    [●] - hello⏎                                                                "
    "( ) bar                                                                      (-)"
    "  [ ] Section 1/1                                                            [-]"
    "    [ ] + world⏎                                                                "
    "    [ ] - hello⏎                                                                "
    "###);
    insta::assert_snapshot!(second_file_selected, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "[●] foo                                                                      [-]"
    "    [●] - hello⏎                                                                "
    "(●) bar                                                                      (-)"
    "  [●] Section 1/1                                                            [-]"
    "    [●] + world⏎                                                                "
    "    [●] - hello⏎                                                                "
    "###);
    Ok(())
}

#[test]
fn test_file_mode_change() -> TestResult {
    let state = RecordState {
        is_read_only: false,
        commits: Default::default(),
        files: vec![
            File {
                old_path: None,
                path: Cow::Borrowed(Path::new("foo")),
                file_mode: FileMode::FILE_DEFAULT,
                sections: vec![],
            },
            File {
                old_path: None,
                path: Cow::Borrowed(Path::new("bar")),
                file_mode: FileMode::FILE_DEFAULT,
                sections: vec![Section::FileMode {
                    is_checked: false,
                    mode: FileMode::Unix(0o100755),
                }],
            },
            File {
                old_path: None,
                path: Cow::Borrowed(Path::new("qux")),
                file_mode: FileMode::FILE_DEFAULT,
                sections: vec![],
            },
        ],
    };

    let before_toggle = TestingScreenshot::default();
    let after_toggle = TestingScreenshot::default();
    let expect_no_crash = TestingScreenshot::default();
    let mut input = TestingInput::new(
        80,
        6,
        [
            Event::ExpandAll,
            before_toggle.event(),
            Event::FocusNext,
            Event::FocusNext,
            Event::ToggleItem,
            after_toggle.event(),
            Event::FocusNext,
            expect_no_crash.event(),
            Event::QuitAccept,
        ],
    );
    let recorder = Recorder::new(state, &mut input);
    insta::assert_debug_snapshot!(recorder.run()?, @r###"
    RecordState {
        is_read_only: false,
        commits: [
            Commit {
                message: None,
            },
            Commit {
                message: None,
            },
        ],
        files: [
            File {
                old_path: None,
                path: "foo",
                file_mode: Unix(
                    33188,
                ),
                sections: [],
            },
            File {
                old_path: None,
                path: "bar",
                file_mode: Unix(
                    33188,
                ),
                sections: [
                    FileMode {
                        is_checked: true,
                        mode: Unix(
                            33261,
                        ),
                    },
                ],
            },
            File {
                old_path: None,
                path: "qux",
                file_mode: Unix(
                    33188,
                ),
                sections: [],
            },
        ],
    }
    "###);
    insta::assert_snapshot!(before_toggle, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "( ) foo                                                                      (-)"
    "[ ] bar                                                                      [-]"
    "  [ ] File mode set to 100755                                                   "
    "[ ] qux                                                                      [-]"
    "                                                                                "
    "###);
    insta::assert_snapshot!(after_toggle, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "[ ] foo                                                                      [-]"
    "[●] bar                                                                      [-]"
    "  (●) File mode set to 100755                                                   "
    "[ ] qux                                                                      [-]"
    "                                                                                "
    "###);
    insta::assert_snapshot!(expect_no_crash, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "[ ] foo                                                                      [-]"
    "[●] bar                                                                      [-]"
    "  [●] File mode set to 100755                                                   "
    "( ) qux                                                                      (-)"
    "                                                                                "
    "###);
    Ok(())
}

#[test]
fn test_abbreviate_unchanged_sections() -> TestResult {
    let num_context_lines = 3;
    let section_length = num_context_lines * 2;
    let middle_length = section_length + 1;
    let state = RecordState {
        is_read_only: false,
        commits: Default::default(),
        files: vec![File {
            old_path: None,
            path: Cow::Borrowed(Path::new("foo")),
            file_mode: FileMode::FILE_DEFAULT,
            sections: vec![
                Section::Unchanged {
                    lines: (1..=section_length)
                        .map(|x| Cow::Owned(format!("start line {x}/{section_length}\n")))
                        .collect(),
                },
                Section::Changed {
                    lines: vec![SectionChangedLine {
                        is_checked: false,
                        change_type: ChangeType::Added,
                        line: Cow::Borrowed("changed\n"),
                    }],
                },
                Section::Unchanged {
                    lines: (1..=middle_length)
                        .map(|x| Cow::Owned(format!("middle line {x}/{middle_length}\n")))
                        .collect(),
                },
                Section::Changed {
                    lines: vec![SectionChangedLine {
                        is_checked: false,
                        change_type: ChangeType::Added,
                        line: Cow::Borrowed("changed\n"),
                    }],
                },
                Section::Unchanged {
                    lines: (1..=section_length)
                        .map(|x| Cow::Owned(format!("end line {x}/{section_length}\n")))
                        .collect(),
                },
            ],
        }],
    };

    let initial = TestingScreenshot::default();
    let collapse_bottom = TestingScreenshot::default();
    let collapse_top = TestingScreenshot::default();
    let expand_bottom = TestingScreenshot::default();
    let mut input = TestingInput::new(
        80,
        24,
        [
            Event::ExpandAll,
            initial.event(),
            Event::FocusNext,
            Event::FocusNext,
            Event::FocusNext,
            Event::ExpandItem,
            collapse_bottom.event(),
            Event::FocusPrev,
            Event::FocusPrev,
            Event::ExpandItem,
            collapse_top.event(),
            Event::FocusNext,
            Event::ExpandItem,
            expand_bottom.event(),
            Event::QuitAccept,
        ],
    );
    let recorder = Recorder::new(state, &mut input);
    recorder.run()?;
    insta::assert_snapshot!(initial, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "( ) foo                                                                      (-)"
    "        ⋮                                                                       "
    "        4 start line 4/6⏎                                                       "
    "        5 start line 5/6⏎                                                       "
    "        6 start line 6/6⏎                                                       "
    "  [ ] Section 1/2                                                            [-]"
    "    [ ] + changed⏎                                                              "
    "        7 middle line 1/7⏎                                                      "
    "        8 middle line 2/7⏎                                                      "
    "        9 middle line 3/7⏎                                                      "
    "        ⋮                                                                       "
    "       11 middle line 5/7⏎                                                      "
    "       12 middle line 6/7⏎                                                      "
    "       13 middle line 7/7⏎                                                      "
    "  [ ] Section 2/2                                                            [-]"
    "    [ ] + changed⏎                                                              "
    "       14 end line 1/6⏎                                                         "
    "       15 end line 2/6⏎                                                         "
    "       16 end line 3/6⏎                                                         "
    "        ⋮                                                                       "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "###);
    // Unchanged sections are collapsed unless there's at least one changed
    // section expanded before or after them.
    insta::assert_snapshot!(collapse_bottom, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "[ ] foo                                                                      [±]"
    "        ⋮                                                                       "
    "        4 start line 4/6⏎                                                       "
    "        5 start line 5/6⏎                                                       "
    "        6 start line 6/6⏎                                                       "
    "  [ ] Section 1/2                                                            [-]"
    "    [ ] + changed⏎                                                              "
    "        7 middle line 1/7⏎                                                      "
    "        8 middle line 2/7⏎                                                      "
    "        9 middle line 3/7⏎                                                      "
    "        ⋮                                                                       "
    "       11 middle line 5/7⏎                                                      "
    "       12 middle line 6/7⏎                                                      "
    "       13 middle line 7/7⏎                                                      "
    "  ( ) Section 2/2                                                            (+)"
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "###);
    insta::assert_snapshot!(collapse_top, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "[ ] foo                                                                      [±]"
    "  ( ) Section 1/2                                                            (+)"
    "  [ ] Section 2/2                                                            [+]"
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "###);
    insta::assert_snapshot!(expand_bottom, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "[ ] foo                                                                      [±]"
    "  [ ] Section 1/2                                                            [+]"
    "        7 middle line 1/7⏎                                                      "
    "        8 middle line 2/7⏎                                                      "
    "        9 middle line 3/7⏎                                                      "
    "        ⋮                                                                       "
    "       11 middle line 5/7⏎                                                      "
    "       12 middle line 6/7⏎                                                      "
    "       13 middle line 7/7⏎                                                      "
    "  ( ) Section 2/2                                                            (-)"
    "    [ ] + changed⏎                                                              "
    "       14 end line 1/6⏎                                                         "
    "       15 end line 2/6⏎                                                         "
    "       16 end line 3/6⏎                                                         "
    "        ⋮                                                                       "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "###);

    Ok(())
}

#[test]
fn test_no_abbreviate_short_unchanged_sections() -> TestResult {
    let num_context_lines = 3;
    let section_length = num_context_lines - 1;
    let middle_length = num_context_lines * 2;
    let state = RecordState {
        is_read_only: false,
        commits: Default::default(),
        files: vec![File {
            old_path: None,
            path: Cow::Borrowed(Path::new("foo")),
            file_mode: FileMode::FILE_DEFAULT,
            sections: vec![
                Section::Unchanged {
                    lines: (1..=section_length)
                        .map(|x| Cow::Owned(format!("start line {x}/{section_length}\n")))
                        .collect(),
                },
                Section::Changed {
                    lines: vec![SectionChangedLine {
                        is_checked: false,
                        change_type: ChangeType::Added,
                        line: Cow::Borrowed("changed\n"),
                    }],
                },
                Section::Unchanged {
                    lines: (1..=middle_length)
                        .map(|x| Cow::Owned(format!("middle line {x}/{middle_length}\n")))
                        .collect(),
                },
                Section::Changed {
                    lines: vec![SectionChangedLine {
                        is_checked: false,
                        change_type: ChangeType::Added,
                        line: Cow::Borrowed("changed\n"),
                    }],
                },
                Section::Unchanged {
                    lines: (1..=section_length)
                        .map(|x| Cow::Owned(format!("end line {x}/{section_length}\n")))
                        .collect(),
                },
            ],
        }],
    };

    let screenshot = TestingScreenshot::default();
    let mut input = TestingInput::new(
        80,
        20,
        [Event::ExpandAll, screenshot.event(), Event::QuitAccept],
    );
    let recorder = Recorder::new(state, &mut input);
    recorder.run()?;
    insta::assert_snapshot!(screenshot, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "( ) foo                                                                      (-)"
    "        1 start line 1/2⏎                                                       "
    "        2 start line 2/2⏎                                                       "
    "  [ ] Section 1/2                                                            [-]"
    "    [ ] + changed⏎                                                              "
    "        3 middle line 1/6⏎                                                      "
    "        4 middle line 2/6⏎                                                      "
    "        5 middle line 3/6⏎                                                      "
    "        6 middle line 4/6⏎                                                      "
    "        7 middle line 5/6⏎                                                      "
    "        8 middle line 6/6⏎                                                      "
    "  [ ] Section 2/2                                                            [-]"
    "    [ ] + changed⏎                                                              "
    "        9 end line 1/2⏎                                                         "
    "       10 end line 2/2⏎                                                         "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "###);

    Ok(())
}

#[test]
fn test_record_binary_file() -> TestResult {
    let state = RecordState {
        is_read_only: false,
        commits: Default::default(),
        files: vec![File {
            old_path: None,
            path: Cow::Borrowed(Path::new("foo")),
            file_mode: FileMode::FILE_DEFAULT,
            sections: vec![Section::Binary {
                is_checked: false,
                old_description: Some(Cow::Owned(make_binary_description("abc123", 123))),
                new_description: Some(Cow::Owned(make_binary_description("def456", 456))),
            }],
        }],
    };

    let initial = TestingScreenshot::default();
    let mut input = TestingInput::new(
        80,
        6,
        [
            Event::ExpandAll,
            initial.event(),
            Event::ToggleItem,
            Event::QuitAccept,
        ],
    );
    let recorder = Recorder::new(state, &mut input);
    let state = recorder.run()?;

    insta::assert_snapshot!(initial, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "( ) foo                                                                      (-)"
    "  [ ] (binary contents: abc123 (123 bytes) -> def456 (456 bytes))               "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "###);

    assert_debug_snapshot!(state, @r###"
    RecordState {
        is_read_only: false,
        commits: [
            Commit {
                message: None,
            },
            Commit {
                message: None,
            },
        ],
        files: [
            File {
                old_path: None,
                path: "foo",
                file_mode: Unix(
                    33188,
                ),
                sections: [
                    Binary {
                        is_checked: true,
                        old_description: Some(
                            "abc123 (123 bytes)",
                        ),
                        new_description: Some(
                            "def456 (456 bytes)",
                        ),
                    },
                ],
            },
        ],
    }
    "###);

    let (selected, unselected) = state.files[0].get_selected_contents();
    assert_debug_snapshot!(selected, @r###"
    SelectedChanges {
        file_mode: Unix(
            33188,
        ),
        contents: Binary {
            old_description: Some(
                "abc123 (123 bytes)",
            ),
            new_description: Some(
                "def456 (456 bytes)",
            ),
        },
    }
    "###);
    assert_debug_snapshot!(unselected, @r"
    SelectedChanges {
        file_mode: Unix(
            33188,
        ),
        contents: Unchanged,
    }
    ");

    Ok(())
}

#[test]
fn test_record_binary_file_noop() -> TestResult {
    let state = RecordState {
        is_read_only: false,
        commits: Default::default(),
        files: vec![File {
            old_path: None,
            path: Cow::Borrowed(Path::new("foo")),
            file_mode: FileMode::FILE_DEFAULT,
            sections: vec![Section::Binary {
                is_checked: false,
                old_description: Some(Cow::Owned(make_binary_description("abc123", 123))),
                new_description: Some(Cow::Owned(make_binary_description("def456", 456))),
            }],
        }],
    };

    let initial = TestingScreenshot::default();
    let mut input = TestingInput::new(
        80,
        6,
        [Event::ExpandAll, initial.event(), Event::QuitAccept],
    );
    let recorder = Recorder::new(state, &mut input);
    let state = recorder.run()?;

    insta::assert_snapshot!(initial, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "( ) foo                                                                      (-)"
    "  [ ] (binary contents: abc123 (123 bytes) -> def456 (456 bytes))               "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "###);

    assert_debug_snapshot!(state, @r###"
    RecordState {
        is_read_only: false,
        commits: [
            Commit {
                message: None,
            },
            Commit {
                message: None,
            },
        ],
        files: [
            File {
                old_path: None,
                path: "foo",
                file_mode: Unix(
                    33188,
                ),
                sections: [
                    Binary {
                        is_checked: false,
                        old_description: Some(
                            "abc123 (123 bytes)",
                        ),
                        new_description: Some(
                            "def456 (456 bytes)",
                        ),
                    },
                ],
            },
        ],
    }
    "###);

    let (selected, unselected) = state.files[0].get_selected_contents();
    assert_debug_snapshot!(selected, @r"
    SelectedChanges {
        file_mode: Unix(
            33188,
        ),
        contents: Unchanged,
    }
    ");
    assert_debug_snapshot!(unselected, @r###"
    SelectedChanges {
        file_mode: Unix(
            33188,
        ),
        contents: Binary {
            old_description: Some(
                "abc123 (123 bytes)",
            ),
            new_description: Some(
                "def456 (456 bytes)",
            ),
        },
    }
    "###);

    Ok(())
}

#[test]
fn test_state_binary_selected_contents() -> TestResult {
    let test = |is_checked, binary| {
        let file = File {
            old_path: None,
            path: Cow::Borrowed(Path::new("foo")),
            file_mode: FileMode::FILE_DEFAULT,
            sections: vec![
                Section::Changed {
                    lines: vec![SectionChangedLine {
                        is_checked,
                        change_type: ChangeType::Removed,
                        line: Cow::Borrowed("foo\n"),
                    }],
                },
                Section::Binary {
                    is_checked: binary,
                    old_description: Some(Cow::Owned(make_binary_description("abc123", 123))),
                    new_description: Some(Cow::Owned(make_binary_description("def456", 456))),
                },
            ],
        };
        let selection = file.get_selected_contents();
        format!("{selection:?}")
    };

    assert_snapshot!(test(false, false), @r###"(SelectedChanges { file_mode: Unix(33188), contents: Unchanged }, SelectedChanges { file_mode: Unix(33188), contents: Binary { old_description: Some("abc123 (123 bytes)"), new_description: Some("def456 (456 bytes)") } })"###);

    // FIXME: should the selected contents be `Present { contents: "" }`? (Or
    // possibly `Absent`?)
    assert_snapshot!(test(true, false), @r###"(SelectedChanges { file_mode: Unix(33188), contents: Unchanged }, SelectedChanges { file_mode: Unix(33188), contents: Binary { old_description: Some("abc123 (123 bytes)"), new_description: Some("def456 (456 bytes)") } })"###);

    // NB: The result for this situation, where we've selected both a text and
    // binary segment for inclusion, is arbitrary. The caller should avoid
    // generating both kinds of sections in the same file (or we should improve
    // the UI to never allow selecting both).
    assert_snapshot!(test(false, true), @r###"(SelectedChanges { file_mode: Unix(33188), contents: Binary { old_description: Some("abc123 (123 bytes)"), new_description: Some("def456 (456 bytes)") } }, SelectedChanges { file_mode: Unix(33188), contents: Unchanged })"###);

    assert_snapshot!(test(true, true), @r###"(SelectedChanges { file_mode: Unix(33188), contents: Binary { old_description: Some("abc123 (123 bytes)"), new_description: Some("def456 (456 bytes)") } }, SelectedChanges { file_mode: Unix(33188), contents: Unchanged })"###);

    Ok(())
}

#[test]
fn test_mouse_support() -> TestResult {
    let state = example_contents();

    let initial = TestingScreenshot::default();
    let first_click = TestingScreenshot::default();
    let click_scrolled_item = TestingScreenshot::default();
    let mut input = TestingInput::new(
        80,
        7,
        [
            Event::ExpandAll,
            initial.event(),
            Event::Click { row: 6, column: 8 },
            Event::EnsureSelectionInViewport,
            first_click.event(),
            Event::Click { row: 6, column: 8 },
            Event::EnsureSelectionInViewport,
            click_scrolled_item.event(),
            Event::QuitAccept,
        ],
    );
    let recorder = Recorder::new(state, &mut input);
    recorder.run()?;

    insta::assert_snapshot!(initial, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "(◐) foo/bar                                                                  (-)"
    "        ⋮                                                                       "
    "       18 this is some text⏎                                                    "
    "       19 this is some text⏎                                                    "
    "       20 this is some text⏎                                                    "
    "  [◐] Section 1/1                                                            [-]"
    "###);
    insta::assert_snapshot!(first_click, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "[◐] foo/bar                                                                  [-]"
    "  (◐) Section 1/1                                                            (-)"
    "    [●] - before text 1⏎                                                        "
    "    [●] - before text 2⏎                                                        "
    "    [●] + after text 1⏎                                                         "
    "    [ ] + after text 2⏎                                                         "
    "###);
    insta::assert_snapshot!(click_scrolled_item, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "[◐] foo/bar                                                                  [-]"
    "  [◐] Section 1/1                                                            [-]"
    "    [●] - before text 1⏎                                                        "
    "    [●] - before text 2⏎                                                        "
    "    [●] + after text 1⏎                                                         "
    "    ( ) + after text 2⏎                                                         "
    "###);

    Ok(())
}
#[test]
fn test_mouse_click_checkbox() -> TestResult {
    let state = RecordState {
        is_read_only: false,
        commits: Default::default(),
        files: vec![
            File {
                old_path: None,
                path: Cow::Borrowed(Path::new("foo")),
                file_mode: FileMode::FILE_DEFAULT,
                sections: vec![],
            },
            File {
                old_path: None,
                path: Cow::Borrowed(Path::new("bar")),
                file_mode: FileMode::Absent,
                sections: vec![Section::FileMode {
                    is_checked: false,
                    mode: FileMode::FILE_DEFAULT,
                }],
            },
        ],
    };

    let initial = TestingScreenshot::default();
    let click_unselected_checkbox = TestingScreenshot::default();
    let click_selected_checkbox = TestingScreenshot::default();
    let mut input = TestingInput::new(
        80,
        4,
        [
            Event::ExpandAll,
            initial.event(),
            Event::Click { row: 2, column: 1 },
            click_unselected_checkbox.event(),
            Event::Click { row: 2, column: 1 },
            click_selected_checkbox.event(),
            Event::QuitAccept,
        ],
    );
    let recorder = Recorder::new(state, &mut input);
    recorder.run()?;

    insta::assert_snapshot!(initial, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "( ) foo                                                                      (-)"
    "[ ] bar                                                                      [-]"
    "  [ ] File mode set to 100644                                                   "
    "###);
    insta::assert_snapshot!(click_unselected_checkbox, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "[ ] foo                                                                      [-]"
    "( ) bar                                                                      (-)"
    "  [ ] File mode set to 100644                                                   "
    "###);
    insta::assert_snapshot!(click_selected_checkbox, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "[ ] foo                                                                      [-]"
    "(●) bar                                                                      (-)"
    "  [●] File mode set to 100644                                                   "
    "###);

    Ok(())
}

#[test]
fn test_mouse_click_wide_line() -> TestResult {
    let state = RecordState {
        is_read_only: false,
        commits: Default::default(),
        files: vec![File {
            old_path: None,
            path: Cow::Borrowed(Path::new("foo")),
            file_mode: FileMode::Absent,
            sections: vec![
                Section::FileMode {
                    is_checked: false,
                    mode: FileMode::FILE_DEFAULT,
                },
                Section::Changed {
                    lines: vec![SectionChangedLine {
                        is_checked: false,
                        change_type: ChangeType::Removed,
                        line: Cow::Borrowed("foo\n"),
                    }],
                },
            ],
        }],
    };

    let initial = TestingScreenshot::default();
    let click_line = TestingScreenshot::default();
    let click_line_section = TestingScreenshot::default();
    let click_file_mode_section = TestingScreenshot::default();
    let click_file = TestingScreenshot::default();
    let mut input = TestingInput::new(
        80,
        5,
        [
            Event::ExpandAll,
            initial.event(),
            Event::Click { row: 4, column: 50 },
            click_line.event(),
            Event::Click { row: 3, column: 50 },
            click_line_section.event(),
            Event::Click { row: 2, column: 50 },
            click_file_mode_section.event(),
            Event::Click { row: 1, column: 50 },
            click_file.event(),
            Event::QuitAccept,
        ],
    );
    let recorder = Recorder::new(state, &mut input);
    recorder.run()?;

    insta::assert_snapshot!(initial, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "( ) foo                                                                      (-)"
    "  [ ] File mode set to 100644                                                   "
    "  [ ] Section 2/2                                                            [-]"
    "    [ ] - foo⏎                                                                  "
    "###);
    insta::assert_snapshot!(click_line, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "[ ] foo                                                                      [-]"
    "  [ ] File mode set to 100644                                                   "
    "  [ ] Section 2/2                                                            [-]"
    "    ( ) - foo⏎                                                                  "
    "###);
    insta::assert_snapshot!(click_line_section, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "[ ] foo                                                                      [-]"
    "  [ ] File mode set to 100644                                                   "
    "  ( ) Section 2/2                                                            (-)"
    "    [ ] - foo⏎                                                                  "
    "###);
    insta::assert_snapshot!(click_file_mode_section, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "[ ] foo                                                                      [-]"
    "  ( ) File mode set to 100644                                                   "
    "  [ ] Section 2/2                                                            [-]"
    "    [ ] - foo⏎                                                                  "
    "###);
    insta::assert_snapshot!(click_file, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "( ) foo                                                                      (-)"
    "  [ ] File mode set to 100644                                                   "
    "  [ ] Section 2/2                                                            [-]"
    "    [ ] - foo⏎                                                                  "
    "###);

    Ok(())
}

#[test]
fn test_mouse_click_dialog_buttons() -> TestResult {
    let state = RecordState {
        is_read_only: false,
        commits: Default::default(),
        files: vec![File {
            old_path: None,
            path: Cow::Borrowed(Path::new("foo")),
            file_mode: FileMode::FILE_DEFAULT,
            sections: vec![Section::Changed {
                lines: vec![SectionChangedLine {
                    is_checked: true,
                    change_type: ChangeType::Removed,
                    line: Cow::Borrowed("foo\n"),
                }],
            }],
        }],
    };

    let click_nothing = TestingScreenshot::default();
    let click_go_back = TestingScreenshot::default();
    let events = [
        Event::ExpandAll,
        Event::QuitCancel,
        Event::Click { row: 3, column: 55 },
        click_nothing.event(),
        Event::QuitCancel,
        Event::Click { row: 3, column: 65 },
        click_go_back.event(),
    ];
    let mut input = TestingInput::new(80, 6, events);
    let recorder = Recorder::new(state, &mut input);
    let result = recorder.run();
    insta::assert_debug_snapshot!(result, @r###"
    Err(
        Cancelled,
    )
    "###);

    insta::assert_snapshot!(click_nothing, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "(●) foo                                                                      (-)"
    "  [●] Section 1/1                                                            [-]"
    "    [●] - foo⏎                                                                  "
    "                                                                                "
    "                                                                                "
    "###);
    insta::assert_snapshot!(click_go_back, @"<this screenshot was never assigned>");

    Ok(())
}

#[test]
fn test_render_old_path() -> TestResult {
    let state = RecordState {
        is_read_only: false,
        commits: Default::default(),
        files: vec![File {
            old_path: Some(Cow::Borrowed(Path::new("foo"))),
            path: Cow::Borrowed(Path::new("bar")),
            file_mode: FileMode::FILE_DEFAULT,
            sections: vec![],
        }],
    };
    let screenshot = TestingScreenshot::default();
    let mut input = TestingInput::new(
        80,
        6,
        [Event::ExpandAll, screenshot.event(), Event::QuitAccept],
    );
    let recorder = Recorder::new(state, &mut input);
    recorder.run()?;

    insta::assert_snapshot!(screenshot, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "( ) foo => bar                                                               (-)"
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "###);

    Ok(())
}

#[test]
fn test_expand() -> TestResult {
    let state = example_contents();
    let initial = TestingScreenshot::default();
    let after_expand = TestingScreenshot::default();
    let after_collapse = TestingScreenshot::default();
    let after_expand_mouse = TestingScreenshot::default();
    let mut input = TestingInput::new(
        80,
        7,
        [
            initial.event(),
            Event::ExpandItem,
            after_expand.event(),
            Event::ExpandItem,
            after_collapse.event(),
            Event::Click { row: 2, column: 78 },
            Event::Click { row: 2, column: 78 },
            after_expand_mouse.event(),
            Event::QuitAccept,
        ],
    );
    let recorder = Recorder::new(state, &mut input);
    recorder.run()?;

    insta::assert_snapshot!(initial, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "(◐) foo/bar                                                                  (+)"
    "[●] baz                                                                      [+]"
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "###);
    insta::assert_snapshot!(after_expand, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "(◐) foo/bar                                                                  (-)"
    "        ⋮                                                                       "
    "       18 this is some text⏎                                                    "
    "       19 this is some text⏎                                                    "
    "       20 this is some text⏎                                                    "
    "  [◐] Section 1/1                                                            [-]"
    "###);
    insta::assert_snapshot!(after_collapse, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "(◐) foo/bar                                                                  (+)"
    "[●] baz                                                                      [+]"
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "###);
    insta::assert_snapshot!(after_expand_mouse, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "(●) baz                                                                      (-)"
    "        1 Some leading text 1⏎                                                  "
    "        2 Some leading text 2⏎                                                  "
    "  [●] Section 1/1                                                            [-]"
    "    [●] - before text 1⏎                                                        "
    "    [●] - before text 2⏎                                                        "
    "###);

    Ok(())
}

#[test]
fn test_expand_line_noop() -> TestResult {
    let state = example_contents();
    let after_select = TestingScreenshot::default();
    let after_expand_noop = TestingScreenshot::default();
    let mut input = TestingInput::new(
        80,
        7,
        [
            Event::ExpandAll,
            Event::FocusNext,
            Event::FocusNext,
            after_select.event(),
            Event::ExpandItem,
            after_expand_noop.event(),
            Event::QuitAccept,
        ],
    );
    let recorder = Recorder::new(state, &mut input);
    recorder.run()?;

    insta::assert_snapshot!(after_select, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "[◐] foo/bar                                                                  [-]"
    "  [◐] Section 1/1                                                            [-]"
    "    (●) - before text 1⏎                                                        "
    "    [●] - before text 2⏎                                                        "
    "    [●] + after text 1⏎                                                         "
    "    [ ] + after text 2⏎                                                         "
    "###);
    insta::assert_snapshot!(after_expand_noop, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "[◐] foo/bar                                                                  [-]"
    "  [◐] Section 1/1                                                            [-]"
    "    (●) - before text 1⏎                                                        "
    "    [●] - before text 2⏎                                                        "
    "    [●] + after text 1⏎                                                         "
    "    [ ] + after text 2⏎                                                         "
    "###);

    Ok(())
}

#[test]
fn test_expand_scroll_into_view() -> TestResult {
    let state = example_contents();
    let before_expand = TestingScreenshot::default();
    let after_expand = TestingScreenshot::default();
    let mut input = TestingInput::new(
        80,
        7,
        [
            Event::FocusNext,
            before_expand.event(),
            Event::ExpandAll,
            after_expand.event(),
            Event::QuitAccept,
        ],
    );
    let recorder = Recorder::new(state, &mut input);
    recorder.run()?;

    insta::assert_snapshot!(before_expand, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "[◐] foo/bar                                                                  [+]"
    "(●) baz                                                                      (+)"
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "###);
    insta::assert_snapshot!(after_expand, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "(●) baz                                                                      (-)"
    "        1 Some leading text 1⏎                                                  "
    "        2 Some leading text 2⏎                                                  "
    "  [●] Section 1/1                                                            [-]"
    "    [●] - before text 1⏎                                                        "
    "    [●] - before text 2⏎                                                        "
    "###);

    Ok(())
}

#[test]
fn test_collapse_select_ancestor() -> TestResult {
    let state = example_contents();
    let before_collapse = TestingScreenshot::default();
    let after_collapse = TestingScreenshot::default();
    let mut input = TestingInput::new(
        80,
        7,
        [
            Event::ExpandAll,
            Event::FocusNext,
            before_collapse.event(),
            Event::ExpandAll,
            after_collapse.event(),
            Event::QuitAccept,
        ],
    );
    let recorder = Recorder::new(state, &mut input);
    recorder.run()?;

    insta::assert_snapshot!(before_collapse, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "[◐] foo/bar                                                                  [-]"
    "  (◐) Section 1/1                                                            (-)"
    "    [●] - before text 1⏎                                                        "
    "    [●] - before text 2⏎                                                        "
    "    [●] + after text 1⏎                                                         "
    "    [ ] + after text 2⏎                                                         "
    "###);
    insta::assert_snapshot!(after_collapse, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "(◐) foo/bar                                                                  (+)"
    "[●] baz                                                                      [+]"
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "###);

    Ok(())
}

#[test]
fn test_focus_inner() -> TestResult {
    let state = example_contents();
    let initial = TestingScreenshot::default();
    let inner1 = TestingScreenshot::default();
    let inner2 = TestingScreenshot::default();
    let inner3 = TestingScreenshot::default();
    let mut input = TestingInput::new(
        80,
        7,
        [
            initial.event(),
            Event::FocusInner,
            inner1.event(),
            Event::FocusInner,
            inner2.event(),
            Event::FocusInner,
            inner3.event(),
            Event::QuitAccept,
        ],
    );
    let recorder = Recorder::new(state, &mut input);
    recorder.run()?;

    insta::assert_snapshot!(initial, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "(◐) foo/bar                                                                  (+)"
    "[●] baz                                                                      [+]"
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "###);
    insta::assert_snapshot!(inner1, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "[◐] foo/bar                                                                  [-]"
    "  (◐) Section 1/1                                                            (-)"
    "    [●] - before text 1⏎                                                        "
    "    [●] - before text 2⏎                                                        "
    "    [●] + after text 1⏎                                                         "
    "    [ ] + after text 2⏎                                                         "
    "###);
    insta::assert_snapshot!(inner2, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "[◐] foo/bar                                                                  [-]"
    "  [◐] Section 1/1                                                            [-]"
    "    (●) - before text 1⏎                                                        "
    "    [●] - before text 2⏎                                                        "
    "    [●] + after text 1⏎                                                         "
    "    [ ] + after text 2⏎                                                         "
    "###);
    insta::assert_snapshot!(inner3, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "[◐] foo/bar                                                                  [-]"
    "  [◐] Section 1/1                                                            [-]"
    "    (●) - before text 1⏎                                                        "
    "    [●] - before text 2⏎                                                        "
    "    [●] + after text 1⏎                                                         "
    "    [ ] + after text 2⏎                                                         "
    "###);

    Ok(())
}

#[test]
fn test_focus_outer() -> TestResult {
    let state = example_contents();
    let initial = TestingScreenshot::default();
    let outer1 = TestingScreenshot::default();
    let outer2 = TestingScreenshot::default();
    let outer3 = TestingScreenshot::default();
    let outer4 = TestingScreenshot::default();
    let mut input = TestingInput::new(
        80,
        7,
        [
            Event::FocusNext,
            Event::ExpandItem,
            Event::FocusNext,
            Event::FocusNext,
            Event::FocusNext,
            initial.event(),
            Event::FocusOuter {
                fold_section: false,
            },
            outer1.event(),
            Event::FocusOuter {
                fold_section: false,
            },
            outer2.event(),
            Event::FocusOuter {
                fold_section: false,
            },
            outer3.event(),
            Event::FocusOuter {
                fold_section: false,
            },
            outer4.event(),
            Event::QuitAccept,
        ],
    );
    let recorder = Recorder::new(state, &mut input);
    recorder.run()?;

    insta::assert_snapshot!(initial, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "[●] baz                                                                      [-]"
    "  [●] Section 1/1                                                            [-]"
    "    [●] - before text 1⏎                                                        "
    "    (●) - before text 2⏎                                                        "
    "    [●] + after text 1⏎                                                         "
    "    [●] + after text 2⏎                                                         "
    "###);
    insta::assert_snapshot!(outer1, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "[●] baz                                                                      [-]"
    "  (●) Section 1/1                                                            (-)"
    "    [●] - before text 1⏎                                                        "
    "    [●] - before text 2⏎                                                        "
    "    [●] + after text 1⏎                                                         "
    "    [●] + after text 2⏎                                                         "
    "###);
    insta::assert_snapshot!(outer2, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "(●) baz                                                                      (-)"
    "        1 Some leading text 1⏎                                                  "
    "        2 Some leading text 2⏎                                                  "
    "  [●] Section 1/1                                                            [-]"
    "    [●] - before text 1⏎                                                        "
    "    [●] - before text 2⏎                                                        "
    "###);
    insta::assert_snapshot!(outer3, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "(●) baz                                                                      (+)"
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "###);
    insta::assert_snapshot!(outer4, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "(●) baz                                                                      (+)"
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "###);

    Ok(())
}

#[test]
fn test_focus_outer_fold_section() -> TestResult {
    let state = example_contents();
    let initial = TestingScreenshot::default();
    let outer1 = TestingScreenshot::default();
    let outer2 = TestingScreenshot::default();
    let outer3 = TestingScreenshot::default();
    let outer4 = TestingScreenshot::default();
    let outer5 = TestingScreenshot::default();
    let mut input = TestingInput::new(
        80,
        7,
        [
            Event::FocusNext,
            Event::ExpandItem,
            Event::FocusNext,
            Event::FocusNext,
            Event::FocusNext,
            initial.event(),
            Event::FocusOuter { fold_section: true },
            outer1.event(),
            Event::FocusOuter { fold_section: true },
            outer2.event(),
            Event::FocusOuter { fold_section: true },
            outer3.event(),
            Event::FocusOuter { fold_section: true },
            outer4.event(),
            Event::FocusOuter { fold_section: true },
            outer5.event(),
            Event::QuitAccept,
        ],
    );
    let recorder = Recorder::new(state, &mut input);
    recorder.run()?;

    insta::assert_snapshot!(initial, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "[●] baz                                                                      [-]"
    "  [●] Section 1/1                                                            [-]"
    "    [●] - before text 1⏎                                                        "
    "    (●) - before text 2⏎                                                        "
    "    [●] + after text 1⏎                                                         "
    "    [●] + after text 2⏎                                                         "
    "###);
    insta::assert_snapshot!(outer1, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "[●] baz                                                                      [-]"
    "  (●) Section 1/1                                                            (-)"
    "    [●] - before text 1⏎                                                        "
    "    [●] - before text 2⏎                                                        "
    "    [●] + after text 1⏎                                                         "
    "    [●] + after text 2⏎                                                         "
    "###);
    insta::assert_snapshot!(outer2, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "[●] baz                                                                      [±]"
    "  (●) Section 1/1                                                            (+)"
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "###);
    insta::assert_snapshot!(outer3, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "(●) baz                                                                      (±)"
    "  [●] Section 1/1                                                            [+]"
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "###);
    insta::assert_snapshot!(outer4, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "(●) baz                                                                      (+)"
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "###);
    insta::assert_snapshot!(outer5, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "(●) baz                                                                      (+)"
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "###);

    Ok(())
}

#[test]
fn test_sticky_header_scroll() -> TestResult {
    let state = example_contents();
    let initial = TestingScreenshot::default();
    let scroll1 = TestingScreenshot::default();
    let scroll2 = TestingScreenshot::default();
    let scroll3 = TestingScreenshot::default();
    let scroll4 = TestingScreenshot::default();
    let scroll5 = TestingScreenshot::default();
    let mut input = TestingInput::new(
        80,
        7,
        [
            Event::ExpandAll,
            initial.event(),
            Event::ScrollDown,
            scroll1.event(),
            Event::ScrollDown,
            scroll2.event(),
            Event::ScrollDown,
            scroll3.event(),
            Event::ScrollDown,
            scroll4.event(),
            Event::ScrollDown,
            scroll5.event(),
            Event::QuitAccept,
        ],
    );
    let recorder = Recorder::new(state, &mut input);
    recorder.run()?;

    insta::assert_snapshot!(initial, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "(◐) foo/bar                                                                  (-)"
    "        ⋮                                                                       "
    "       18 this is some text⏎                                                    "
    "       19 this is some text⏎                                                    "
    "       20 this is some text⏎                                                    "
    "  [◐] Section 1/1                                                            [-]"
    "###);
    insta::assert_snapshot!(scroll1, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "(◐) foo/bar                                                                  (-)"
    "       18 this is some text⏎                                                    "
    "       19 this is some text⏎                                                    "
    "       20 this is some text⏎                                                    "
    "  [◐] Section 1/1                                                            [-]"
    "    [●] - before text 1⏎                                                        "
    "###);
    insta::assert_snapshot!(scroll2, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "(◐) foo/bar                                                                  (-)"
    "       19 this is some text⏎                                                    "
    "       20 this is some text⏎                                                    "
    "  [◐] Section 1/1                                                            [-]"
    "    [●] - before text 1⏎                                                        "
    "    [●] - before text 2⏎                                                        "
    "###);
    insta::assert_snapshot!(scroll3, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "(◐) foo/bar                                                                  (-)"
    "       20 this is some text⏎                                                    "
    "  [◐] Section 1/1                                                            [-]"
    "    [●] - before text 1⏎                                                        "
    "    [●] - before text 2⏎                                                        "
    "    [●] + after text 1⏎                                                         "
    "###);
    insta::assert_snapshot!(scroll4, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "(◐) foo/bar                                                                  (-)"
    "  [◐] Section 1/1                                                            [-]"
    "    [●] - before text 1⏎                                                        "
    "    [●] - before text 2⏎                                                        "
    "    [●] + after text 1⏎                                                         "
    "    [ ] + after text 2⏎                                                         "
    "###);
    insta::assert_snapshot!(scroll5, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "(◐) foo/bar                                                                  (-)"
    "    [●] - before text 1⏎                                                        "
    "    [●] - before text 2⏎                                                        "
    "    [●] + after text 1⏎                                                         "
    "    [ ] + after text 2⏎                                                         "
    "       23 this is some trailing text⏎                                           "
    "###);

    Ok(())
}

#[test]
fn test_sticky_header_click_expand() -> TestResult {
    let state = example_contents();
    let initial = TestingScreenshot::default();
    let after_scroll = TestingScreenshot::default();
    let after_click1 = TestingScreenshot::default();
    let after_click2 = TestingScreenshot::default();
    let mut input = TestingInput::new(
        80,
        7,
        [
            initial.event(),
            Event::FocusNext,
            Event::ExpandItem,
            Event::FocusNext,
            after_scroll.event(),
            Event::Click { row: 1, column: 70 },
            after_click1.event(),
            Event::Click { row: 1, column: 78 },
            after_click2.event(),
            Event::QuitAccept,
        ],
    );
    let recorder = Recorder::new(state, &mut input);
    recorder.run()?;

    insta::assert_snapshot!(initial, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "(◐) foo/bar                                                                  (+)"
    "[●] baz                                                                      [+]"
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "###);
    insta::assert_snapshot!(after_scroll, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "[●] baz                                                                      [-]"
    "  (●) Section 1/1                                                            (-)"
    "    [●] - before text 1⏎                                                        "
    "    [●] - before text 2⏎                                                        "
    "    [●] + after text 1⏎                                                         "
    "    [●] + after text 2⏎                                                         "
    "###);
    insta::assert_snapshot!(after_click1, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "(●) baz                                                                      (-)"
    "  [●] Section 1/1                                                            [-]"
    "    [●] - before text 1⏎                                                        "
    "    [●] - before text 2⏎                                                        "
    "    [●] + after text 1⏎                                                         "
    "    [●] + after text 2⏎                                                         "
    "###);
    insta::assert_snapshot!(after_click2, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "(●) baz                                                                      (+)"
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "###);

    Ok(())
}

#[test]
fn test_scroll_click_no_jump() -> TestResult {
    let state = example_contents();
    let initial = TestingScreenshot::default();
    let after_click = TestingScreenshot::default();
    let mut input = TestingInput::new(
        80,
        7,
        [
            Event::ExpandAll,
            initial.event(),
            Event::Click { row: 5, column: 5 },
            after_click.event(),
            Event::QuitAccept,
        ],
    );
    let recorder = Recorder::new(state, &mut input);
    recorder.run()?;

    insta::assert_snapshot!(initial, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "(◐) foo/bar                                                                  (-)"
    "        ⋮                                                                       "
    "       18 this is some text⏎                                                    "
    "       19 this is some text⏎                                                    "
    "       20 this is some text⏎                                                    "
    "  [◐] Section 1/1                                                            [-]"
    "###);
    insta::assert_snapshot!(after_click, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "[◐] foo/bar                                                                  [-]"
    "        ⋮                                                                       "
    "       18 this is some text⏎                                                    "
    "       19 this is some text⏎                                                    "
    "       20 this is some text⏎                                                    "
    "  [◐] Section 1/1                                                            [-]"
    "###);

    Ok(())
}

#[test]
fn test_menu_bar_scroll_into_view() -> TestResult {
    let state = example_contents();
    let initial = TestingScreenshot::default();
    let after_scroll1 = TestingScreenshot::default();
    let after_scroll2 = TestingScreenshot::default();
    let mut input = TestingInput::new(
        80,
        6,
        [
            initial.event(),
            Event::ScrollDown,
            after_scroll1.event(),
            Event::ScrollDown,
            after_scroll2.event(),
            Event::QuitAccept,
        ],
    );
    let recorder = Recorder::new(state, &mut input);
    recorder.run()?;

    insta::assert_snapshot!(initial, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "(◐) foo/bar                                                                  (+)"
    "[●] baz                                                                      [+]"
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "###);
    insta::assert_snapshot!(after_scroll1, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "[●] baz                                                                      [+]"
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "###);
    insta::assert_snapshot!(after_scroll2, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "[●] baz                                                                      [+]"
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "###);

    Ok(())
}

#[test]
fn test_expand_menu() -> TestResult {
    let state = example_contents();
    let initial = TestingScreenshot::default();
    let after_click = TestingScreenshot::default();
    let after_click_different = TestingScreenshot::default();
    let after_click_same = TestingScreenshot::default();
    let after_click_menu_bar = TestingScreenshot::default();
    let mut input = TestingInput::new(
        80,
        6,
        [
            initial.event(),
            Event::Click { row: 0, column: 8 },
            after_click.event(),
            Event::Click { row: 0, column: 0 },
            after_click_different.event(),
            Event::Click { row: 0, column: 0 },
            after_click_same.event(),
            Event::Click { row: 0, column: 79 },
            after_click_menu_bar.event(),
            Event::QuitAccept,
        ],
    );
    let recorder = Recorder::new(state, &mut input);
    recorder.run()?;

    insta::assert_snapshot!(initial, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "(◐) foo/bar                                                                  (+)"
    "[●] baz                                                                      [+]"
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "###);
    insta::assert_snapshot!(after_click, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "(◐) foo[Edit message (e)]                                                    (+)"
    "[●] baz[Toggle current (space)]                                              [+]"
    "       [Toggle current and advance (enter)]                                     "
    "       [Invert all items (a)]                                                   "
    "       [Invert all items uniformly (A)]                                         "
    "###);
    insta::assert_snapshot!(after_click_different, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "[Confirm (c)]                                                                (+)"
    "[Quit (q)]                                                                   [+]"
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "###);
    insta::assert_snapshot!(after_click_same, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "(◐) foo/bar                                                                  (+)"
    "[●] baz                                                                      [+]"
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "###);
    insta::assert_snapshot!(after_click_menu_bar, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "(◐) foo/bar                                                                  (+)"
    "[●] baz                                                                      [+]"
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "###);

    Ok(())
}

#[test]
fn test_read_only() -> TestResult {
    let state = RecordState {
        is_read_only: true,
        ..example_contents()
    };
    let initial = TestingScreenshot::default();
    let after_toggle_all_ignored = TestingScreenshot::default();
    let after_toggle_all_uniform_ignored = TestingScreenshot::default();
    let mut input = TestingInput::new(
        80,
        23,
        [
            Event::ExpandAll,
            initial.event(),
            Event::ToggleAll,
            after_toggle_all_ignored.event(),
            Event::ToggleAllUniform,
            after_toggle_all_uniform_ignored.event(),
            Event::QuitAccept,
        ],
    );
    let recorder = Recorder::new(state, &mut input);
    let state = recorder.run()?;

    insta::assert_snapshot!(initial, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "<◐> foo/bar                                                                  (-)"
    "        ⋮                                                                       "
    "       18 this is some text⏎                                                    "
    "       19 this is some text⏎                                                    "
    "       20 this is some text⏎                                                    "
    "  <◐> Section 1/1                                                            [-]"
    "    <●> - before text 1⏎                                                        "
    "    <●> - before text 2⏎                                                        "
    "    <●> + after text 1⏎                                                         "
    "    < > + after text 2⏎                                                         "
    "       23 this is some trailing text⏎                                           "
    "<●> baz                                                                      [-]"
    "        1 Some leading text 1⏎                                                  "
    "        2 Some leading text 2⏎                                                  "
    "  <●> Section 1/1                                                            [-]"
    "    <●> - before text 1⏎                                                        "
    "    <●> - before text 2⏎                                                        "
    "    <●> + after text 1⏎                                                         "
    "    <●> + after text 2⏎                                                         "
    "        5 this is some trailing text⏎                                           "
    "                                                                                "
    "                                                                                "
    "###);
    insta::assert_snapshot!(after_toggle_all_ignored, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "<◐> foo/bar                                                                  (-)"
    "        ⋮                                                                       "
    "       18 this is some text⏎                                                    "
    "       19 this is some text⏎                                                    "
    "       20 this is some text⏎                                                    "
    "  <◐> Section 1/1                                                            [-]"
    "    <●> - before text 1⏎                                                        "
    "    <●> - before text 2⏎                                                        "
    "    <●> + after text 1⏎                                                         "
    "    < > + after text 2⏎                                                         "
    "       23 this is some trailing text⏎                                           "
    "<●> baz                                                                      [-]"
    "        1 Some leading text 1⏎                                                  "
    "        2 Some leading text 2⏎                                                  "
    "  <●> Section 1/1                                                            [-]"
    "    <●> - before text 1⏎                                                        "
    "    <●> - before text 2⏎                                                        "
    "    <●> + after text 1⏎                                                         "
    "    <●> + after text 2⏎                                                         "
    "        5 this is some trailing text⏎                                           "
    "                                                                                "
    "                                                                                "
    "###);
    insta::assert_snapshot!(after_toggle_all_uniform_ignored, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "<◐> foo/bar                                                                  (-)"
    "        ⋮                                                                       "
    "       18 this is some text⏎                                                    "
    "       19 this is some text⏎                                                    "
    "       20 this is some text⏎                                                    "
    "  <◐> Section 1/1                                                            [-]"
    "    <●> - before text 1⏎                                                        "
    "    <●> - before text 2⏎                                                        "
    "    <●> + after text 1⏎                                                         "
    "    < > + after text 2⏎                                                         "
    "       23 this is some trailing text⏎                                           "
    "<●> baz                                                                      [-]"
    "        1 Some leading text 1⏎                                                  "
    "        2 Some leading text 2⏎                                                  "
    "  <●> Section 1/1                                                            [-]"
    "    <●> - before text 1⏎                                                        "
    "    <●> - before text 2⏎                                                        "
    "    <●> + after text 1⏎                                                         "
    "    <●> + after text 2⏎                                                         "
    "        5 this is some trailing text⏎                                           "
    "                                                                                "
    "                                                                                "
    "###);

    insta::assert_debug_snapshot!(state, @r###"
    RecordState {
        is_read_only: true,
        commits: [
            Commit {
                message: None,
            },
            Commit {
                message: None,
            },
        ],
        files: [
            File {
                old_path: None,
                path: "foo/bar",
                file_mode: Unix(
                    33188,
                ),
                sections: [
                    Unchanged {
                        lines: [
                            "this is some text\n",
                            "this is some text\n",
                            "this is some text\n",
                            "this is some text\n",
                            "this is some text\n",
                            "this is some text\n",
                            "this is some text\n",
                            "this is some text\n",
                            "this is some text\n",
                            "this is some text\n",
                            "this is some text\n",
                            "this is some text\n",
                            "this is some text\n",
                            "this is some text\n",
                            "this is some text\n",
                            "this is some text\n",
                            "this is some text\n",
                            "this is some text\n",
                            "this is some text\n",
                            "this is some text\n",
                        ],
                    },
                    Changed {
                        lines: [
                            SectionChangedLine {
                                is_checked: true,
                                change_type: Removed,
                                line: "before text 1\n",
                            },
                            SectionChangedLine {
                                is_checked: true,
                                change_type: Removed,
                                line: "before text 2\n",
                            },
                            SectionChangedLine {
                                is_checked: true,
                                change_type: Added,
                                line: "after text 1\n",
                            },
                            SectionChangedLine {
                                is_checked: false,
                                change_type: Added,
                                line: "after text 2\n",
                            },
                        ],
                    },
                    Unchanged {
                        lines: [
                            "this is some trailing text\n",
                        ],
                    },
                ],
            },
            File {
                old_path: None,
                path: "baz",
                file_mode: Unix(
                    33188,
                ),
                sections: [
                    Unchanged {
                        lines: [
                            "Some leading text 1\n",
                            "Some leading text 2\n",
                        ],
                    },
                    Changed {
                        lines: [
                            SectionChangedLine {
                                is_checked: true,
                                change_type: Removed,
                                line: "before text 1\n",
                            },
                            SectionChangedLine {
                                is_checked: true,
                                change_type: Removed,
                                line: "before text 2\n",
                            },
                            SectionChangedLine {
                                is_checked: true,
                                change_type: Added,
                                line: "after text 1\n",
                            },
                            SectionChangedLine {
                                is_checked: true,
                                change_type: Added,
                                line: "after text 2\n",
                            },
                        ],
                    },
                    Unchanged {
                        lines: [
                            "this is some trailing text\n",
                        ],
                    },
                ],
            },
        ],
    }
    "###);

    Ok(())
}

#[test]
fn test_toggle_unchanged_line() -> TestResult {
    let state = example_contents();
    let initial = TestingScreenshot::default();
    let after_toggle = TestingScreenshot::default();
    let mut input = TestingInput::new(
        80,
        6,
        [
            Event::ExpandAll,
            initial.event(),
            Event::Click { row: 4, column: 10 },
            Event::ToggleItem, // should not crash
            after_toggle.event(),
            Event::QuitAccept,
        ],
    );
    let recorder = Recorder::new(state, &mut input);
    let state = recorder.run()?;

    insta::assert_snapshot!(initial, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "(◐) foo/bar                                                                  (-)"
    "        ⋮                                                                       "
    "       18 this is some text⏎                                                    "
    "       19 this is some text⏎                                                    "
    "       20 this is some text⏎                                                    "
    "###);

    insta::assert_debug_snapshot!(state, @r###"
    RecordState {
        is_read_only: false,
        commits: [
            Commit {
                message: None,
            },
            Commit {
                message: None,
            },
        ],
        files: [
            File {
                old_path: None,
                path: "foo/bar",
                file_mode: Unix(
                    33188,
                ),
                sections: [
                    Unchanged {
                        lines: [
                            "this is some text\n",
                            "this is some text\n",
                            "this is some text\n",
                            "this is some text\n",
                            "this is some text\n",
                            "this is some text\n",
                            "this is some text\n",
                            "this is some text\n",
                            "this is some text\n",
                            "this is some text\n",
                            "this is some text\n",
                            "this is some text\n",
                            "this is some text\n",
                            "this is some text\n",
                            "this is some text\n",
                            "this is some text\n",
                            "this is some text\n",
                            "this is some text\n",
                            "this is some text\n",
                            "this is some text\n",
                        ],
                    },
                    Changed {
                        lines: [
                            SectionChangedLine {
                                is_checked: true,
                                change_type: Removed,
                                line: "before text 1\n",
                            },
                            SectionChangedLine {
                                is_checked: true,
                                change_type: Removed,
                                line: "before text 2\n",
                            },
                            SectionChangedLine {
                                is_checked: true,
                                change_type: Added,
                                line: "after text 1\n",
                            },
                            SectionChangedLine {
                                is_checked: false,
                                change_type: Added,
                                line: "after text 2\n",
                            },
                        ],
                    },
                    Unchanged {
                        lines: [
                            "this is some trailing text\n",
                        ],
                    },
                ],
            },
            File {
                old_path: None,
                path: "baz",
                file_mode: Unix(
                    33188,
                ),
                sections: [
                    Unchanged {
                        lines: [
                            "Some leading text 1\n",
                            "Some leading text 2\n",
                        ],
                    },
                    Changed {
                        lines: [
                            SectionChangedLine {
                                is_checked: true,
                                change_type: Removed,
                                line: "before text 1\n",
                            },
                            SectionChangedLine {
                                is_checked: true,
                                change_type: Removed,
                                line: "before text 2\n",
                            },
                            SectionChangedLine {
                                is_checked: true,
                                change_type: Added,
                                line: "after text 1\n",
                            },
                            SectionChangedLine {
                                is_checked: true,
                                change_type: Added,
                                line: "after text 2\n",
                            },
                        ],
                    },
                    Unchanged {
                        lines: [
                            "this is some trailing text\n",
                        ],
                    },
                ],
            },
        ],
    }
    "###);

    Ok(())
}

#[test]
fn test_max_file_view_width() -> TestResult {
    let state = RecordState {
        is_read_only: false,
        commits: Default::default(),
        files: vec![File {
            old_path: None,
            path: Cow::Owned("very/".repeat(100).into()),
            file_mode: FileMode::FILE_DEFAULT,
            sections: vec![
                Section::Unchanged {
                    lines: vec![Cow::Owned("very ".repeat(100))],
                },
                Section::Changed {
                    lines: vec![SectionChangedLine {
                        is_checked: false,
                        change_type: ChangeType::Added,
                        line: Cow::Owned("very ".repeat(100)),
                    }],
                },
            ],
        }],
    };
    let initial_wide = TestingScreenshot::default();
    let mut input = TestingInput::new(
        250,
        6,
        [
            Event::ExpandAll,
            Event::ToggleCommitViewMode,
            initial_wide.event(),
            Event::QuitAccept,
        ],
    );
    let recorder = Recorder::new(state.clone(), &mut input);
    recorder.run()?;

    insta::assert_snapshot!(initial_wide, @r###"
    "[File] [Edit] [Select] [View]                                                                                                                                                                                                                             "
    "( ) very/very/very/very/very/very/very/very/very/very/very/very/very/very/very/very/very/very/very/very/very/very/ve…(-) [ ] very/very/very/very/very/very/very/very/very/very/very/very/very/very/very/very/very/very/very/very/very/very/ve…[+]         "
    "        1 very very very very very very very very very very very very very very very very very very very very very very…                                                                                                                                  "
    "  [ ] Section 1/1                                                                                                    [-]                                                                                                                                  "
    "    [ ] + very very very very very very very very very very very very very very very very very very very very very very…                                                                                                                                  "
    "                                                                                                                                                                                                                                                          "
    "###);

    let initial_narrow = TestingScreenshot::default();
    let mut input = TestingInput::new(
        15,
        6,
        [Event::ExpandAll, initial_narrow.event(), Event::QuitAccept],
    );
    let recorder = Recorder::new(state, &mut input);
    recorder.run()?;

    insta::assert_snapshot!(initial_narrow, @r###"
    "[File] [Edit] ["
    "( ) very/ve…(-)"
    "        1 very…"
    "  [ ] Secti…[-]"
    "    [ ] + very…"
    "               "
    "###);

    Ok(())
}

#[test]
fn test_commit_message_view() -> TestResult {
    let mut state = example_contents();
    state.commits = vec![Commit {
        message: Some("".to_string()),
    }];

    let initial = TestingScreenshot::default();
    let after_edit = TestingScreenshot::default();
    let after_scroll1 = TestingScreenshot::default();
    let after_scroll2 = TestingScreenshot::default();
    let mut input = TestingInput {
        width: 80,
        height: 24,
        events: Box::new(
            [
                Event::ExpandAll,
                initial.event(),
                Event::EditCommitMessage,
                after_edit.event(),
                Event::ScrollDown,
                after_scroll1.event(),
                Event::ScrollDown,
                Event::ScrollDown,
                Event::ScrollDown,
                after_scroll2.event(),
                Event::QuitAccept,
            ]
            .into_iter(),
        ),
        commit_messages: ["Hello, world!".to_string()].into_iter().collect(),
    };
    let recorder = Recorder::new(state, &mut input);
    recorder.run()?;

    insta::assert_snapshot!(initial, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "                                                                                "
    "[Edit message]  •  (no message)                                                 "
    "                                                                                "
    "(◐) foo/bar                                                                  (-)"
    "        ⋮                                                                       "
    "       18 this is some text⏎                                                    "
    "       19 this is some text⏎                                                    "
    "       20 this is some text⏎                                                    "
    "  [◐] Section 1/1                                                            [-]"
    "    [●] - before text 1⏎                                                        "
    "    [●] - before text 2⏎                                                        "
    "    [●] + after text 1⏎                                                         "
    "    [ ] + after text 2⏎                                                         "
    "       23 this is some trailing text⏎                                           "
    "[●] baz                                                                      [-]"
    "        1 Some leading text 1⏎                                                  "
    "        2 Some leading text 2⏎                                                  "
    "  [●] Section 1/1                                                            [-]"
    "    [●] - before text 1⏎                                                        "
    "    [●] - before text 2⏎                                                        "
    "    [●] + after text 1⏎                                                         "
    "    [●] + after text 2⏎                                                         "
    "        5 this is some trailing text⏎                                           "
    "###);
    insta::assert_snapshot!(after_edit, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "                                                                                "
    "[Edit message]  •  Hello, world!                                                "
    "                                                                                "
    "(◐) foo/bar                                                                  (-)"
    "        ⋮                                                                       "
    "       18 this is some text⏎                                                    "
    "       19 this is some text⏎                                                    "
    "       20 this is some text⏎                                                    "
    "  [◐] Section 1/1                                                            [-]"
    "    [●] - before text 1⏎                                                        "
    "    [●] - before text 2⏎                                                        "
    "    [●] + after text 1⏎                                                         "
    "    [ ] + after text 2⏎                                                         "
    "       23 this is some trailing text⏎                                           "
    "[●] baz                                                                      [-]"
    "        1 Some leading text 1⏎                                                  "
    "        2 Some leading text 2⏎                                                  "
    "  [●] Section 1/1                                                            [-]"
    "    [●] - before text 1⏎                                                        "
    "    [●] - before text 2⏎                                                        "
    "    [●] + after text 1⏎                                                         "
    "    [●] + after text 2⏎                                                         "
    "        5 this is some trailing text⏎                                           "
    "###);
    insta::assert_snapshot!(after_scroll1, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "[Edit message]  •  Hello, world!                                                "
    "                                                                                "
    "(◐) foo/bar                                                                  (-)"
    "        ⋮                                                                       "
    "       18 this is some text⏎                                                    "
    "       19 this is some text⏎                                                    "
    "       20 this is some text⏎                                                    "
    "  [◐] Section 1/1                                                            [-]"
    "    [●] - before text 1⏎                                                        "
    "    [●] - before text 2⏎                                                        "
    "    [●] + after text 1⏎                                                         "
    "    [ ] + after text 2⏎                                                         "
    "       23 this is some trailing text⏎                                           "
    "[●] baz                                                                      [-]"
    "        1 Some leading text 1⏎                                                  "
    "        2 Some leading text 2⏎                                                  "
    "  [●] Section 1/1                                                            [-]"
    "    [●] - before text 1⏎                                                        "
    "    [●] - before text 2⏎                                                        "
    "    [●] + after text 1⏎                                                         "
    "    [●] + after text 2⏎                                                         "
    "        5 this is some trailing text⏎                                           "
    "                                                                                "
    "###);
    insta::assert_snapshot!(after_scroll2, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "(◐) foo/bar                                                                  (-)"
    "       18 this is some text⏎                                                    "
    "       19 this is some text⏎                                                    "
    "       20 this is some text⏎                                                    "
    "  [◐] Section 1/1                                                            [-]"
    "    [●] - before text 1⏎                                                        "
    "    [●] - before text 2⏎                                                        "
    "    [●] + after text 1⏎                                                         "
    "    [ ] + after text 2⏎                                                         "
    "       23 this is some trailing text⏎                                           "
    "[●] baz                                                                      [-]"
    "        1 Some leading text 1⏎                                                  "
    "        2 Some leading text 2⏎                                                  "
    "  [●] Section 1/1                                                            [-]"
    "    [●] - before text 1⏎                                                        "
    "    [●] - before text 2⏎                                                        "
    "    [●] + after text 1⏎                                                         "
    "    [●] + after text 2⏎                                                         "
    "        5 this is some trailing text⏎                                           "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "###);

    Ok(())
}

#[test]
fn test_quit_dialog_when_commit_message_provided() -> TestResult {
    let mut state = example_contents();
    state.commits = vec![Commit {
        message: Some("hello".to_string()),
    }];

    let changed_message_and_files = TestingScreenshot::default();
    let changed_message_only = TestingScreenshot::default();
    let mut input = TestingInput {
        width: 80,
        height: 24,
        events: Box::new(
            [
                Event::QuitInterrupt,
                changed_message_and_files.event(),
                Event::QuitCancel,
                Event::ToggleAllUniform, // toggle all
                Event::ToggleAllUniform, // toggle none
                Event::QuitInterrupt,
                changed_message_only.event(),
                Event::QuitInterrupt,
            ]
            .into_iter(),
        ),
        commit_messages: [].into_iter().collect(),
    };
    let recorder = Recorder::new(state, &mut input);
    assert_matches!(recorder.run(), Err(RecordError::Cancelled));

    insta::assert_snapshot!(changed_message_and_files, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "                                                                                "
    "[Edit message]  •  hello                                                        "
    "                                                                                "
    "(◐) foo/bar                                                                  (+)"
    "[●] baz                                                                      [+]"
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "  ┌Quit─────────────────────────────────────────────────────────────────────┐   "
    "  │You have changes to 1 message and 2 files. Are you sure you want to quit?│   "
    "  │                                                                         │   "
    "  └─────────────────────────────────────────────────────────[Go Back]─(Quit)┘   "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "###);
    insta::assert_snapshot!(changed_message_only, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "                                                                                "
    "[Edit message]  •  hello                                                        "
    "                                                                                "
    "( ) foo/bar                                                                  (+)"
    "[ ] baz                                                                      [+]"
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "        ┌Quit─────────────────────────────────────────────────────────┐         "
    "        │You have changes to 1 message. Are you sure you want to quit?│         "
    "        │                                                             │         "
    "        └─────────────────────────────────────────────[Go Back]─(Quit)┘         "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "###);

    Ok(())
}

#[test]
fn test_prev_same_kind() -> TestResult {
    let initial = TestingScreenshot::default();
    let to_baz = TestingScreenshot::default();
    let to_baz_section = TestingScreenshot::default();
    let to_bar_section = TestingScreenshot::default();
    let to_baz_lines = TestingScreenshot::default();
    let mut input = TestingInput::new(
        80,
        20,
        [
            Event::ExpandAll,
            initial.event(),
            // Moves the current item from foo/bar to baz
            Event::FocusPrevSameKind,
            to_baz.event(),
            Event::FocusInner,
            to_baz_section.event(),
            Event::FocusPrevSameKind,
            to_bar_section.event(),
            Event::FocusInner,
            Event::FocusPrevSameKind,
            Event::FocusPrevSameKind,
            to_baz_lines.event(),
            Event::QuitAccept,
        ],
    );
    let state = example_contents();
    let recorder = Recorder::new(state, &mut input);
    recorder.run()?;

    insta::assert_snapshot!(initial, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "(◐) foo/bar                                                                  (-)"
    "        ⋮                                                                       "
    "       18 this is some text⏎                                                    "
    "       19 this is some text⏎                                                    "
    "       20 this is some text⏎                                                    "
    "  [◐] Section 1/1                                                            [-]"
    "    [●] - before text 1⏎                                                        "
    "    [●] - before text 2⏎                                                        "
    "    [●] + after text 1⏎                                                         "
    "    [ ] + after text 2⏎                                                         "
    "       23 this is some trailing text⏎                                           "
    "[●] baz                                                                      [-]"
    "        1 Some leading text 1⏎                                                  "
    "        2 Some leading text 2⏎                                                  "
    "  [●] Section 1/1                                                            [-]"
    "    [●] - before text 1⏎                                                        "
    "    [●] - before text 2⏎                                                        "
    "    [●] + after text 1⏎                                                         "
    "    [●] + after text 2⏎                                                         "
    "###);
    insta::assert_snapshot!(to_baz, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "[◐] foo/bar                                                                  [-]"
    "       18 this is some text⏎                                                    "
    "       19 this is some text⏎                                                    "
    "       20 this is some text⏎                                                    "
    "  [◐] Section 1/1                                                            [-]"
    "    [●] - before text 1⏎                                                        "
    "    [●] - before text 2⏎                                                        "
    "    [●] + after text 1⏎                                                         "
    "    [ ] + after text 2⏎                                                         "
    "       23 this is some trailing text⏎                                           "
    "(●) baz                                                                      (-)"
    "        1 Some leading text 1⏎                                                  "
    "        2 Some leading text 2⏎                                                  "
    "  [●] Section 1/1                                                            [-]"
    "    [●] - before text 1⏎                                                        "
    "    [●] - before text 2⏎                                                        "
    "    [●] + after text 1⏎                                                         "
    "    [●] + after text 2⏎                                                         "
    "        5 this is some trailing text⏎                                           "
    "###);
    insta::assert_snapshot!(to_baz_section, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "[◐] foo/bar                                                                  [-]"
    "       18 this is some text⏎                                                    "
    "       19 this is some text⏎                                                    "
    "       20 this is some text⏎                                                    "
    "  [◐] Section 1/1                                                            [-]"
    "    [●] - before text 1⏎                                                        "
    "    [●] - before text 2⏎                                                        "
    "    [●] + after text 1⏎                                                         "
    "    [ ] + after text 2⏎                                                         "
    "       23 this is some trailing text⏎                                           "
    "[●] baz                                                                      [-]"
    "        1 Some leading text 1⏎                                                  "
    "        2 Some leading text 2⏎                                                  "
    "  (●) Section 1/1                                                            (-)"
    "    [●] - before text 1⏎                                                        "
    "    [●] - before text 2⏎                                                        "
    "    [●] + after text 1⏎                                                         "
    "    [●] + after text 2⏎                                                         "
    "        5 this is some trailing text⏎                                           "
    "###);
    insta::assert_snapshot!(to_bar_section, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "[◐] foo/bar                                                                  [-]"
    "       18 this is some text⏎                                                    "
    "       19 this is some text⏎                                                    "
    "       20 this is some text⏎                                                    "
    "  (◐) Section 1/1                                                            (-)"
    "    [●] - before text 1⏎                                                        "
    "    [●] - before text 2⏎                                                        "
    "    [●] + after text 1⏎                                                         "
    "    [ ] + after text 2⏎                                                         "
    "       23 this is some trailing text⏎                                           "
    "[●] baz                                                                      [-]"
    "        1 Some leading text 1⏎                                                  "
    "        2 Some leading text 2⏎                                                  "
    "  [●] Section 1/1                                                            [-]"
    "    [●] - before text 1⏎                                                        "
    "    [●] - before text 2⏎                                                        "
    "    [●] + after text 1⏎                                                         "
    "    [●] + after text 2⏎                                                         "
    "        5 this is some trailing text⏎                                           "
    "###);
    insta::assert_snapshot!(to_baz_lines, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "[◐] foo/bar                                                                  [-]"
    "       18 this is some text⏎                                                    "
    "       19 this is some text⏎                                                    "
    "       20 this is some text⏎                                                    "
    "  [◐] Section 1/1                                                            [-]"
    "    [●] - before text 1⏎                                                        "
    "    [●] - before text 2⏎                                                        "
    "    [●] + after text 1⏎                                                         "
    "    [ ] + after text 2⏎                                                         "
    "       23 this is some trailing text⏎                                           "
    "[●] baz                                                                      [-]"
    "        1 Some leading text 1⏎                                                  "
    "        2 Some leading text 2⏎                                                  "
    "  [●] Section 1/1                                                            [-]"
    "    [●] - before text 1⏎                                                        "
    "    [●] - before text 2⏎                                                        "
    "    (●) + after text 1⏎                                                         "
    "    [●] + after text 2⏎                                                         "
    "        5 this is some trailing text⏎                                           "
    "###);
    Ok(())
}

#[test]
fn test_next_same_kind() -> TestResult {
    let initial = TestingScreenshot::default();
    let to_baz = TestingScreenshot::default();
    let to_baz_section = TestingScreenshot::default();
    let to_bar_section = TestingScreenshot::default();
    let to_bar_lines = TestingScreenshot::default();
    let mut input = TestingInput::new(
        80,
        20,
        [
            Event::ExpandAll,
            initial.event(),
            // Moves the current item from foo/bar to baz
            Event::FocusNextSameKind,
            to_baz.event(),
            Event::FocusInner,
            to_baz_section.event(),
            Event::FocusNextSameKind,
            to_bar_section.event(),
            Event::FocusInner,
            Event::FocusNextSameKind,
            Event::FocusNextSameKind,
            to_bar_lines.event(),
            Event::QuitAccept,
        ],
    );
    let state = example_contents();
    let recorder = Recorder::new(state, &mut input);
    recorder.run()?;

    insta::assert_snapshot!(initial, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "(◐) foo/bar                                                                  (-)"
    "        ⋮                                                                       "
    "       18 this is some text⏎                                                    "
    "       19 this is some text⏎                                                    "
    "       20 this is some text⏎                                                    "
    "  [◐] Section 1/1                                                            [-]"
    "    [●] - before text 1⏎                                                        "
    "    [●] - before text 2⏎                                                        "
    "    [●] + after text 1⏎                                                         "
    "    [ ] + after text 2⏎                                                         "
    "       23 this is some trailing text⏎                                           "
    "[●] baz                                                                      [-]"
    "        1 Some leading text 1⏎                                                  "
    "        2 Some leading text 2⏎                                                  "
    "  [●] Section 1/1                                                            [-]"
    "    [●] - before text 1⏎                                                        "
    "    [●] - before text 2⏎                                                        "
    "    [●] + after text 1⏎                                                         "
    "    [●] + after text 2⏎                                                         "
    "###);
    insta::assert_snapshot!(to_baz, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "[◐] foo/bar                                                                  [-]"
    "       18 this is some text⏎                                                    "
    "       19 this is some text⏎                                                    "
    "       20 this is some text⏎                                                    "
    "  [◐] Section 1/1                                                            [-]"
    "    [●] - before text 1⏎                                                        "
    "    [●] - before text 2⏎                                                        "
    "    [●] + after text 1⏎                                                         "
    "    [ ] + after text 2⏎                                                         "
    "       23 this is some trailing text⏎                                           "
    "(●) baz                                                                      (-)"
    "        1 Some leading text 1⏎                                                  "
    "        2 Some leading text 2⏎                                                  "
    "  [●] Section 1/1                                                            [-]"
    "    [●] - before text 1⏎                                                        "
    "    [●] - before text 2⏎                                                        "
    "    [●] + after text 1⏎                                                         "
    "    [●] + after text 2⏎                                                         "
    "        5 this is some trailing text⏎                                           "
    "###);
    insta::assert_snapshot!(to_baz_section, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "[◐] foo/bar                                                                  [-]"
    "       18 this is some text⏎                                                    "
    "       19 this is some text⏎                                                    "
    "       20 this is some text⏎                                                    "
    "  [◐] Section 1/1                                                            [-]"
    "    [●] - before text 1⏎                                                        "
    "    [●] - before text 2⏎                                                        "
    "    [●] + after text 1⏎                                                         "
    "    [ ] + after text 2⏎                                                         "
    "       23 this is some trailing text⏎                                           "
    "[●] baz                                                                      [-]"
    "        1 Some leading text 1⏎                                                  "
    "        2 Some leading text 2⏎                                                  "
    "  (●) Section 1/1                                                            (-)"
    "    [●] - before text 1⏎                                                        "
    "    [●] - before text 2⏎                                                        "
    "    [●] + after text 1⏎                                                         "
    "    [●] + after text 2⏎                                                         "
    "        5 this is some trailing text⏎                                           "
    "###);
    insta::assert_snapshot!(to_bar_section, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "[◐] foo/bar                                                                  [-]"
    "       18 this is some text⏎                                                    "
    "       19 this is some text⏎                                                    "
    "       20 this is some text⏎                                                    "
    "  (◐) Section 1/1                                                            (-)"
    "    [●] - before text 1⏎                                                        "
    "    [●] - before text 2⏎                                                        "
    "    [●] + after text 1⏎                                                         "
    "    [ ] + after text 2⏎                                                         "
    "       23 this is some trailing text⏎                                           "
    "[●] baz                                                                      [-]"
    "        1 Some leading text 1⏎                                                  "
    "        2 Some leading text 2⏎                                                  "
    "  [●] Section 1/1                                                            [-]"
    "    [●] - before text 1⏎                                                        "
    "    [●] - before text 2⏎                                                        "
    "    [●] + after text 1⏎                                                         "
    "    [●] + after text 2⏎                                                         "
    "        5 this is some trailing text⏎                                           "
    "###);
    insta::assert_snapshot!(to_bar_lines, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "[◐] foo/bar                                                                  [-]"
    "       18 this is some text⏎                                                    "
    "       19 this is some text⏎                                                    "
    "       20 this is some text⏎                                                    "
    "  [◐] Section 1/1                                                            [-]"
    "    [●] - before text 1⏎                                                        "
    "    [●] - before text 2⏎                                                        "
    "    (●) + after text 1⏎                                                         "
    "    [ ] + after text 2⏎                                                         "
    "       23 this is some trailing text⏎                                           "
    "[●] baz                                                                      [-]"
    "        1 Some leading text 1⏎                                                  "
    "        2 Some leading text 2⏎                                                  "
    "  [●] Section 1/1                                                            [-]"
    "    [●] - before text 1⏎                                                        "
    "    [●] - before text 2⏎                                                        "
    "    [●] + after text 1⏎                                                         "
    "    [●] + after text 2⏎                                                         "
    "        5 this is some trailing text⏎                                           "
    "###);
    Ok(())
}

// Test the prev/next same kind keybindings when there is only a single section
// of a given kind.
#[test]
fn test_prev_next_same_kind_single_section() -> TestResult {
    let initial = TestingScreenshot::default();
    let next = TestingScreenshot::default();
    let prev = TestingScreenshot::default();
    let mut input = TestingInput::new(
        80,
        10,
        [
            Event::ExpandAll,
            // Move down to the section so the current selection isn't the
            // first item.
            Event::FocusNext,
            initial.event(),
            // Moves the current item from foo/bar to baz
            Event::FocusNextSameKind,
            next.event(),
            Event::FocusPrevSameKind,
            prev.event(),
            Event::QuitAccept,
        ],
    );
    let mut state = example_contents();
    // Change the example so that there's only a single file.
    state.files = vec![state.files[0].clone()];
    let recorder = Recorder::new(state, &mut input);
    recorder.run()?;
    // Since we start at the foo/bar file section and there are no other
    // sections, the current section never changes.
    insta::assert_snapshot!(initial, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "[◐] foo/bar                                                                  [-]"
    "       18 this is some text⏎                                                    "
    "       19 this is some text⏎                                                    "
    "       20 this is some text⏎                                                    "
    "  (◐) Section 1/1                                                            (-)"
    "    [●] - before text 1⏎                                                        "
    "    [●] - before text 2⏎                                                        "
    "    [●] + after text 1⏎                                                         "
    "    [ ] + after text 2⏎                                                         "
    "###);
    insta::assert_snapshot!(next, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "[◐] foo/bar                                                                  [-]"
    "       18 this is some text⏎                                                    "
    "       19 this is some text⏎                                                    "
    "       20 this is some text⏎                                                    "
    "  (◐) Section 1/1                                                            (-)"
    "    [●] - before text 1⏎                                                        "
    "    [●] - before text 2⏎                                                        "
    "    [●] + after text 1⏎                                                         "
    "    [ ] + after text 2⏎                                                         "
    "###);
    insta::assert_snapshot!(prev, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "[◐] foo/bar                                                                  [-]"
    "       18 this is some text⏎                                                    "
    "       19 this is some text⏎                                                    "
    "       20 this is some text⏎                                                    "
    "  (◐) Section 1/1                                                            (-)"
    "    [●] - before text 1⏎                                                        "
    "    [●] - before text 2⏎                                                        "
    "    [●] + after text 1⏎                                                         "
    "    [ ] + after text 2⏎                                                         "
    "###);
    Ok(())
}

#[cfg(feature = "serde")]
#[test]
fn test_deserialize() -> TestResult {
    let example_json = include_str!("example_contents.json");
    let deserialized: RecordState<'static> = serde_json::from_str(example_json).unwrap();
    assert_eq!(example_contents(), deserialized);
    Ok(())
}

#[test]
fn test_no_files() -> TestResult {
    let state = RecordState {
        is_read_only: false,
        commits: Default::default(),
        files: vec![],
    };
    let initial = TestingScreenshot::default();
    let mut input = TestingInput::new(
        80,
        6,
        [Event::ExpandAll, initial.event(), Event::QuitAccept],
    );
    let recorder = Recorder::new(state, &mut input);
    recorder.run()?;

    insta::assert_snapshot!(initial, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "                                                                                "
    "                    There are no changes to view.                               "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "###);

    Ok(())
}

#[test]
fn test_tabs_in_files() -> TestResult {
    let state = RecordState {
        is_read_only: false,
        commits: Default::default(),
        files: vec![File {
            old_path: None,
            path: Cow::Borrowed(Path::new("foo/bar")),
            file_mode: FileMode::FILE_DEFAULT,
            sections: vec![
                Section::Unchanged {
                    lines: iter::repeat(Cow::Borrowed("\tthis is some indented text\n"))
                        .take(10)
                        .collect(),
                },
                Section::Changed {
                    lines: vec![
                        SectionChangedLine {
                            is_checked: true,
                            change_type: ChangeType::Removed,
                            line: Cow::Borrowed("before text\t1\n"),
                        },
                        SectionChangedLine {
                            is_checked: true,
                            change_type: ChangeType::Added,
                            line: Cow::Borrowed("after text 1\n"),
                        },
                        SectionChangedLine {
                            is_checked: true,
                            change_type: ChangeType::Removed,
                            line: Cow::Borrowed("before text 2\n"),
                        },
                        SectionChangedLine {
                            is_checked: true,
                            change_type: ChangeType::Added,
                            line: Cow::Borrowed("after text\t2\n"),
                        },
                        SectionChangedLine {
                            is_checked: true,
                            change_type: ChangeType::Removed,
                            line: Cow::Borrowed("\tbefore text 3\n"),
                        },
                        SectionChangedLine {
                            is_checked: true,
                            change_type: ChangeType::Added,
                            line: Cow::Borrowed("\tafter text\t3\n"),
                        },
                        SectionChangedLine {
                            is_checked: true,
                            change_type: ChangeType::Removed,
                            line: Cow::Borrowed("\tbefore text\t4\n"),
                        },
                        SectionChangedLine {
                            is_checked: true,
                            change_type: ChangeType::Added,
                            line: Cow::Borrowed("\tafter text 4\n"),
                        },
                        SectionChangedLine {
                            is_checked: true,
                            change_type: ChangeType::Removed,
                            line: Cow::Borrowed("\tbefore text\t5"),
                        },
                        SectionChangedLine {
                            is_checked: true,
                            change_type: ChangeType::Added,
                            line: Cow::Borrowed("\tafter text\t5"),
                        },
                    ],
                },
                Section::Unchanged {
                    lines: vec![Cow::Borrowed("this is some trailing\ttext\n")],
                },
            ],
        }],
    };
    let initial = TestingScreenshot::default();
    let mut input = TestingInput::new(
        80,
        18,
        [Event::ExpandAll, initial.event(), Event::QuitAccept],
    );
    let recorder = Recorder::new(state, &mut input);
    recorder.run()?;

    insta::assert_snapshot!(initial, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "(●) foo/bar                                                                  (-)"
    "        ⋮                                                                       "
    "        8 →   this is some indented text⏎                                       "
    "        9 →   this is some indented text⏎                                       "
    "       10 →   this is some indented text⏎                                       "
    "  [●] Section 1/1                                                            [-]"
    "    [●] - before text→   1⏎                                                     "
    "    [●] + after text 1⏎                                                         "
    "    [●] - before text 2⏎                                                        "
    "    [●] + after text→   2⏎                                                      "
    "    [●] - →   before text 3⏎                                                    "
    "    [●] + →   after text→   3⏎                                                  "
    "    [●] - →   before text→   4⏎                                                 "
    "    [●] + →   after text 4⏎                                                     "
    "    [●] - →   before text→   5                                                  "
    "    [●] + →   after text→   5                                                   "
    "       16 this is some trailing→   text⏎                                        "
    "###);

    Ok(())
}

#[test]
fn test_carriage_return() -> TestResult {
    let state = RecordState {
        is_read_only: false,
        commits: Default::default(),
        files: vec![File {
            old_path: None,
            path: Cow::Borrowed(Path::new("foo")),
            file_mode: FileMode::FILE_DEFAULT,
            sections: vec![Section::Changed {
                lines: vec![
                    SectionChangedLine {
                        is_checked: false,
                        change_type: ChangeType::Removed,
                        line: Cow::Borrowed("before text\n"),
                    },
                    SectionChangedLine {
                        is_checked: false,
                        change_type: ChangeType::Added,
                        line: Cow::Borrowed("before text\r\n"),
                    },
                ],
            }],
        }],
    };

    let initial = TestingScreenshot::default();
    let focus = TestingScreenshot::default();
    let unfocus = TestingScreenshot::default();
    let mut input = TestingInput::new(
        80,
        8,
        [
            Event::ExpandAll,
            initial.event(),
            Event::FocusNext,
            Event::FocusNext,
            focus.event(),
            Event::FocusPrev,
            Event::FocusPrev,
            unfocus.event(),
            Event::QuitAccept,
        ],
    );
    let recorder = Recorder::new(state, &mut input);
    recorder.run()?;

    insta::assert_snapshot!(initial, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "( ) foo                                                                      (-)"
    "  [ ] Section 1/1                                                            [-]"
    "    [ ] - before text⏎                                                          "
    "    [ ] + before text␍⏎                                                         "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "###);

    insta::assert_snapshot!(focus, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "[ ] foo                                                                      [-]"
    "  [ ] Section 1/1                                                            [-]"
    "    ( ) - before text⏎                                                          "
    "    [ ] + before text␍⏎                                                         "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "###);

    insta::assert_snapshot!(unfocus, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "( ) foo                                                                      (-)"
    "  [ ] Section 1/1                                                            [-]"
    "    [ ] - before text⏎                                                          "
    "    [ ] + before text␍⏎                                                         "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "###);

    Ok(())
}

#[test]
fn test_some_control_characters() -> TestResult {
    let state = RecordState {
        is_read_only: false,
        commits: Default::default(),
        files: vec![File {
            old_path: None,
            path: Cow::Borrowed(Path::new("foo")),
            file_mode: FileMode::FILE_DEFAULT,
            sections: vec![Section::Changed {
                lines: vec![SectionChangedLine {
                    is_checked: false,
                    change_type: ChangeType::Added,
                    line: Cow::Borrowed("nul:\0, bel:\x07, esc:\x1b, del:\x7f\n"),
                }],
            }],
        }],
    };

    let initial = TestingScreenshot::default();
    let mut input = TestingInput::new(
        80,
        8,
        [Event::ExpandAll, initial.event(), Event::QuitAccept],
    );
    let recorder = Recorder::new(state, &mut input);
    recorder.run()?;

    insta::assert_snapshot!(initial, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "( ) foo                                                                      (-)"
    "  [ ] Section 1/1                                                            [-]"
    "    [ ] + nul:␀, bel:␇, esc:␛, del:␡⏎                                           "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "###);

    Ok(())
}

#[test]
fn test_non_printing_characters() -> TestResult {
    let state = RecordState {
        is_read_only: false,
        commits: Default::default(),
        files: vec![File {
            old_path: None,
            path: Cow::Borrowed(Path::new("foo")),
            file_mode: FileMode::FILE_DEFAULT,
            sections: vec![Section::Changed {
                lines: vec![SectionChangedLine {
                    is_checked: false,
                    change_type: ChangeType::Added,
                    line: Cow::Borrowed("zwj:\u{200d}, zwnj:\u{200c}"),
                }],
            }],
        }],
    };

    let initial = TestingScreenshot::default();
    let mut input = TestingInput::new(
        80,
        8,
        [Event::ExpandAll, initial.event(), Event::QuitAccept],
    );
    let recorder = Recorder::new(state, &mut input);
    recorder.run()?;

    insta::assert_snapshot!(initial, @r###"
    "[File] [Edit] [Select] [View]                                                   "
    "( ) foo                                                                      (-)"
    "  [ ] Section 1/1                                                            [-]"
    "    [ ] + zwj:�, zwnj:�                                                         "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "###);

    Ok(())
}
