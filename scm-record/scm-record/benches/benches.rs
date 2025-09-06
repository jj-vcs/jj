use std::{borrow::Cow, path::Path};

use criterion::{criterion_group, criterion_main, BatchSize, Criterion};

use scm_record::{
    helpers::TestingInput, ChangeType, Event, File, FileMode, RecordState, Recorder, Section,
    SectionChangedLine,
};

fn bench_record(c: &mut Criterion) {
    c.bench_function("scm_record: toggle line", |b| {
        let before_line = SectionChangedLine {
            line: Cow::Borrowed("foo"),
            is_checked: false,
            change_type: ChangeType::Removed,
        };
        let after_line = SectionChangedLine {
            line: Cow::Borrowed("foo"),
            is_checked: false,
            change_type: ChangeType::Added,
        };
        let record_state = RecordState {
            is_read_only: false,
            commits: Default::default(),
            files: vec![File {
                old_path: None,
                path: Cow::Borrowed(Path::new("foo")),
                file_mode: FileMode::FILE_DEFAULT,
                sections: vec![Section::Changed {
                    lines: [vec![before_line; 1000], vec![after_line; 1000]].concat(),
                }],
            }],
        };
        let mut input = TestingInput::new(
            80,
            24,
            [Event::ToggleItem, Event::ToggleItem, Event::QuitAccept],
        );
        b.iter_batched(
            || record_state.clone(),
            |record_state| {
                let recorder = Recorder::new(record_state, &mut input);
                recorder.run()
            },
            BatchSize::PerIteration,
        )
    });
}

criterion_group!(
    name = benches;
    config = Criterion::default().sample_size(10);
    targets = bench_record,
);
criterion_main!(benches);
