// Copyright 2023 The Jujutsu Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// https://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use itertools::Itertools as _;
use jj_lib::backend::ChangeId;
use jj_lib::backend::CommitId;
use jj_lib::backend::MillisSinceEpoch;
use jj_lib::backend::Signature;
use jj_lib::backend::Timestamp;
use jj_lib::config::ConfigLayer;
use jj_lib::config::ConfigSource;
use jj_lib::id_prefix::IdPrefixContext;
use jj_lib::id_prefix::IdPrefixIndex;
use jj_lib::index::ResolvedChangeTargets;
use jj_lib::object_id::HexPrefix;
use jj_lib::object_id::ObjectId as _;
use jj_lib::object_id::PrefixResolution::AmbiguousMatch;
use jj_lib::object_id::PrefixResolution::NoMatch;
use jj_lib::object_id::PrefixResolution::SingleMatch;
use jj_lib::op_store::RefTarget;
use jj_lib::repo::MutableRepo;
use jj_lib::repo::Repo as _;
use jj_lib::revset::RevsetExpression;
use jj_lib::settings::UserSettings;
use testutils::TestRepo;
use testutils::TestRepoBackend;

fn stable_settings() -> UserSettings {
    let mut config = testutils::base_user_config();
    let mut layer = ConfigLayer::empty(ConfigSource::User);
    layer
        .set_value("debug.commit-timestamp", "2001-02-03T04:05:06+07:00")
        .unwrap();
    config.add_layer(layer);
    UserSettings::from_config(config).unwrap()
}

#[test]
fn test_id_prefix() {
    let test_repo = TestRepo::init_with_backend(TestRepoBackend::Git);
    let repo = &test_repo.repo;
    let root_commit_id = repo.store().root_commit_id();
    let root_change_id = repo.store().root_change_id();

    let mut tx = repo.start_transaction();
    let mut create_commit = |parent_id: &CommitId| {
        let signature = Signature {
            name: "Some One".to_string(),
            email: "some.one@example.com".to_string(),
            timestamp: Timestamp {
                timestamp: MillisSinceEpoch(0),
                tz_offset: 0,
            },
        };
        tx.repo_mut()
            .new_commit(vec![parent_id.clone()], repo.store().empty_merged_tree())
            .set_author(signature.clone())
            .set_committer(signature)
            .write()
            .unwrap()
    };
    let mut commits = vec![create_commit(root_commit_id)];
    for _ in 0..25 {
        commits.push(create_commit(commits.last().unwrap().id()));
    }
    let repo = tx.commit("test").unwrap();

    // Print the commit IDs and change IDs for reference
    let commit_prefixes = commits
        .iter()
        .enumerate()
        .map(|(i, commit)| format!("{} {}", &commit.id().hex()[..3], i))
        .sorted()
        .join("\n");
    insta::assert_snapshot!(commit_prefixes, @r"
    0c8 9
    18f 7
    19a 10
    37a 13
    3b4 21
    3c0 1
    4ee 16
    51f 4
    56e 14
    711 17
    761 3
    7b1 11
    7c6 24
    7f4 8
    846 23
    8d7 25
    960 15
    a30 12
    b51 19
    b97 22
    b9d 5
    bb4 2
    c3a 18
    c47 0
    d3c 6
    d54 20
    ");
    let change_prefixes = commits
        .iter()
        .enumerate()
        .map(|(i, commit)| format!("{} {}", &commit.change_id().hex()[..3], i))
        .sorted()
        .join("\n");
    insta::assert_snapshot!(change_prefixes, @r"
    026 9
    030 13
    1b5 6
    26b 3
    26c 8
    271 10
    439 2
    44a 17
    4e9 16
    5b2 23
    6c2 19
    781 0
    79f 14
    7d5 24
    86b 20
    871 7
    896 5
    9e4 18
    a2c 1
    a63 22
    b19 11
    b93 4
    bf5 21
    c24 15
    d64 12
    fee 25
    ");

    let prefix = |x| HexPrefix::try_from_hex(x).unwrap();
    let shortest_commit_prefix_len = |index: &IdPrefixIndex, commit_id| {
        index
            .shortest_commit_prefix_len(repo.as_ref(), commit_id)
            .unwrap()
    };
    let resolve_commit_prefix = |index: &IdPrefixIndex, prefix: HexPrefix| {
        index.resolve_commit_prefix(repo.as_ref(), &prefix).unwrap()
    };
    let shortest_change_prefix_len = |index: &IdPrefixIndex, change_id| {
        index
            .shortest_change_prefix_len(repo.as_ref(), change_id)
            .unwrap()
    };
    let resolve_change_prefix = |index: &IdPrefixIndex, prefix: HexPrefix| {
        index
            .resolve_change_prefix(repo.as_ref(), &prefix)
            .unwrap()
            .filter_map(ResolvedChangeTargets::into_visible)
    };

    // Without a disambiguation revset
    // ---------------------------------------------------------------------------------------------
    let context = IdPrefixContext::default();
    let index = context.populate(repo.as_ref()).unwrap();

    assert_eq!(shortest_commit_prefix_len(&index, commits[7].id()), 2);
    assert_eq!(shortest_commit_prefix_len(&index, commits[16].id()), 1);
    assert_eq!(resolve_commit_prefix(&index, prefix("1")), AmbiguousMatch);
    assert_eq!(
        resolve_commit_prefix(&index, prefix("18")),
        SingleMatch(commits[7].id().clone())
    );
    assert_eq!(resolve_commit_prefix(&index, prefix("10")), NoMatch);
    assert_eq!(resolve_commit_prefix(&index, prefix("180")), NoMatch);
    assert_eq!(
        shortest_change_prefix_len(&index, commits[2].change_id()),
        2
    );
    assert_eq!(
        shortest_change_prefix_len(&index, commits[6].change_id()),
        1
    );
    assert_eq!(resolve_change_prefix(&index, prefix("4")), AmbiguousMatch);
    assert_eq!(
        resolve_change_prefix(&index, prefix("43")),
        SingleMatch(vec![commits[2].id().clone()])
    );
    assert_eq!(resolve_change_prefix(&index, prefix("40")), NoMatch);
    assert_eq!(resolve_change_prefix(&index, prefix("430")), NoMatch);

    // Disambiguate within a revset
    // ---------------------------------------------------------------------------------------------
    let expression =
        RevsetExpression::commits(vec![commits[7].id().clone(), commits[2].id().clone()]);
    let context = context.disambiguate_within(expression);
    let index = context.populate(repo.as_ref()).unwrap();
    // The prefix is now shorter
    assert_eq!(shortest_commit_prefix_len(&index, commits[7].id()), 1);
    // Shorter prefix within the set can be used
    assert_eq!(
        resolve_commit_prefix(&index, prefix("1")),
        SingleMatch(commits[7].id().clone())
    );
    // Can still resolve commits outside the set
    assert_eq!(
        resolve_commit_prefix(&index, prefix("19")),
        SingleMatch(commits[10].id().clone())
    );
    assert_eq!(
        shortest_change_prefix_len(&index, commits[2].change_id()),
        1
    );
    assert_eq!(
        resolve_change_prefix(&index, prefix("4")),
        SingleMatch(vec![commits[2].id().clone()])
    );

    // Single commit in revset. Length 0 is unambiguous, but we pretend 1 digit is
    // needed.
    // ---------------------------------------------------------------------------------------------
    let expression = RevsetExpression::commit(root_commit_id.clone());
    let context = context.disambiguate_within(expression);
    let index = context.populate(repo.as_ref()).unwrap();
    assert_eq!(shortest_commit_prefix_len(&index, root_commit_id), 1);
    assert_eq!(resolve_commit_prefix(&index, prefix("")), AmbiguousMatch);
    assert_eq!(
        resolve_commit_prefix(&index, prefix("0")),
        SingleMatch(root_commit_id.clone())
    );
    assert_eq!(shortest_change_prefix_len(&index, root_change_id), 1);
    assert_eq!(resolve_change_prefix(&index, prefix("")), AmbiguousMatch);
    assert_eq!(
        resolve_change_prefix(&index, prefix("0")),
        SingleMatch(vec![root_commit_id.clone()])
    );

    // Disambiguate within revset that fails to evaluate
    // ---------------------------------------------------------------------------------------------
    let expression = RevsetExpression::symbol("nonexistent".to_string());
    let context = context.disambiguate_within(expression);
    assert!(context.populate(repo.as_ref()).is_err());
}

#[test]
fn test_id_prefix_divergent() {
    let test_repo = TestRepo::init_with_backend(TestRepoBackend::Git);
    let repo = &test_repo.repo;
    let root_commit_id = repo.store().root_commit_id();

    let mut tx = repo.start_transaction();
    let mut create_commit_with_change_id =
        |parent_id: &CommitId, description: &str, change_id: ChangeId| {
            let signature = Signature {
                name: "Some One".to_string(),
                email: "some.one@example.com".to_string(),
                timestamp: Timestamp {
                    timestamp: MillisSinceEpoch(0),
                    tz_offset: 0,
                },
            };
            tx.repo_mut()
                .new_commit(vec![parent_id.clone()], repo.store().empty_merged_tree())
                .set_description(description)
                .set_author(signature.clone())
                .set_committer(signature)
                .set_change_id(change_id)
                .write()
                .unwrap()
        };

    let first_change_id = ChangeId::from_hex("a5333333333333333333333333333333");
    let second_change_id = ChangeId::from_hex("a5000000000000000000000000000000");

    let first_commit = create_commit_with_change_id(root_commit_id, "first", first_change_id);
    let second_commit =
        create_commit_with_change_id(first_commit.id(), "second", second_change_id.clone());
    let third_commit_divergent_with_second =
        create_commit_with_change_id(first_commit.id(), "third", second_change_id);
    let commits = [
        first_commit.clone(),
        second_commit.clone(),
        third_commit_divergent_with_second.clone(),
    ];
    let repo = tx.commit("test").unwrap();

    // Print the commit IDs and change IDs for reference
    let change_prefixes = commits
        .iter()
        .enumerate()
        .map(|(i, commit)| format!("{} {}", &commit.change_id().hex()[..4], i))
        .join("\n");
    insta::assert_snapshot!(change_prefixes, @r"
    a533 0
    a500 1
    a500 2
    ");
    let commit_prefixes = commits
        .iter()
        .enumerate()
        .map(|(i, commit)| format!("{} {}", &commit.id().hex()[..4], i))
        .join("\n");
    insta::assert_snapshot!(commit_prefixes, @r"
    e2b9 0
    f8d1 1
    c596 2
    ");

    let prefix = |x| HexPrefix::try_from_hex(x).unwrap();
    let shortest_change_prefix_len = |index: &IdPrefixIndex, change_id| {
        index
            .shortest_change_prefix_len(repo.as_ref(), change_id)
            .unwrap()
    };
    let resolve_change_prefix = |index: &IdPrefixIndex, prefix: HexPrefix| {
        index
            .resolve_change_prefix(repo.as_ref(), &prefix)
            .unwrap()
            .filter_map(ResolvedChangeTargets::into_visible)
    };

    // Without a disambiguation revset
    // --------------------------------
    let context = IdPrefixContext::default();
    let index = context.populate(repo.as_ref()).unwrap();
    assert_eq!(
        shortest_change_prefix_len(&index, commits[0].change_id()),
        3
    );
    assert_eq!(
        shortest_change_prefix_len(&index, commits[1].change_id()),
        3
    );
    assert_eq!(
        shortest_change_prefix_len(&index, commits[2].change_id()),
        3
    );
    assert_eq!(resolve_change_prefix(&index, prefix("a5")), AmbiguousMatch);
    assert_eq!(
        resolve_change_prefix(&index, prefix("a53")),
        SingleMatch(vec![first_commit.id().clone()])
    );
    assert_eq!(
        resolve_change_prefix(&index, prefix("a50")),
        SingleMatch(vec![
            third_commit_divergent_with_second.id().clone(),
            second_commit.id().clone(),
        ])
    );

    // Now, disambiguate within the revset containing only the second commit
    // ----------------------------------------------------------------------
    let expression = RevsetExpression::commits(vec![second_commit.id().clone()]);
    let context = context.disambiguate_within(expression);
    let index = context.populate(repo.as_ref()).unwrap();
    // The prefix is now shorter
    assert_eq!(
        shortest_change_prefix_len(&index, second_commit.change_id()),
        1
    );
    // This tests two issues, both important:
    // - We find both commits with the same change id, even though
    // `third_commit_divergent_with_second` is not in the short prefix set
    // (#2476).
    // - The short prefix set still works: we do *not* find the first commit and the
    //   match is not ambiguous, even though the first commit's change id would also
    //   match the prefix.
    assert_eq!(
        resolve_change_prefix(&index, prefix("a")),
        SingleMatch(vec![
            third_commit_divergent_with_second.id().clone(),
            second_commit.id().clone(),
        ])
    );

    // We can still resolve commits outside the set
    assert_eq!(
        resolve_change_prefix(&index, prefix("a53")),
        SingleMatch(vec![first_commit.id().clone()])
    );
    assert_eq!(
        shortest_change_prefix_len(&index, first_commit.change_id()),
        3
    );
}

#[test]
fn test_id_prefix_hidden() {
    let test_repo = TestRepo::init_with_backend(TestRepoBackend::Git);
    let repo = &test_repo.repo;
    let root_commit_id = repo.store().root_commit_id();

    let mut tx = repo.start_transaction();
    let mut commits = vec![];
    for i in 0..10 {
        let signature = Signature {
            name: "Some One".to_string(),
            email: "some.one@example.com".to_string(),
            timestamp: Timestamp {
                timestamp: MillisSinceEpoch(i),
                tz_offset: 0,
            },
        };
        let commit = tx
            .repo_mut()
            .new_commit(
                vec![root_commit_id.clone()],
                repo.store().empty_merged_tree(),
            )
            .set_author(signature.clone())
            .set_committer(signature)
            .write()
            .unwrap();
        commits.push(commit);
    }

    // Print the commit IDs and change IDs for reference
    let commit_prefixes = commits
        .iter()
        .enumerate()
        .map(|(i, commit)| format!("{} {}", &commit.id().hex()[..3], i))
        .sorted()
        .join("\n");
    insta::assert_snapshot!(commit_prefixes, @r"
    3ae 6
    64c 5
    84e 2
    906 8
    912 7
    9d1 3
    a6b 1
    c47 0
    d9b 4
    f5f 9
    ");
    let change_prefixes = commits
        .iter()
        .enumerate()
        .map(|(i, commit)| format!("{} {}", &commit.change_id().hex()[..3], i))
        .sorted()
        .join("\n");
    insta::assert_snapshot!(change_prefixes, @r"
    026 9
    1b5 6
    26b 3
    26c 8
    439 2
    781 0
    871 7
    896 5
    a2c 1
    b93 4
    ");

    let hidden_commit = &commits[8];
    tx.repo_mut().record_abandoned_commit(hidden_commit);
    tx.repo_mut().rebase_descendants().unwrap();
    let repo = tx.commit("test").unwrap();

    let prefix = |x: &str| HexPrefix::try_from_hex(x).unwrap();
    let shortest_commit_prefix_len = |index: &IdPrefixIndex, commit_id| {
        index
            .shortest_commit_prefix_len(repo.as_ref(), commit_id)
            .unwrap()
    };
    let resolve_commit_prefix = |index: &IdPrefixIndex, prefix: HexPrefix| {
        index.resolve_commit_prefix(repo.as_ref(), &prefix).unwrap()
    };
    let shortest_change_prefix_len = |index: &IdPrefixIndex, change_id| {
        index
            .shortest_change_prefix_len(repo.as_ref(), change_id)
            .unwrap()
    };
    let resolve_change_prefix = |index: &IdPrefixIndex, prefix: HexPrefix| {
        index
            .resolve_change_prefix(repo.as_ref(), &prefix)
            .unwrap()
            .filter_map(ResolvedChangeTargets::into_visible)
    };

    // Without a disambiguation revset
    // --------------------------------
    let context = IdPrefixContext::default();
    let index = context.populate(repo.as_ref()).unwrap();
    assert_eq!(shortest_commit_prefix_len(&index, hidden_commit.id()), 2);
    assert_eq!(
        shortest_change_prefix_len(&index, hidden_commit.change_id()),
        3
    );
    assert_eq!(
        resolve_commit_prefix(&index, prefix(&hidden_commit.id().hex()[..1])),
        AmbiguousMatch
    );
    assert_eq!(
        resolve_commit_prefix(&index, prefix(&hidden_commit.id().hex()[..2])),
        SingleMatch(hidden_commit.id().clone())
    );
    assert_eq!(
        resolve_change_prefix(&index, prefix(&hidden_commit.change_id().hex()[..2])),
        AmbiguousMatch
    );
    assert_eq!(
        resolve_change_prefix(&index, prefix(&hidden_commit.change_id().hex()[..3])),
        NoMatch
    );

    // Disambiguate within hidden
    // --------------------------
    let expression = RevsetExpression::commit(hidden_commit.id().clone());
    let context = context.disambiguate_within(expression);
    let index = context.populate(repo.as_ref()).unwrap();
    assert_eq!(shortest_commit_prefix_len(&index, hidden_commit.id()), 1);
    assert_eq!(
        shortest_change_prefix_len(&index, hidden_commit.change_id()),
        1
    );
    // Short commit id can be resolved even if it's hidden.
    assert_eq!(
        resolve_commit_prefix(&index, prefix(&hidden_commit.id().hex()[..1])),
        SingleMatch(hidden_commit.id().clone())
    );
    // OTOH, hidden change id should never be found. The resolution might be
    // ambiguous if hidden commits were excluded from the disambiguation set.
    // In that case, shortest_change_prefix_len() shouldn't be 1.
    assert_eq!(
        resolve_change_prefix(&index, prefix(&hidden_commit.change_id().hex()[..1])),
        NoMatch
    );
    assert_eq!(
        resolve_change_prefix(&index, prefix(&hidden_commit.change_id().hex()[..2])),
        NoMatch
    );
}

#[test]
fn test_id_prefix_shadowed_by_ref() {
    let settings = stable_settings();
    let test_repo = TestRepo::init_with_settings(&settings);
    let repo = &test_repo.repo;
    let root_commit_id = repo.store().root_commit_id();

    let mut tx = repo.start_transaction();
    let commit = tx
        .repo_mut()
        .new_commit(
            vec![root_commit_id.clone()],
            repo.store().empty_merged_tree(),
        )
        .write()
        .unwrap();

    let commit_id_sym = commit.id().to_string();
    let change_id_sym = commit.change_id().to_string();
    insta::assert_snapshot!(commit_id_sym, @"b06a01f026da65ac5821");
    insta::assert_snapshot!(change_id_sym, @"sryyqqkqmuumyrlruupspprvnulvovzm");

    let context = IdPrefixContext::default();
    let index = context.populate(tx.repo()).unwrap();
    let shortest_commit_prefix_len =
        |repo: &MutableRepo, commit_id| index.shortest_commit_prefix_len(repo, commit_id).unwrap();
    let shortest_change_prefix_len =
        |repo: &MutableRepo, change_id| index.shortest_change_prefix_len(repo, change_id).unwrap();

    assert_eq!(shortest_commit_prefix_len(tx.repo(), commit.id()), 1);
    assert_eq!(shortest_change_prefix_len(tx.repo(), commit.change_id()), 1);

    // Longer symbol doesn't count
    let dummy_target = RefTarget::normal(root_commit_id.clone());
    tx.repo_mut()
        .set_local_tag_target(commit_id_sym[..2].as_ref(), dummy_target.clone());
    tx.repo_mut()
        .set_local_tag_target(change_id_sym[..2].as_ref(), dummy_target.clone());
    assert_eq!(shortest_commit_prefix_len(tx.repo(), commit.id()), 1);
    assert_eq!(shortest_change_prefix_len(tx.repo(), commit.change_id()), 1);

    // 1-char conflict with bookmark, 2-char with tag
    tx.repo_mut()
        .set_local_bookmark_target(commit_id_sym[..1].as_ref(), dummy_target.clone());
    tx.repo_mut()
        .set_local_bookmark_target(change_id_sym[..1].as_ref(), dummy_target.clone());
    assert_eq!(shortest_commit_prefix_len(tx.repo(), commit.id()), 3);
    assert_eq!(shortest_change_prefix_len(tx.repo(), commit.change_id()), 3);

    // Many-char conflicts
    for n in 3..commit_id_sym.len() {
        tx.repo_mut()
            .set_local_tag_target(commit_id_sym[..n].as_ref(), dummy_target.clone());
    }
    for n in 3..change_id_sym.len() {
        tx.repo_mut()
            .set_local_tag_target(change_id_sym[..n].as_ref(), dummy_target.clone());
    }
    assert_eq!(
        shortest_commit_prefix_len(tx.repo(), commit.id()),
        commit_id_sym.len()
    );
    assert_eq!(
        shortest_change_prefix_len(tx.repo(), commit.change_id()),
        change_id_sym.len()
    );

    // Full-char conflicts
    tx.repo_mut()
        .set_local_tag_target(commit_id_sym.as_ref(), dummy_target.clone());
    tx.repo_mut()
        .set_local_tag_target(change_id_sym.as_ref(), dummy_target.clone());
    assert_eq!(
        shortest_commit_prefix_len(tx.repo(), commit.id()),
        commit_id_sym.len()
    );
    assert_eq!(
        shortest_change_prefix_len(tx.repo(), commit.change_id()),
        change_id_sym.len()
    );
}
