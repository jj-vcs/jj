#![warn(clippy::all, clippy::as_conversions)]
#![allow(clippy::too_many_arguments)]

use std::borrow::Cow;
use std::path::Path;

use scm_record::{
    helpers::CrosstermInput, ChangeType, File, FileMode, RecordError, RecordState, Recorder,
    Section, SectionChangedLine, SelectedChanges, SelectedContents,
};

fn main() {
    let files = vec![
        File {
            old_path: None,
            path: Cow::Borrowed(Path::new("foo/bar")),
            file_mode: FileMode::FILE_DEFAULT,
            sections: vec![
                Section::Unchanged {
                    lines: std::iter::repeat(Cow::Borrowed("this is some text\n"))
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
                    lines: vec![Cow::Borrowed("this is some trailing text")],
                },
            ],
        },
    ];
    let record_state = RecordState {
        is_read_only: false,
        commits: Default::default(),
        files,
    };
    let mut input = CrosstermInput;
    let recorder = Recorder::new(record_state, &mut input);
    let result = recorder.run();
    match result {
        Ok(result) => {
            let RecordState {
                is_read_only: _,
                commits: _,
                files,
            } = result;
            for file in files {
                println!("--- Path {:?} final lines: ---", file.path);
                let (selected, _unselected) = file.get_selected_contents();

                let SelectedChanges {
                    contents,
                    file_mode,
                } = selected;

                if file_mode == FileMode::Absent {
                    println!("<absent>");
                } else {
                    print!(
                        "{}",
                        match contents {
                            SelectedContents::Binary {
                                old_description: _,
                                new_description: None,
                            } => "<binary>\n".to_string(),
                            SelectedContents::Binary {
                                old_description: _,
                                new_description: Some(description),
                            } => format!("<binary description={description}>\n"),
                            SelectedContents::Text { contents } => contents.clone(),
                            SelectedContents::Unchanged => "<unchanged\n>".to_string(),
                        }
                    );
                }
            }
        }
        Err(RecordError::Cancelled) => println!("Cancelled!\n"),
        Err(err) => {
            println!("Error: {err}");
        }
    }
}
