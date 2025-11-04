// Copyright 2025 The Jujutsu Authors
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

//! Test that optimized revset evaluates to the same result as the original
//! expression.
//!
//! Use `PROPTEST_CASES=10000` to increase the number of test cases to run.
//! The default is `256`, which might be too small to catch edge-case bugs.
//! <https://proptest-rs.github.io/proptest/proptest/tutorial/config.html>

use std::sync::Arc;

use itertools::Itertools as _;
use jj_lib::backend::CommitId;
use jj_lib::commit::Commit;
use jj_lib::config::ConfigLayer;
use jj_lib::config::ConfigSource;
use jj_lib::repo::MutableRepo;
use jj_lib::repo::Repo;
use jj_lib::revset::ResolvedRevsetExpression;
use jj_lib::revset::RevsetExpression;
use jj_lib::revset::RevsetFilterPredicate;
use jj_lib::rewrite::RebaseOptions;
use jj_lib::rewrite::RebasedCommit;
use jj_lib::settings::UserSettings;
use proptest::prelude::*;
use testutils::TestRepo;

fn stable_settings() -> UserSettings {
    let mut config = testutils::base_user_config();
    let mut layer = ConfigLayer::empty(ConfigSource::User);
    layer
        .set_value("debug.commit-timestamp", "2001-02-03T04:05:06+07:00")
        .unwrap();
    config.add_layer(layer);
    UserSettings::from_config(config).unwrap()
}

fn write_new_commit<'a>(
    repo: &mut MutableRepo,
    desc: &str,
    parents: impl IntoIterator<Item = &'a Commit>,
) -> Commit {
    let parents = parents.into_iter().map(|c| c.id().clone()).collect();
    let tree = repo.store().empty_merged_tree();
    repo.new_commit(parents, tree)
        .set_description(desc)
        .write()
        .unwrap()
}

fn rebase_descendants(repo: &mut MutableRepo) -> Vec<Commit> {
    let mut commits = Vec::new();
    repo.rebase_descendants_with_options(&RebaseOptions::default(), |_, rebased| match rebased {
        RebasedCommit::Rewritten(commit) => commits.push(commit),
        RebasedCommit::Abandoned { .. } => {}
    })
    .unwrap();
    commits
}

/// Strategy to generate arbitrary revset expressions.
fn arb_expression(
    known_commits: Vec<CommitId>,
    visible_heads: Vec<Vec<CommitId>>,
) -> impl Strategy<Value = Arc<ResolvedRevsetExpression>> {
    // https://proptest-rs.github.io/proptest/proptest/tutorial/recursive.html
    let max_commits = known_commits.len();
    let leaf_expr = prop_oneof![
        Just(RevsetExpression::none()),
        Just(RevsetExpression::all()),
        Just(RevsetExpression::visible_heads()),
        Just(RevsetExpression::root()),
        proptest::sample::subsequence(known_commits, 1..=5.min(max_commits))
            .prop_map(RevsetExpression::commits),
        // Use merges() as a filter that isn't constant. Since we don't have an
        // optimization rule that rewrites filter predicates, we wouldn't have
        // to add various filter predicates.
        Just(RevsetExpression::filter(
            RevsetFilterPredicate::ParentCount(2..u32::MAX)
        )),
    ];
    leaf_expr.prop_recursive(
        10,  // depth
        100, // total nodes
        2,   // unary or binary
        move |expr| {
            // This table includes redundant expressions (e.g. parents() and
            // ancestors()) if they are common, which will probably make them be
            // more weighted?
            prop_oneof![
                // Ancestors
                expr.clone().prop_map(|x| x.parents()),
                expr.clone().prop_map(|x| x.ancestors()),
                (expr.clone(), 0..5_u64).prop_map(|(x, d)| x.ancestors_range(0..d)),
                // Descendants
                expr.clone().prop_map(|x| x.children()),
                expr.clone().prop_map(|x| x.descendants()),
                (expr.clone(), 0..5_u64).prop_map(|(x, d)| x.descendants_range(0..d)),
                // First ancestors
                expr.clone().prop_map(|x| x.first_ancestors_at(1)),
                expr.clone().prop_map(|x| x.first_ancestors()),
                (expr.clone(), 0..5_u64).prop_map(|(x, d)| x.first_ancestors_range(0..d)),
                // Range
                (expr.clone(), expr.clone()).prop_map(|(x, y)| x.range(&y)),
                // DagRange
                (expr.clone(), expr.clone()).prop_map(|(x, y)| x.dag_range_to(&y)),
                expr.clone().prop_map(|x| x.connected()),
                // Reachable
                (expr.clone(), expr.clone()).prop_map(|(x, y)| x.reachable(&y)),
                // Heads
                expr.clone().prop_map(|x| x.heads()),
                // Roots
                expr.clone().prop_map(|x| x.roots()),
                // ForkPoint
                expr.clone().prop_map(|x| x.fork_point()),
                // Latest
                (expr.clone(), 0..5_usize).prop_map(|(x, n)| x.latest(n)),
                // AtOperation (or WithinVisibility)
                (
                    expr.clone(),
                    proptest::sample::select(visible_heads.clone())
                )
                    .prop_map(|(candidates, visible_heads)| Arc::new(
                        RevsetExpression::WithinVisibility {
                            candidates,
                            visible_heads
                        }
                    )),
                // Coalesce (in binary form)
                [expr.clone(), expr.clone()].prop_map(|xs| RevsetExpression::coalesce(&xs)),
                // General set operations
                expr.clone().prop_map(|x| x.negated()),
                (expr.clone(), expr.clone()).prop_map(|(x, y)| x.union(&y)),
                (expr.clone(), expr.clone()).prop_map(|(x, y)| x.intersection(&y)),
                (expr.clone(), expr.clone()).prop_map(|(x, y)| x.minus(&y)),
            ]
        },
    )
}

fn verify_optimized(
    repo: &dyn Repo,
    expression: &Arc<ResolvedRevsetExpression>,
) -> Result<(), TestCaseError> {
    let optimized_revset = expression.clone().evaluate(repo).unwrap();
    let unoptimized_revset = expression.clone().evaluate_unoptimized(repo).unwrap();
    let optimized_ids: Vec<_> = optimized_revset.iter().try_collect().unwrap();
    let unoptimized_ids: Vec<_> = unoptimized_revset.iter().try_collect().unwrap();
    prop_assert_eq!(optimized_ids, unoptimized_ids);
    Ok(())
}

#[test]
fn test_mostly_linear() {
    let settings = stable_settings();
    let test_repo = TestRepo::init_with_settings(&settings);
    let repo = test_repo.repo;

    // 8 9
    // 6 7
    // |/|
    // 4 5
    // 3 |
    // 1 2
    // |/
    // 0
    let mut tx = repo.start_transaction();
    let commit0 = repo.store().root_commit();
    let commit1 = write_new_commit(tx.repo_mut(), "1", [&commit0]);
    let commit2 = write_new_commit(tx.repo_mut(), "2", [&commit0]);
    let commit3 = write_new_commit(tx.repo_mut(), "3", [&commit1]);
    let commit4 = write_new_commit(tx.repo_mut(), "4", [&commit3]);
    let commit5 = write_new_commit(tx.repo_mut(), "5", [&commit2]);
    let commit6 = write_new_commit(tx.repo_mut(), "6", [&commit4]);
    let commit7 = write_new_commit(tx.repo_mut(), "7", [&commit4, &commit5]);
    let commit8 = write_new_commit(tx.repo_mut(), "8", [&commit6]);
    let commit9 = write_new_commit(tx.repo_mut(), "9", [&commit7]);
    let commits = vec![
        commit0, commit1, commit2, commit3, commit4, commit5, commit6, commit7, commit8, commit9,
    ];
    let repo = tx.commit("a").unwrap();

    // Commit ids for reference
    insta::assert_snapshot!(
        commits.iter().map(|c| format!("{:<2} {}\n", c.description(), c.id())).join(""), @r"
       00000000000000000000
    1  0481b93947ec320582da
    2  cf00f20ba8d03cfe27d7
    3  db1fb816b776ac1257a1
    4  3fa336059698229e1869
    5  f1d81483ce728dac9c5c
    6  c0620144c8381147c3eb
    7  f1355db0abd492eb4df4
    8  2aa8feb562b8426f485f
    9  fee08f51862cdd69739d
    ");

    let commit_ids = commits.iter().map(|c| c.id().clone()).collect_vec();
    let visible_heads = vec![
        vec![commit_ids[0].clone()],
        vec![commit_ids[8].clone(), commit_ids[9].clone()],
    ];

    proptest!(|(expression in arb_expression(commit_ids, visible_heads))| {
        verify_optimized(repo.as_ref(), &expression)?;
    });
}

#[test]
fn test_weird_merges() {
    let settings = stable_settings();
    let test_repo = TestRepo::init_with_settings(&settings);
    let repo = test_repo.repo;

    //     8
    //    /|\
    // 4 5 6 7
    // |X| |/
    // 1 2 3
    //  \|/
    //   0
    let mut tx = repo.start_transaction();
    let commit0 = repo.store().root_commit();
    let commit1 = write_new_commit(tx.repo_mut(), "1", [&commit0]);
    let commit2 = write_new_commit(tx.repo_mut(), "2", [&commit0]);
    let commit3 = write_new_commit(tx.repo_mut(), "3", [&commit0]);
    let commit4 = write_new_commit(tx.repo_mut(), "4", [&commit1, &commit2]);
    let commit5 = write_new_commit(tx.repo_mut(), "5", [&commit1, &commit2]);
    let commit6 = write_new_commit(tx.repo_mut(), "6", [&commit3]);
    let commit7 = write_new_commit(tx.repo_mut(), "7", [&commit3]);
    let commit8 = write_new_commit(tx.repo_mut(), "8", [&commit5, &commit6, &commit7]);
    let commits = vec![
        commit0, commit1, commit2, commit3, commit4, commit5, commit6, commit7, commit8,
    ];
    let repo = tx.commit("a").unwrap();

    // Commit ids for reference
    insta::assert_snapshot!(
        commits.iter().map(|c| format!("{:<2} {}\n", c.description(), c.id())).join(""), @r"
       00000000000000000000
    1  0481b93947ec320582da
    2  cf00f20ba8d03cfe27d7
    3  eae7b745114ccd1e7c2b
    4  8ff5f4ecfd6e51f7fb46
    5  0701d1a6ff427cc5d1c1
    6  8b2aa399c528813d0bfc
    7  abd7903b1690685bb9c8
    8  0f916c2bef3fe0aa2f54
    ");

    let commit_ids = commits.iter().map(|c| c.id().clone()).collect_vec();
    let visible_heads = vec![
        vec![commit_ids[0].clone()],
        vec![commit_ids[4].clone(), commit_ids[8].clone()],
    ];

    proptest!(|(expression in arb_expression(commit_ids, visible_heads))| {
        verify_optimized(repo.as_ref(), &expression)?;
    });
}

#[test]
fn test_feature_branches() {
    let settings = stable_settings();
    let test_repo = TestRepo::init_with_settings(&settings);
    let repo = test_repo.repo;

    // 9
    // |\
    // 8 \
    // |\ \
    // | 7 |
    // |/  |
    // 6   |
    // |\  5
    // | | 4
    // | | 3
    // | 2 |
    // | 1 |
    // | |/
    // |/
    // 0

    // Fetch branch 2 and 5
    let mut tx = repo.start_transaction();
    let commit0 = repo.store().root_commit();
    let commit1 = write_new_commit(tx.repo_mut(), "1", [&commit0]);
    let commit2 = write_new_commit(tx.repo_mut(), "2", [&commit1]);
    let commit3 = write_new_commit(tx.repo_mut(), "3", [&commit0]);
    let commit4 = write_new_commit(tx.repo_mut(), "4", [&commit3]);
    let commit5 = write_new_commit(tx.repo_mut(), "5", [&commit4]);
    let repo = tx.commit("a").unwrap();

    // Merge branch 2
    let mut tx = repo.start_transaction();
    let commit6 = write_new_commit(tx.repo_mut(), "6", [&commit0, &commit2]);
    let repo = tx.commit("a").unwrap();

    // Fetch merged branch 7
    let mut tx = repo.start_transaction();
    let commit7 = write_new_commit(tx.repo_mut(), "7", [&commit6]);
    let commit8 = write_new_commit(tx.repo_mut(), "8", [&commit6, &commit7]);
    let repo = tx.commit("a").unwrap();

    // Merge branch 5
    let mut tx = repo.start_transaction();
    let commit9 = write_new_commit(tx.repo_mut(), "9", [&commit8, &commit5]);
    let commits = vec![
        commit0, commit1, commit2, commit3, commit4, commit5, commit6, commit7, commit8, commit9,
    ];
    let repo = tx.commit("a").unwrap();

    // Commit ids for reference
    insta::assert_snapshot!(
        commits.iter().map(|c| format!("{:<2} {}\n", c.description(), c.id())).join(""), @r"
       00000000000000000000
    1  0481b93947ec320582da
    2  dfc04cc2cdd8ddb7b55b
    3  eae7b745114ccd1e7c2b
    4  8e272dcf1dfef181cb22
    5  62bb52a8eb37e1dffcb1
    6  900d6e697d7e53fe31c5
    7  74bcf8cf11c54a565c0c
    8  01c9367ecaaa7c0bfc47
    9  60b2edccd2b633d4b164
    ");

    let commit_ids = commits.iter().map(|c| c.id().clone()).collect_vec();
    let visible_heads = vec![
        vec![commit_ids[0].clone()],
        vec![commit_ids[2].clone(), commit_ids[5].clone()],
        vec![commit_ids[5].clone(), commit_ids[6].clone()],
        vec![commit_ids[5].clone(), commit_ids[8].clone()],
        vec![commit_ids[9].clone()],
    ];

    proptest!(|(expression in arb_expression(commit_ids, visible_heads))| {
        verify_optimized(repo.as_ref(), &expression)?;
    });
}

#[test]
fn test_rewritten() {
    let settings = stable_settings();
    let test_repo = TestRepo::init_with_settings(&settings);
    let repo = test_repo.repo;

    // 5
    // |\
    // 4 | 3
    // | |/
    // 1 2
    // |/
    // 0
    let mut tx = repo.start_transaction();
    let commit0 = repo.store().root_commit();
    let commit1 = write_new_commit(tx.repo_mut(), "1", [&commit0]);
    let commit2 = write_new_commit(tx.repo_mut(), "2", [&commit0]);
    let commit3 = write_new_commit(tx.repo_mut(), "3", [&commit2]);
    let commit4 = write_new_commit(tx.repo_mut(), "4", [&commit1]);
    let commit5 = write_new_commit(tx.repo_mut(), "5", [&commit4, &commit2]);
    let mut commits = vec![commit0, commit1, commit2, commit3, commit4, commit5];
    let repo = tx.commit("a").unwrap();

    // Rewrite 2, rebase 3 and 5
    let mut tx = repo.start_transaction();
    let commit2b = tx
        .repo_mut()
        .rewrite_commit(&commits[2])
        .set_description("2b")
        .write()
        .unwrap();
    commits.push(commit2b);
    commits.extend(rebase_descendants(tx.repo_mut()));
    let repo = tx.commit("b").unwrap();

    // Abandon 4, rebase 5
    let mut tx = repo.start_transaction();
    tx.repo_mut().record_abandoned_commit(&commits[4]);
    commits.extend(rebase_descendants(tx.repo_mut()));
    let repo = tx.commit("c").unwrap();

    // Commit ids for reference
    insta::assert_snapshot!(
        commits.iter().map(|c| format!("{:<2} {}\n", c.description(), c.id())).join(""), @r"
       00000000000000000000
    1  0481b93947ec320582da
    2  cf00f20ba8d03cfe27d7
    3  0ae48179bdee2dc5cbee
    4  01e8f78ec985350a98f5
    5  2586f14733ac1c4f2b89
    2b 4dc5572169ee6230a7fc
    3  9efc19868da6032ea4f1
    5  ea4b6ce59436a2ccfa67
    5  31f5e38f8d68c839c6f6
    ");

    let commit_ids = commits.iter().map(|c| c.id().clone()).collect_vec();
    let visible_heads = vec![
        vec![commit_ids[0].clone()],
        vec![commit_ids[3].clone(), commit_ids[5].clone()],
        vec![commit_ids[7].clone(), commit_ids[8].clone()],
        vec![commit_ids[7].clone(), commit_ids[9].clone()],
    ];

    proptest!(|(expression in arb_expression(commit_ids, visible_heads))| {
        verify_optimized(repo.as_ref(), &expression)?;
    });
}
