#![warn(clippy::all, clippy::as_conversions)]
#![allow(clippy::too_many_arguments)]

use std::path::Path;

use scm_record::{
    helpers::CrosstermInput, FileMode, RecordError, RecordState, Recorder, SelectedChanges,
    SelectedContents,
};

#[cfg(feature = "serde")]
fn load_state(path: impl AsRef<Path>) -> RecordState<'static> {
    let json_file = std::fs::File::open(path).expect("opening JSON file");
    serde_json::from_reader(json_file).expect("deserializing state")
}

#[cfg(not(feature = "serde"))]
fn load_state(_path: impl AsRef<Path>) -> RecordState<'static> {
    panic!("load_json example requires `serde` feature")
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let json_filename = args.get(1).expect("expected JSON dump as first argument");
    let record_state: RecordState = load_state(json_filename);

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
                            SelectedContents::Unchanged => "<unchanged\n>".to_string(),
                            SelectedContents::Binary {
                                old_description: _,
                                new_description: None,
                            } => "<binary>\n".to_string(),
                            SelectedContents::Binary {
                                old_description: _,
                                new_description: Some(description),
                            } => format!("<binary description={description}>\n"),
                            SelectedContents::Text { contents } => contents.clone(),
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
