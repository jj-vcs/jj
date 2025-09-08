use std::borrow::Cow;
use std::path::PathBuf;

use scm_record::helpers::make_binary_description;
use scm_record::{ChangeType, File, Section, SectionChangedLine};
use tracing::warn;

use super::{Error, FileContents, FileInfo, Filesystem};

fn make_section_changed_lines(
    contents: &str,
    change_type: ChangeType,
) -> Vec<SectionChangedLine<'static>> {
    contents
        .split_inclusive('\n')
        .map(|line| SectionChangedLine {
            is_checked: false,
            change_type,
            line: Cow::Owned(line.to_owned()),
        })
        .collect()
}

pub fn create_file(
    filesystem: &dyn Filesystem,
    left_path: PathBuf,
    left_display_path: PathBuf,
    right_path: PathBuf,
    right_display_path: PathBuf,
) -> Result<File<'static>, Error> {
    let FileInfo {
        file_mode: left_file_mode,
        contents: left_contents,
    } = filesystem.read_file_info(&left_path)?;
    let FileInfo {
        file_mode: right_file_mode,
        contents: right_contents,
    } = filesystem.read_file_info(&right_path)?;
    let mut sections = Vec::new();

    if left_file_mode != right_file_mode {
        sections.push(Section::FileMode {
            is_checked: false,
            mode: right_file_mode,
        });
    }

    match (left_contents, right_contents) {
        (FileContents::Absent, FileContents::Absent) => {}
        (
            FileContents::Absent,
            FileContents::Text {
                contents,
                hash: _,
                num_bytes: _,
            },
        ) => sections.push(Section::Changed {
            lines: make_section_changed_lines(&contents, ChangeType::Added),
        }),

        (FileContents::Absent, FileContents::Binary { hash, num_bytes }) => {
            sections.push(Section::Binary {
                is_checked: false,
                old_description: None,
                new_description: Some(Cow::Owned(make_binary_description(&hash, num_bytes))),
            })
        }

        (
            FileContents::Text {
                contents,
                hash: _,
                num_bytes: _,
            },
            FileContents::Absent,
        ) => sections.push(Section::Changed {
            lines: make_section_changed_lines(&contents, ChangeType::Removed),
        }),

        (
            FileContents::Text {
                contents: old_contents,
                hash: _,
                num_bytes: _,
            },
            FileContents::Text {
                contents: new_contents,
                hash: _,
                num_bytes: _,
            },
        ) => {
            sections.extend(create_diff(&old_contents, &new_contents));
        }

        (
            FileContents::Text {
                contents: _,
                hash: old_hash,
                num_bytes: old_num_bytes,
            }
            | FileContents::Binary {
                hash: old_hash,
                num_bytes: old_num_bytes,
            },
            FileContents::Text {
                contents: _,
                hash: new_hash,
                num_bytes: new_num_bytes,
            }
            | FileContents::Binary {
                hash: new_hash,
                num_bytes: new_num_bytes,
            },
        ) => sections.push(Section::Binary {
            is_checked: false,
            old_description: Some(Cow::Owned(make_binary_description(
                &old_hash,
                old_num_bytes,
            ))),
            new_description: Some(Cow::Owned(make_binary_description(
                &new_hash,
                new_num_bytes,
            ))),
        }),

        (FileContents::Binary { hash, num_bytes }, FileContents::Absent) => {
            sections.push(Section::Binary {
                is_checked: false,
                old_description: Some(Cow::Owned(make_binary_description(&hash, num_bytes))),
                new_description: None,
            })
        }
    }

    Ok(File {
        old_path: if left_display_path != right_display_path {
            Some(Cow::Owned(left_display_path))
        } else {
            None
        },
        path: Cow::Owned(right_display_path),
        file_mode: left_file_mode,
        sections,
    })
}

pub fn create_merge_file(
    filesystem: &dyn Filesystem,
    base_path: PathBuf,
    left_path: PathBuf,
    right_path: PathBuf,
    output_path: PathBuf,
) -> Result<File<'static>, Error> {
    let FileInfo {
        file_mode: left_file_mode,
        contents: left_contents,
    } = filesystem.read_file_info(&left_path)?;
    let FileInfo {
        file_mode: _,
        contents: right_contents,
    } = filesystem.read_file_info(&right_path)?;
    let FileInfo {
        file_mode: _,
        contents: base_contents,
    } = filesystem.read_file_info(&base_path)?;

    let (base_contents, left_contents, right_contents) =
        match (base_contents, left_contents, right_contents) {
            (FileContents::Absent, _, _) => {
                return Err(Error::MissingMergeFile { path: base_path })
            }
            (_, FileContents::Absent, _) => {
                return Err(Error::MissingMergeFile { path: left_path })
            }
            (_, _, FileContents::Absent) => {
                return Err(Error::MissingMergeFile { path: right_path })
            }
            (FileContents::Binary { .. }, _, _) => {
                return Err(Error::BinaryMergeFile { path: base_path })
            }
            (_, FileContents::Binary { .. }, _) => {
                return Err(Error::BinaryMergeFile { path: left_path })
            }
            (_, _, FileContents::Binary { .. }) => {
                return Err(Error::BinaryMergeFile { path: right_path })
            }
            (
                FileContents::Text {
                    contents: base_contents,
                    hash: _,
                    num_bytes: _,
                },
                FileContents::Text {
                    contents: left_contents,
                    hash: _,
                    num_bytes: _,
                },
                FileContents::Text {
                    contents: right_contents,
                    hash: _,
                    num_bytes: _,
                },
            ) => (base_contents, left_contents, right_contents),
        };

    let sections = create_merge(&base_contents, &left_contents, &right_contents);
    Ok(File {
        old_path: Some(Cow::Owned(base_path)),
        path: Cow::Owned(output_path),
        file_mode: left_file_mode,
        sections,
    })
}

fn create_diff(old_contents: &str, new_contents: &str) -> Vec<Section<'static>> {
    let patch = {
        // Set the context length to the maximum number of lines in either file,
        // because we will handle abbreviating context ourselves.
        let max_lines = old_contents
            .lines()
            .count()
            .max(new_contents.lines().count());
        let mut diff_options = diffy::DiffOptions::new();
        diff_options.set_context_len(max_lines);
        diff_options.create_patch(old_contents, new_contents)
    };

    let mut sections = Vec::new();
    for hunk in patch.hunks() {
        sections.extend(hunk.lines().iter().fold(Vec::new(), |mut acc, line| {
            match line {
                diffy::Line::Context(line) => match acc.last_mut() {
                    Some(Section::Unchanged { lines }) => {
                        lines.push(Cow::Owned((*line).to_owned()));
                    }
                    _ => {
                        acc.push(Section::Unchanged {
                            lines: vec![Cow::Owned((*line).to_owned())],
                        });
                    }
                },
                diffy::Line::Delete(line) => {
                    let line = SectionChangedLine {
                        is_checked: false,
                        change_type: ChangeType::Removed,
                        line: Cow::Owned((*line).to_owned()),
                    };
                    match acc.last_mut() {
                        Some(Section::Changed { lines }) => {
                            lines.push(line);
                        }
                        _ => {
                            acc.push(Section::Changed { lines: vec![line] });
                        }
                    }
                }
                diffy::Line::Insert(line) => {
                    let line = SectionChangedLine {
                        is_checked: false,
                        change_type: ChangeType::Added,
                        line: Cow::Owned((*line).to_owned()),
                    };
                    match acc.last_mut() {
                        Some(Section::Changed { lines }) => {
                            lines.push(line);
                        }
                        _ => {
                            acc.push(Section::Changed { lines: vec![line] });
                        }
                    }
                }
            }
            acc
        }));
    }
    sections
}

fn make_conflict_markers(base: &str, left: &str, right: &str) -> (String, String, String, String) {
    let all = [base, left, right].concat();
    let left_char = "<";
    let base_start_char = "|";
    let base_end_char = "=";
    let right_char = ">";
    let mut len = 7;
    loop {
        let left_marker = left_char.repeat(len);
        let base_start_marker = base_start_char.repeat(len);
        let base_end_marker = base_end_char.repeat(len);
        let right_marker = right_char.repeat(len);
        if !all.contains(&left_marker)
            && !all.contains(&base_start_marker)
            && !all.contains(&base_end_marker)
            && !all.contains(&right_marker)
        {
            return (
                left_marker,
                base_start_marker,
                base_end_marker,
                right_marker,
            );
        }
        len += 1;
    }
}

fn create_merge(
    base_contents: &str,
    left_contents: &str,
    right_contents: &str,
) -> Vec<Section<'static>> {
    let (left_marker, base_start_marker, base_end_marker, right_marker) =
        make_conflict_markers(base_contents, left_contents, right_contents);

    let mut merge_options = diffy::MergeOptions::new();
    merge_options.set_conflict_marker_length(right_marker.len());
    merge_options.set_conflict_style(diffy::ConflictStyle::Diff3);
    let merge = merge_options.merge(base_contents, left_contents, right_contents);
    let conflicted_text = match merge {
        Ok(_) => return Default::default(),
        Err(conflicted_text) => conflicted_text,
    };

    enum MarkerType {
        Left,
        BaseStart,
        BaseEnd,
        Right,
    }
    #[derive(Debug)]
    enum State<'a> {
        Empty,
        Unchanged {
            lines: Vec<Cow<'a, str>>,
        },
        Left {
            left_lines: Vec<Cow<'a, str>>,
        },
        Base {
            left_lines: Vec<Cow<'a, str>>,
            base_lines: Vec<Cow<'a, str>>,
        },
        Right {
            left_lines: Vec<Cow<'a, str>>,
            base_lines: Vec<Cow<'a, str>>,
            right_lines: Vec<Cow<'a, str>>,
        },
    }

    let mut sections = vec![];
    let mut state = State::Empty;
    for line in conflicted_text.split_inclusive('\n') {
        let marker_type = if line.starts_with(&left_marker) {
            Some(MarkerType::Left)
        } else if line.starts_with(&base_start_marker) {
            Some(MarkerType::BaseStart)
        } else if line.starts_with(&base_end_marker) {
            Some(MarkerType::BaseEnd)
        } else if line.starts_with(&right_marker) {
            Some(MarkerType::Right)
        } else {
            None
        };

        let line = Cow::Owned(line.to_owned());
        let (new_state, new_section) = match (state, marker_type) {
            (State::Empty, Some(MarkerType::Left)) => {
                let new_state = State::Left {
                    left_lines: Default::default(),
                };
                (new_state, None)
            }
            (State::Empty, _) => {
                let new_state = State::Unchanged { lines: vec![line] };
                (new_state, None)
            }

            (State::Unchanged { lines }, Some(MarkerType::Left)) => {
                let new_state = State::Left {
                    left_lines: Default::default(),
                };
                let new_section = Section::Unchanged { lines };
                (new_state, Some(new_section))
            }
            (State::Unchanged { mut lines }, _) => {
                lines.push(line);
                let new_state = State::Unchanged { lines };
                (new_state, None)
            }

            (State::Left { left_lines }, Some(MarkerType::BaseStart)) => {
                let new_state = State::Base {
                    left_lines,
                    base_lines: Default::default(),
                };
                (new_state, None)
            }
            (State::Left { mut left_lines }, _) => {
                left_lines.push(line);
                let new_state = State::Left { left_lines };
                (new_state, None)
            }

            (
                State::Base {
                    left_lines,
                    base_lines,
                },
                Some(MarkerType::BaseEnd),
            ) => {
                let new_state = State::Right {
                    left_lines,
                    base_lines,
                    right_lines: Default::default(),
                };
                (new_state, None)
            }
            (
                State::Base {
                    left_lines,
                    mut base_lines,
                },
                _,
            ) => {
                base_lines.push(line);
                let new_state = State::Base {
                    left_lines,
                    base_lines,
                };
                (new_state, None)
            }

            (
                State::Right {
                    left_lines,
                    base_lines,
                    right_lines,
                },
                Some(MarkerType::Right),
            ) => {
                let new_state = State::Empty;
                let new_section = Section::Changed {
                    lines: left_lines
                        .into_iter()
                        .map(|line| (line, ChangeType::Added))
                        .chain(
                            base_lines
                                .into_iter()
                                .map(|line| (line, ChangeType::Removed)),
                        )
                        .chain(
                            right_lines
                                .into_iter()
                                .map(|line| (line, ChangeType::Added)),
                        )
                        .map(|(line, change_type)| SectionChangedLine {
                            is_checked: false,
                            change_type,
                            line,
                        })
                        .collect(),
                };
                (new_state, Some(new_section))
            }
            (
                State::Right {
                    left_lines,
                    base_lines,
                    mut right_lines,
                },
                _,
            ) => {
                right_lines.push(line);
                let new_state = State::Right {
                    left_lines,
                    base_lines,
                    right_lines,
                };
                (new_state, None)
            }
        };

        state = new_state;
        if let Some(new_section) = new_section {
            sections.push(new_section);
        }
    }

    match state {
        State::Empty => {}
        State::Unchanged { lines } => {
            sections.push(Section::Unchanged { lines });
        }
        state @ (State::Left { .. } | State::Base { .. } | State::Right { .. }) => {
            warn!(?state, "Diff section not terminated");
        }
    }

    sections
}
