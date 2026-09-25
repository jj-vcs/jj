// Copyright 2026 The Jujutsu Authors
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

use std::collections::HashMap;

use assert_matches::assert_matches;
use jj_lib::backend::CommitId;
use jj_lib::commit::Commit;
use jj_lib::git;
use jj_lib::git::GitImportOptions;
use jj_lib::git::GitImportRefUpdate;
use jj_lib::git::GitImportStats;
use jj_lib::git_sync::GitSyncError;
use jj_lib::git_sync::GitSyncUnsupportedReason;
use jj_lib::git_sync::sync_imported_refs;
use jj_lib::object_id::ObjectId as _;
use jj_lib::op_store::RefTarget;
use jj_lib::op_store::RemoteRef;
use jj_lib::op_store::RemoteRefState;
use jj_lib::ref_name::RemoteRefSymbol;
use jj_lib::ref_name::WorkspaceName;
use jj_lib::repo::MutableRepo;
use jj_lib::repo::Repo as _;
use jj_lib::rewrite::EmptyBehavior;
use jj_lib::rewrite::RebaseOptions;
use jj_lib::rewrite::RebasedCommit;
use maplit::hashset;
use pollster::FutureExt as _;
use testutils::CommitBuilderExt as _;
use testutils::TestRepo;
use testutils::TestRepoBackend;
use testutils::TestResult;
use testutils::create_random_commit;
use testutils::write_random_commit;
use testutils::write_random_commit_with_parents;

fn imported_update(name: &str, remote: &str, old: &Commit, new: &Commit) -> GitImportRefUpdate {
    GitImportRefUpdate::new(
        RemoteRefSymbol {
            name: name.as_ref(),
            remote: remote.as_ref(),
        }
        .to_owned(),
        RemoteRef {
            target: RefTarget::normal(old.id().clone()),
            state: RemoteRefState::Tracked,
        },
        RefTarget::normal(new.id().clone()),
    )
}

fn apply_import_updates(
    mut_repo: &mut MutableRepo,
    mut updates: Vec<GitImportRefUpdate>,
) -> GitImportStats {
    updates.sort_unstable_by(|left, right| left.symbol.cmp(&right.symbol));
    for update in &updates {
        mut_repo.set_remote_bookmark(
            update.symbol.as_ref(),
            RemoteRef {
                target: update.new_target.clone(),
                state: RemoteRefState::Tracked,
            },
        );
    }
    GitImportStats {
        changed_remote_bookmarks: updates,
        ..Default::default()
    }
}

fn expect_rewritten<'a>(results: &'a HashMap<CommitId, RebasedCommit>, old: &Commit) -> &'a Commit {
    match results.get(old.id()) {
        Some(RebasedCommit::Rewritten(new)) => new,
        Some(RebasedCommit::Abandoned { parent_id }) => {
            panic!(
                "expected commit {} to be rewritten, but it was abandoned onto {parent_id}",
                old.id()
            );
        }
        None => panic!("expected a result for commit {}", old.id()),
    }
}

fn set_git_ref(git_repo: &gix::Repository, name: &str, target: &Commit) -> TestResult {
    let target = gix::ObjectId::from_bytes_or_panic(target.id().as_bytes());
    git_repo.reference(name, target, gix::refs::transaction::PreviousValue::Any, "")?;
    Ok(())
}

#[test]
fn test_sync_real_imported_fast_forward() -> TestResult {
    // main@origin is first imported at old_main and marked tracked. The feature
    // commit is selected before main@origin advances to new_main. The second import
    // returns that fast-forward to sync_imported_refs().
    //
    // Before import and sync:
    //
    // old_main ─┬─> new_main (new main@origin target)
    //           └─> feature (selected)
    //
    // After sync:
    //
    // old_main ──> new_main (main@origin) ──> feature'
    let test_repo = TestRepo::init_with_backend(TestRepoBackend::Git);
    let git_repo = git::get_git_repo(test_repo.repo.store())?;
    let import_options = GitImportOptions {
        abandon_unreachable_commits: true,
        record_synthetic_predecessors: true,
        remote_auto_track_bookmarks: HashMap::new(),
    };

    let mut tx = test_repo.repo.start_transaction();
    let old_main = write_random_commit(tx.repo_mut());
    set_git_ref(&git_repo, "refs/remotes/origin/main", &old_main)?;
    git::import_refs(tx.repo_mut(), &import_options).block_on()?;
    tx.repo_mut().set_remote_bookmark(
        RemoteRefSymbol {
            name: "main".as_ref(),
            remote: "origin".as_ref(),
        },
        RemoteRef {
            target: RefTarget::normal(old_main.id().clone()),
            state: RemoteRefState::Tracked,
        },
    );
    let new_main = write_random_commit_with_parents(tx.repo_mut(), &[&old_main]);
    let feature = write_random_commit_with_parents(tx.repo_mut(), &[&old_main]);
    let selected_feature_id = feature.id().clone();
    let repo = tx.commit("setup").block_on()?;

    set_git_ref(&git_repo, "refs/remotes/origin/main", &new_main)?;
    let mut tx = repo.start_transaction();
    let stats = git::import_refs(tx.repo_mut(), &import_options).block_on()?;
    assert!(!tx.repo().has_rewrites());
    let result = sync_imported_refs(
        tx.repo_mut(),
        &stats,
        vec![selected_feature_id],
        &RebaseOptions::default(),
    )
    .block_on()?;

    let new_feature = expect_rewritten(&result, &feature);
    assert_eq!(new_feature.parent_ids(), vec![new_main.id().clone()]);
    assert!(!tx.repo().has_rewrites());
    Ok(())
}

#[test]
fn test_sync_exact_selected_graph_and_references() -> TestResult {
    // Imported: main@origin moved from old_main to new_main.
    // Selected: feature_auth, feature_docs, release_merge.
    //
    // Before sync:
    //
    // old_main ─┬─> feature_auth ───────────────┐
    //           ├─> feature_docs ───────────────┼─> release_merge (@, release)
    //           └─> new_main (main@origin) ─────┘
    //
    // After sync:
    //
    // old_main ──> new_main (main@origin)
    //              ├─> feature_auth' ──────────┐
    //              ├─> feature_docs' ──────────┼─> release_merge' (@, release)
    //              └───────────────────────────┘
    let test_repo = TestRepo::init();
    let mut tx = test_repo.repo.start_transaction();
    let old_main = write_random_commit(tx.repo_mut());
    let new_main = write_random_commit_with_parents(tx.repo_mut(), &[&old_main]);
    let feature_auth = write_random_commit_with_parents(tx.repo_mut(), &[&old_main]);
    let feature_docs = write_random_commit_with_parents(tx.repo_mut(), &[&old_main]);
    let release_merge =
        write_random_commit_with_parents(tx.repo_mut(), &[&feature_auth, &feature_docs, &new_main]);
    let symbol = RemoteRefSymbol {
        name: "main".as_ref(),
        remote: "origin".as_ref(),
    };
    tx.repo_mut().set_local_bookmark_target(
        "release".as_ref(),
        RefTarget::normal(release_merge.id().clone()),
    );
    tx.repo_mut().set_wc_commit(
        WorkspaceName::DEFAULT.to_owned(),
        release_merge.id().clone(),
    )?;
    let stats = apply_import_updates(
        tx.repo_mut(),
        vec![imported_update("main", "origin", &old_main, &new_main)],
    );
    let options = RebaseOptions {
        // Rewrite roots may use merge simplification, but release_merge is a
        // structural descendant and must retain its explicit new_main parent.
        simplify_ancestor_merge: true,
        ..Default::default()
    };
    let result = sync_imported_refs(
        tx.repo_mut(),
        &stats,
        vec![
            feature_auth.id().clone(),
            feature_docs.id().clone(),
            release_merge.id().clone(),
        ],
        &options,
    )
    .block_on()?;

    assert_eq!(result.len(), 3);
    let new_feature_auth = expect_rewritten(&result, &feature_auth);
    let new_feature_docs = expect_rewritten(&result, &feature_docs);
    let new_release_merge = expect_rewritten(&result, &release_merge);
    assert_eq!(new_feature_auth.parent_ids(), vec![new_main.id().clone()]);
    assert_eq!(new_feature_docs.parent_ids(), vec![new_main.id().clone()]);
    assert_eq!(
        new_release_merge.parent_ids(),
        vec![
            new_feature_auth.id().clone(),
            new_feature_docs.id().clone(),
            new_main.id().clone(),
        ]
    );
    assert_eq!(
        tx.repo().get_local_bookmark("release".as_ref()),
        RefTarget::normal(new_release_merge.id().clone())
    );
    assert_eq!(
        tx.repo().view().get_wc_commit_id(WorkspaceName::DEFAULT),
        Some(new_release_merge.id())
    );
    assert_eq!(
        tx.repo().get_remote_bookmark(symbol).target,
        RefTarget::normal(new_main.id().clone())
    );
    assert_eq!(
        *tx.repo().view().heads(),
        hashset! { new_release_merge.id().clone() }
    );
    assert!(!tx.repo().has_rewrites());
    Ok(())
}

#[test]
fn test_sync_empty_wc_root_with_hidden_selected_descendant() -> TestResult {
    // Imported: main@origin moved from old_main to new_main.
    // Selected: release_base and feature_hidden, which is not a repository head.
    //
    // Before sync:
    //
    // old_main ─┬─> release_base (@, empty) ──> feature_hidden (hidden)
    //           └─> new_main (main@origin)
    //
    // After sync (release_base abandoned):
    //
    // old_main ──> new_main (main@origin)
    //              ├─> feature_hidden'
    //              └─> new_wc (@, empty)
    let test_repo = TestRepo::init();
    let mut tx = test_repo.repo.start_transaction();
    let old_main = write_random_commit(tx.repo_mut());
    let new_main = write_random_commit_with_parents(tx.repo_mut(), &[&old_main]);
    let release_base = create_random_commit(tx.repo_mut())
        .set_parents(vec![old_main.id().clone()])
        .set_tree(old_main.tree())
        .write_unwrap();
    let feature_hidden = write_random_commit_with_parents(tx.repo_mut(), &[&release_base]);
    tx.repo_mut().remove_head(feature_hidden.id());
    tx.repo_mut()
        .set_wc_commit(WorkspaceName::DEFAULT.to_owned(), release_base.id().clone())?;
    let stats = apply_import_updates(
        tx.repo_mut(),
        vec![imported_update("main", "origin", &old_main, &new_main)],
    );
    let result = sync_imported_refs(
        tx.repo_mut(),
        &stats,
        vec![release_base.id().clone(), feature_hidden.id().clone()],
        &RebaseOptions {
            empty: EmptyBehavior::AbandonAllEmpty,
            ..Default::default()
        },
    )
    .block_on()?;

    assert_matches!(
        &result[release_base.id()],
        RebasedCommit::Abandoned { parent_id } if parent_id == new_main.id()
    );
    let new_feature_hidden = expect_rewritten(&result, &feature_hidden);
    assert_eq!(new_feature_hidden.parent_ids(), vec![new_main.id().clone()]);
    let wc_id = tx
        .repo()
        .view()
        .get_wc_commit_id(WorkspaceName::DEFAULT)
        .expect("working copy should be recreated after abandoning its selected commit");
    let wc = tx.repo().store().get_commit(wc_id)?;
    assert_eq!(wc.parent_ids(), vec![new_main.id().clone()]);
    assert_eq!(
        *tx.repo().view().heads(),
        hashset! { new_feature_hidden.id().clone(), wc.id().clone() }
    );
    assert!(!tx.repo().has_rewrites());
    Ok(())
}

#[test]
fn test_sync_keeps_empty_structural_descendant() -> TestResult {
    // Imported: main@origin moved from old_main to new_main.
    // Selected: feature_root and empty_descendant.
    //
    // Before sync:
    //
    // old_main ─┬─> new_main (main@origin)
    //           └─> feature_root ──> empty_descendant (empty)
    //
    // After sync:
    //
    // old_main ──> new_main (main@origin)
    //              └─> feature_root' ──> empty_descendant' (empty)
    //
    // Caller empty-commit policy applies to the rewrite root, while structural
    // descendants are preserved one-for-one.
    let test_repo = TestRepo::init();
    let mut tx = test_repo.repo.start_transaction();
    let old_main = write_random_commit(tx.repo_mut());
    let new_main = write_random_commit_with_parents(tx.repo_mut(), &[&old_main]);
    let feature_root = write_random_commit_with_parents(tx.repo_mut(), &[&old_main]);
    let empty_descendant = create_random_commit(tx.repo_mut())
        .set_parents(vec![feature_root.id().clone()])
        .set_tree(feature_root.tree())
        .write_unwrap();
    let stats = apply_import_updates(
        tx.repo_mut(),
        vec![imported_update("main", "origin", &old_main, &new_main)],
    );

    let result = sync_imported_refs(
        tx.repo_mut(),
        &stats,
        vec![feature_root.id().clone(), empty_descendant.id().clone()],
        &RebaseOptions {
            empty: EmptyBehavior::AbandonAllEmpty,
            ..Default::default()
        },
    )
    .block_on()?;

    assert_eq!(result.len(), 2);
    let new_feature_root = expect_rewritten(&result, &feature_root);
    let new_empty_descendant = expect_rewritten(&result, &empty_descendant);
    assert_eq!(new_feature_root.parent_ids(), vec![new_main.id().clone()]);
    assert_eq!(
        new_empty_descendant.parent_ids(),
        vec![new_feature_root.id().clone()]
    );
    Ok(())
}

#[test]
fn test_sync_ignores_selected_commits_on_imported_lineage() -> TestResult {
    // Imported: main@origin moved from old_main to new_main through midpoint.
    // Selected: midpoint, deployed, and feature_root.
    //
    // Before sync:
    //
    // old_main ──> midpoint ─┬─> new_main (main@origin) ──> deployed
    //                      └─> feature_root
    //
    // After sync: only the side-branch feature is rewritten. Selected commits
    // on or beyond the imported lineage are not part of the rewrite plan.
    //
    // old_main ──> midpoint ──> new_main (main@origin) ─┬─> deployed
    //                                               └─> feature_root'
    let test_repo = TestRepo::init();
    let mut tx = test_repo.repo.start_transaction();
    let old_main = write_random_commit(tx.repo_mut());
    let midpoint = write_random_commit_with_parents(tx.repo_mut(), &[&old_main]);
    let new_main = write_random_commit_with_parents(tx.repo_mut(), &[&midpoint]);
    let deployed = write_random_commit_with_parents(tx.repo_mut(), &[&new_main]);
    let feature_root = write_random_commit_with_parents(tx.repo_mut(), &[&midpoint]);
    let stats = apply_import_updates(
        tx.repo_mut(),
        vec![imported_update("main", "origin", &old_main, &new_main)],
    );

    let result = sync_imported_refs(
        tx.repo_mut(),
        &stats,
        vec![
            midpoint.id().clone(),
            deployed.id().clone(),
            feature_root.id().clone(),
        ],
        &RebaseOptions::default(),
    )
    .block_on()?;

    assert_eq!(result.len(), 1);
    assert!(!result.contains_key(midpoint.id()));
    assert!(!result.contains_key(deployed.id()));
    let new_feature_root = expect_rewritten(&result, &feature_root);
    assert_eq!(new_feature_root.parent_ids(), vec![new_main.id().clone()]);
    Ok(())
}

#[test]
fn test_sync_rejects_conflicting_rewrite_roles_across_moves() -> TestResult {
    // Imported: main@origin moved old_main to new_main through old_release;
    // release@origin moved old_release to new_release through feature_root.
    // Selected: feature_root and empty_mixed_role.
    //
    // Before sync:
    //
    // old_main ──> old_release ─┬─> new_main (main@origin)
    //                           └─> feature_root ─┬─> new_release (release@origin)
    //                                              └─> empty_mixed_role
    //
    // The selected child is structural for the main update and a rewrite root
    // for the release update. Applying the release parent override would
    // detach it from the rewritten selected parent produced by the main
    // update. The planner has no branch-ownership input, so it rejects the
    // combined plan instead of choosing one update implicitly.
    let test_repo = TestRepo::init();
    let mut tx = test_repo.repo.start_transaction();
    let old_main = write_random_commit(tx.repo_mut());
    let old_release = write_random_commit_with_parents(tx.repo_mut(), &[&old_main]);
    let new_main = write_random_commit_with_parents(tx.repo_mut(), &[&old_release]);
    let feature_root = write_random_commit_with_parents(tx.repo_mut(), &[&old_release]);
    let new_release = create_random_commit(tx.repo_mut())
        .set_parents(vec![feature_root.id().clone()])
        .set_tree(feature_root.tree())
        .write_unwrap();
    let empty_mixed_role = create_random_commit(tx.repo_mut())
        .set_parents(vec![feature_root.id().clone()])
        .set_tree(feature_root.tree())
        .write_unwrap();
    let stats = apply_import_updates(
        tx.repo_mut(),
        vec![
            imported_update("main", "origin", &old_main, &new_main),
            imported_update("release", "origin", &old_release, &new_release),
        ],
    );

    let error = sync_imported_refs(
        tx.repo_mut(),
        &stats,
        vec![feature_root.id().clone(), empty_mixed_role.id().clone()],
        &RebaseOptions {
            empty: EmptyBehavior::AbandonAllEmpty,
            ..Default::default()
        },
    )
    .block_on()
    .unwrap_err();
    assert_matches!(
        error,
        GitSyncError::ConflictingRewriteRoles { commit_ids }
            if commit_ids == vec![empty_mixed_role.id().clone()]
    );
    Ok(())
}

#[test]
fn test_sync_rejects_merge_with_conflicting_rewrite_roles() -> TestResult {
    // Imported: main@origin moved old to new; release@origin moved new to newer.
    // Selected: local and merge.
    //
    // old ─┬─> new ──> newer (release@origin)
    //      │    └────────────┐
    //      └─> local ────────┴─> merge
    //
    // merge is structural for the main update because it follows local, but
    // it is a rewrite root for the release update through new. Reject rather
    // than deciding whether to treat it as a root or a descendant.
    let test_repo = TestRepo::init();
    let mut tx = test_repo.repo.start_transaction();
    let old = write_random_commit(tx.repo_mut());
    let new = write_random_commit_with_parents(tx.repo_mut(), &[&old]);
    let newer = write_random_commit_with_parents(tx.repo_mut(), &[&new]);
    let local = write_random_commit_with_parents(tx.repo_mut(), &[&old]);
    let merge = write_random_commit_with_parents(tx.repo_mut(), &[&new, &local]);
    let stats = apply_import_updates(
        tx.repo_mut(),
        vec![
            imported_update("main", "origin", &old, &new),
            imported_update("release", "origin", &new, &newer),
        ],
    );

    let error = sync_imported_refs(
        tx.repo_mut(),
        &stats,
        vec![local.id().clone(), merge.id().clone()],
        &RebaseOptions::default(),
    )
    .block_on()
    .unwrap_err();
    assert_matches!(
        error,
        GitSyncError::ConflictingRewriteRoles { commit_ids }
            if commit_ids == vec![merge.id().clone()]
    );
    Ok(())
}

#[test]
fn test_sync_rejects_replacing_selected_parent_in_rewrite_plan() -> TestResult {
    // Imported: main@origin moved old_a to new_a; release@origin moved r_a to
    // new_b. Selected: parent and child.
    //
    // old_a ─┬─> r_a ──> parent ──> new_b (release@origin)
    //        └─> q_a
    // new_a([r_a, q_a]) (main@origin)
    // child([parent, q_a])
    //
    // Both moves make child a root. The release move would explicitly replace
    // its selected parent, so applying the root overrides would detach child'
    // from parent'.
    let test_repo = TestRepo::init();
    let mut tx = test_repo.repo.start_transaction();
    let old_a = write_random_commit(tx.repo_mut());
    let r_a = write_random_commit_with_parents(tx.repo_mut(), &[&old_a]);
    let q_a = write_random_commit_with_parents(tx.repo_mut(), &[&old_a]);
    let parent = write_random_commit_with_parents(tx.repo_mut(), &[&r_a]);
    let new_a = write_random_commit_with_parents(tx.repo_mut(), &[&r_a, &q_a]);
    let new_b = write_random_commit_with_parents(tx.repo_mut(), &[&parent]);
    let child = write_random_commit_with_parents(tx.repo_mut(), &[&parent, &q_a]);
    let stats = apply_import_updates(
        tx.repo_mut(),
        vec![
            imported_update("main", "origin", &old_a, &new_a),
            imported_update("release", "origin", &r_a, &new_b),
        ],
    );

    let error = sync_imported_refs(
        tx.repo_mut(),
        &stats,
        vec![parent.id().clone(), child.id().clone()],
        &RebaseOptions::default(),
    )
    .block_on()
    .unwrap_err();
    assert_matches!(
        error,
        GitSyncError::OverlappingRewrite {
            commit_id,
            parent_id,
            destination,
        } if commit_id == child.id().clone()
            && parent_id == parent.id().clone()
            && destination == new_b.id().clone()
    );
    Ok(())
}

#[test]
fn test_sync_selection_can_keep_unselected_merge_parent() -> TestResult {
    // Imported: main@origin moved from old_main to new_main through release_cut.
    // The first attempt selects feature_payments, feature_api, and release_merge.
    //
    // Before sync:
    //
    // old_main ──> release_cut
    //              ├─> new_main (main@origin)
    //              └─> feature_payments
    //                   ├─> feature_api ─────────┐
    //                   └─> release_bridge ──────┴─> release_merge
    //
    // After sync:
    //
    // old_main ──> release_cut
    //              ├─> new_main (main@origin)
    //              │   └─> feature_payments'
    //              │       └─> feature_api' ─────┐
    //              └─> feature_payments
    //                  └─> release_bridge ───────┴─> release_merge'
    let test_repo = TestRepo::init();
    let mut tx = test_repo.repo.start_transaction();
    let old_main = write_random_commit(tx.repo_mut());
    let release_cut = write_random_commit_with_parents(tx.repo_mut(), &[&old_main]);
    let new_main = write_random_commit_with_parents(tx.repo_mut(), &[&release_cut]);
    let feature_payments = write_random_commit_with_parents(tx.repo_mut(), &[&release_cut]);
    let feature_api = write_random_commit_with_parents(tx.repo_mut(), &[&feature_payments]);
    let release_bridge = write_random_commit_with_parents(tx.repo_mut(), &[&feature_payments]);
    let release_merge =
        write_random_commit_with_parents(tx.repo_mut(), &[&feature_api, &release_bridge]);
    let repo = tx.commit("setup").block_on()?;

    let mut tx = repo.start_transaction();
    let stats = apply_import_updates(
        tx.repo_mut(),
        vec![imported_update("main", "origin", &old_main, &new_main)],
    );
    let result = sync_imported_refs(
        tx.repo_mut(),
        &stats,
        vec![
            feature_payments.id().clone(),
            feature_api.id().clone(),
            release_merge.id().clone(),
        ],
        &RebaseOptions::default(),
    )
    .block_on()?;
    let new_feature_payments = expect_rewritten(&result, &feature_payments);
    let new_feature_api = expect_rewritten(&result, &feature_api);
    let new_release_merge = expect_rewritten(&result, &release_merge);
    assert_eq!(
        new_feature_payments.parent_ids(),
        vec![new_main.id().clone()]
    );
    assert_eq!(
        new_feature_api.parent_ids(),
        vec![new_feature_payments.id().clone()]
    );
    assert!(!result.contains_key(release_bridge.id()));
    assert_eq!(
        new_release_merge.parent_ids(),
        vec![new_feature_api.id().clone(), release_bridge.id().clone()]
    );
    assert!(!tx.repo().has_rewrites());

    // The second attempt also selects release_bridge.
    //
    // After sync:
    //
    // old_main ──> release_cut
    //              └─> new_main (main@origin)
    //                  └─> feature_payments'
    //                      ├─> feature_api' ──────┐
    //                      └─> release_bridge' ───┴─> release_merge'
    let mut tx = repo.start_transaction();
    let stats = apply_import_updates(
        tx.repo_mut(),
        vec![imported_update("main", "origin", &old_main, &new_main)],
    );
    let result = sync_imported_refs(
        tx.repo_mut(),
        &stats,
        vec![
            feature_payments.id().clone(),
            feature_api.id().clone(),
            release_bridge.id().clone(),
            release_merge.id().clone(),
        ],
        &RebaseOptions::default(),
    )
    .block_on()?;
    let new_feature_payments = expect_rewritten(&result, &feature_payments);
    let new_feature_api = expect_rewritten(&result, &feature_api);
    let new_release_bridge = expect_rewritten(&result, &release_bridge);
    let new_release_merge = expect_rewritten(&result, &release_merge);
    assert_eq!(
        new_release_bridge.parent_ids(),
        vec![new_feature_payments.id().clone()]
    );
    assert_eq!(
        new_release_merge.parent_ids(),
        vec![
            new_feature_api.id().clone(),
            new_release_bridge.id().clone(),
        ]
    );
    assert!(!tx.repo().has_rewrites());

    Ok(())
}

#[test]
fn test_sync_rejects_unselected_intermediate_commit() -> TestResult {
    // Imported: main@origin moved from old_main to new_main.
    // Selected: feature_payments and release_candidate. release_bridge is
    // unselected.
    //
    // Before sync:
    //
    // old_main ─┬─> new_main (main@origin)
    //           └─> feature_payments ──> release_bridge ──> release_candidate
    //
    // Rejected because release_candidate has no selected-only path from the
    // rewrite root across release_bridge.
    let test_repo = TestRepo::init();
    let mut tx = test_repo.repo.start_transaction();
    let old_main = write_random_commit(tx.repo_mut());
    let new_main = write_random_commit_with_parents(tx.repo_mut(), &[&old_main]);
    let feature_payments = write_random_commit_with_parents(tx.repo_mut(), &[&old_main]);
    let release_bridge = write_random_commit_with_parents(tx.repo_mut(), &[&feature_payments]);
    let release_candidate = write_random_commit_with_parents(tx.repo_mut(), &[&release_bridge]);
    let stats = apply_import_updates(
        tx.repo_mut(),
        vec![imported_update("main", "origin", &old_main, &new_main)],
    );
    let error = sync_imported_refs(
        tx.repo_mut(),
        &stats,
        vec![
            feature_payments.id().clone(),
            release_candidate.id().clone(),
        ],
        &RebaseOptions::default(),
    )
    .block_on()
    .unwrap_err();
    assert_matches!(
        error,
        GitSyncError::SelectionBoundary { old, new, commit_ids }
            if old == old_main.id().clone()
                && new == new_main.id().clone()
                && commit_ids == vec![release_candidate.id().clone()]
    );
    Ok(())
}

#[test]
fn test_sync_collapses_parent_replacements_from_multiple_moves() -> TestResult {
    // Imported: main@origin and staging@origin moved old_main to new_main;
    // release@origin moved old_release to new_main.
    // Selected: release_merge.
    //
    // Before sync:
    //
    // [old_main, old_release] ─┬─> new_main
    //                         └─> release_merge
    //
    // After sync (duplicate boundary move and parent collapse):
    //
    // [old_main, old_release] ──> new_main ──> release_merge'
    let test_repo = TestRepo::init();
    let mut tx = test_repo.repo.start_transaction();
    let old_main = write_random_commit(tx.repo_mut());
    let old_release = write_random_commit(tx.repo_mut());
    let new_main = write_random_commit_with_parents(tx.repo_mut(), &[&old_main, &old_release]);
    let release_merge = write_random_commit_with_parents(tx.repo_mut(), &[&old_main, &old_release]);
    let stats = apply_import_updates(
        tx.repo_mut(),
        vec![
            imported_update("main", "origin", &old_main, &new_main),
            imported_update("staging", "origin", &old_main, &new_main),
            imported_update("release", "origin", &old_release, &new_main),
        ],
    );
    let result = sync_imported_refs(
        tx.repo_mut(),
        &stats,
        vec![release_merge.id().clone()],
        &RebaseOptions::default(),
    )
    .block_on()?;
    let new_release_merge = expect_rewritten(&result, &release_merge);
    assert_eq!(new_release_merge.parent_ids(), vec![new_main.id().clone()]);

    Ok(())
}

#[test]
fn test_sync_accepts_same_parent_replacement() -> TestResult {
    // Imported: main@origin moved old_base to new_main; release@origin moved
    // old_main to new_main. Selected: feature_tip.
    //
    // old_base ──> old_main ──> shared ─┬─> new_main (main@origin, release@origin)
    //                                    └─> feature_tip
    //
    // Both updates replace feature_tip's same parent edge with the same
    // destination, so the combined plan is unambiguous.
    let test_repo = TestRepo::init();
    let mut tx = test_repo.repo.start_transaction();
    let old_base = write_random_commit(tx.repo_mut());
    let old_main = write_random_commit_with_parents(tx.repo_mut(), &[&old_base]);
    let shared = write_random_commit_with_parents(tx.repo_mut(), &[&old_main]);
    let new_main = write_random_commit_with_parents(tx.repo_mut(), &[&shared]);
    let feature_tip = write_random_commit_with_parents(tx.repo_mut(), &[&shared]);
    let stats = apply_import_updates(
        tx.repo_mut(),
        vec![
            imported_update("main", "origin", &old_base, &new_main),
            imported_update("release", "origin", &old_main, &new_main),
        ],
    );

    let result = sync_imported_refs(
        tx.repo_mut(),
        &stats,
        vec![feature_tip.id().clone()],
        &RebaseOptions::default(),
    )
    .block_on()?;
    let new_feature_tip = expect_rewritten(&result, &feature_tip);
    assert_eq!(new_feature_tip.parent_ids(), vec![new_main.id().clone()]);
    Ok(())
}

#[test]
fn test_sync_accepts_independent_updates() -> TestResult {
    // Imported: main@origin and release@origin advanced independent lineages.
    // Selected: feature_main and feature_release.
    let test_repo = TestRepo::init();
    let mut tx = test_repo.repo.start_transaction();
    let old_main = write_random_commit(tx.repo_mut());
    let old_release = write_random_commit(tx.repo_mut());
    let new_main = write_random_commit_with_parents(tx.repo_mut(), &[&old_main]);
    let new_release = write_random_commit_with_parents(tx.repo_mut(), &[&old_release]);
    let feature_main = write_random_commit_with_parents(tx.repo_mut(), &[&old_main]);
    let feature_release = write_random_commit_with_parents(tx.repo_mut(), &[&old_release]);
    let stats = apply_import_updates(
        tx.repo_mut(),
        vec![
            imported_update("main", "origin", &old_main, &new_main),
            imported_update("release", "origin", &old_release, &new_release),
        ],
    );

    let result = sync_imported_refs(
        tx.repo_mut(),
        &stats,
        vec![feature_main.id().clone(), feature_release.id().clone()],
        &RebaseOptions::default(),
    )
    .block_on()?;
    assert_eq!(result.len(), 2);
    assert_eq!(
        expect_rewritten(&result, &feature_main).parent_ids(),
        vec![new_main.id().clone()]
    );
    assert_eq!(
        expect_rewritten(&result, &feature_release).parent_ids(),
        vec![new_release.id().clone()]
    );
    Ok(())
}

#[test]
fn test_sync_root_merge_honors_simplify_ancestor_merge() -> TestResult {
    // Imported: main@origin moved old to new through old_tip. Selected:
    // root_merge, whose old ancestor parent becomes redundant after replacement.
    let test_repo = TestRepo::init();
    let mut tx = test_repo.repo.start_transaction();
    let old = write_random_commit(tx.repo_mut());
    let old_tip = write_random_commit_with_parents(tx.repo_mut(), &[&old]);
    let new = write_random_commit_with_parents(tx.repo_mut(), &[&old_tip]);
    let root_merge = write_random_commit_with_parents(tx.repo_mut(), &[&old_tip, &old]);
    let stats = apply_import_updates(
        tx.repo_mut(),
        vec![imported_update("main", "origin", &old, &new)],
    );

    let result = sync_imported_refs(
        tx.repo_mut(),
        &stats,
        vec![root_merge.id().clone()],
        &RebaseOptions {
            simplify_ancestor_merge: true,
            ..Default::default()
        },
    )
    .block_on()?;
    assert_eq!(
        expect_rewritten(&result, &root_merge).parent_ids(),
        vec![new.id().clone()]
    );
    Ok(())
}

#[test]
fn test_sync_rejects_conflicting_parent_replacements() -> TestResult {
    // Imported: main@origin moved old_base to new_main; release@origin moved
    // old_main to new_release. Selected: feature_checkout.
    //
    // Before sync:
    //
    // old_base ──> old_main ──> shared ─┬─> new_main (main@origin)
    //                                   ├─> new_release (release@origin)
    //                                   └─> feature_checkout
    //
    // Rejected because the two overlapping moved segments would replace the
    // same parent of feature_checkout with different destinations.
    let test_repo = TestRepo::init();
    let mut tx = test_repo.repo.start_transaction();
    let old_base = write_random_commit(tx.repo_mut());
    let old_main = write_random_commit_with_parents(tx.repo_mut(), &[&old_base]);
    let shared = write_random_commit_with_parents(tx.repo_mut(), &[&old_main]);
    let new_main = write_random_commit_with_parents(tx.repo_mut(), &[&shared]);
    let new_release = write_random_commit_with_parents(tx.repo_mut(), &[&shared]);
    let feature_checkout = write_random_commit_with_parents(tx.repo_mut(), &[&shared]);
    let stats = apply_import_updates(
        tx.repo_mut(),
        vec![
            imported_update("main", "origin", &old_base, &new_main),
            imported_update("release", "origin", &old_main, &new_release),
        ],
    );
    let error = sync_imported_refs(
        tx.repo_mut(),
        &stats,
        vec![feature_checkout.id().clone()],
        &RebaseOptions::default(),
    )
    .block_on()
    .unwrap_err();
    let mut expected = vec![new_main.id().clone(), new_release.id().clone()];
    expected.sort_unstable();
    assert_matches!(
        error,
        GitSyncError::ConflictingParentReplacement {
            commit_id,
            parent_id,
            destinations,
        } if commit_id == feature_checkout.id().clone()
            && parent_id == shared.id().clone()
            && destinations == expected
    );

    Ok(())
}

#[test]
fn test_sync_requires_selected_path_for_each_update() -> TestResult {
    // Imported: main@origin moved old_main to new_main; release@origin moved
    // old_release to new_release.
    // Selected: feature_main, feature_release, and release_candidate.
    // release_bridge is unselected.
    //
    // Before sync:
    //
    // old_main ─┬─> new_main
    //           └─> feature_main ─────────────────────────┐
    //
    // old_release ─┬─> new_release                         │
    //              └─> feature_release ──> release_bridge ─┴─> release_candidate
    //
    // Rejected because the release update cannot cross the unselected bridge.
    let test_repo = TestRepo::init();
    let mut tx = test_repo.repo.start_transaction();
    let old_main = write_random_commit(tx.repo_mut());
    let old_release = write_random_commit(tx.repo_mut());
    let new_main = write_random_commit_with_parents(tx.repo_mut(), &[&old_main]);
    let new_release = write_random_commit_with_parents(tx.repo_mut(), &[&old_release]);
    let feature_main = write_random_commit_with_parents(tx.repo_mut(), &[&old_main]);
    let feature_release = write_random_commit_with_parents(tx.repo_mut(), &[&old_release]);
    let release_bridge = write_random_commit_with_parents(tx.repo_mut(), &[&feature_release]);
    let release_candidate =
        write_random_commit_with_parents(tx.repo_mut(), &[&feature_main, &release_bridge]);
    let stats = apply_import_updates(
        tx.repo_mut(),
        vec![
            imported_update("main", "origin", &old_main, &new_main),
            imported_update("release", "origin", &old_release, &new_release),
        ],
    );
    let error = sync_imported_refs(
        tx.repo_mut(),
        &stats,
        vec![
            feature_main.id().clone(),
            feature_release.id().clone(),
            release_candidate.id().clone(),
        ],
        &RebaseOptions::default(),
    )
    .block_on()
    .unwrap_err();
    assert_matches!(
        error,
        GitSyncError::SelectionBoundary { old, new, commit_ids }
            if old == old_release.id().clone()
                && new == new_release.id().clone()
                && commit_ids == vec![release_candidate.id().clone()]
    );
    Ok(())
}

#[test]
fn test_sync_accepts_same_name_updates_with_equal_destinations() -> TestResult {
    // Imported: main@origin moved old_main to new_main; newly tracked
    // main@backup was added at new_main.
    // Selected: feature_tip.
    //
    // Before sync:
    //
    // old_main ─┬─> new_main (main@origin, main@backup)
    //           └─> feature_tip
    //
    // After sync (same-name destinations agree):
    //
    // old_main ──> new_main (main@origin, main@backup) ──> feature_tip'
    let test_repo = TestRepo::init();
    let mut tx = test_repo.repo.start_transaction();
    let old_main = write_random_commit(tx.repo_mut());
    let new_main = write_random_commit_with_parents(tx.repo_mut(), &[&old_main]);
    let feature_tip = write_random_commit_with_parents(tx.repo_mut(), &[&old_main]);
    let stats = apply_import_updates(
        tx.repo_mut(),
        vec![
            GitImportRefUpdate::new(
                RemoteRefSymbol {
                    name: "main".as_ref(),
                    remote: "backup".as_ref(),
                }
                .to_owned(),
                RemoteRef::absent(),
                RefTarget::normal(new_main.id().clone()),
            ),
            imported_update("main", "origin", &old_main, &new_main),
        ],
    );
    let result = sync_imported_refs(
        tx.repo_mut(),
        &stats,
        vec![feature_tip.id().clone()],
        &RebaseOptions::default(),
    )
    .block_on()?;
    assert_eq!(result.len(), 1);
    let new_feature_tip = expect_rewritten(&result, &feature_tip);
    assert_eq!(new_feature_tip.parent_ids(), vec![new_main.id().clone()]);

    Ok(())
}

#[test]
fn test_sync_rejects_relevant_unsafe_remote_changes() -> TestResult {
    // Imported in separate cases: main@origin moved from old_main to
    // absent, to the unrelated release_hotfix, or to a conflicted target.
    // Selected: feature_checkout.
    //
    // Before sync:
    //
    // old_main ──> feature_checkout
    // release_hotfix (force-push destination in the second case)
    //
    // Rejected because the relevant remote bookmark was deleted, conflicted,
    // or moved by a non-fast-forward update.
    {
        let test_repo = TestRepo::init();
        let mut tx = test_repo.repo.start_transaction();
        let old_main = write_random_commit(tx.repo_mut());
        let feature_checkout = write_random_commit_with_parents(tx.repo_mut(), &[&old_main]);
        let stats = apply_import_updates(
            tx.repo_mut(),
            vec![GitImportRefUpdate::new(
                RemoteRefSymbol {
                    name: "main".as_ref(),
                    remote: "origin".as_ref(),
                }
                .to_owned(),
                RemoteRef {
                    target: RefTarget::normal(old_main.id().clone()),
                    state: RemoteRefState::Tracked,
                },
                RefTarget::absent(),
            )],
        );
        let error = sync_imported_refs(
            tx.repo_mut(),
            &stats,
            vec![feature_checkout.id().clone()],
            &RebaseOptions::default(),
        )
        .block_on()
        .unwrap_err();
        assert_matches!(
            error,
            GitSyncError::UnsupportedRemoteChange {
                reason: GitSyncUnsupportedReason::NewTargetAbsent,
                ..
            }
        );
    }

    {
        let test_repo = TestRepo::init();
        let mut tx = test_repo.repo.start_transaction();
        let old_main = write_random_commit(tx.repo_mut());
        let feature_checkout = write_random_commit_with_parents(tx.repo_mut(), &[&old_main]);
        let release_hotfix = write_random_commit(tx.repo_mut());
        let stats = apply_import_updates(
            tx.repo_mut(),
            vec![GitImportRefUpdate::new(
                RemoteRefSymbol {
                    name: "main".as_ref(),
                    remote: "origin".as_ref(),
                }
                .to_owned(),
                RemoteRef {
                    target: RefTarget::normal(old_main.id().clone()),
                    state: RemoteRefState::Tracked,
                },
                RefTarget::normal(release_hotfix.id().clone()),
            )],
        );
        let error = sync_imported_refs(
            tx.repo_mut(),
            &stats,
            vec![feature_checkout.id().clone()],
            &RebaseOptions::default(),
        )
        .block_on()
        .unwrap_err();
        assert_matches!(
            error,
            GitSyncError::UnsupportedRemoteChange {
                reason: GitSyncUnsupportedReason::NonFastForward { .. },
                ..
            }
        );
    }

    {
        let test_repo = TestRepo::init();
        let mut tx = test_repo.repo.start_transaction();
        let old_main = write_random_commit(tx.repo_mut());
        let feature_checkout = write_random_commit_with_parents(tx.repo_mut(), &[&old_main]);
        let new_main = write_random_commit_with_parents(tx.repo_mut(), &[&old_main]);
        let release_hotfix = write_random_commit(tx.repo_mut());
        let conflict =
            RefTarget::from_legacy_form([], [new_main.id().clone(), release_hotfix.id().clone()]);
        let stats = apply_import_updates(
            tx.repo_mut(),
            vec![GitImportRefUpdate::new(
                RemoteRefSymbol {
                    name: "main".as_ref(),
                    remote: "origin".as_ref(),
                }
                .to_owned(),
                RemoteRef {
                    target: RefTarget::normal(old_main.id().clone()),
                    state: RemoteRefState::Tracked,
                },
                conflict.clone(),
            )],
        );
        let error = sync_imported_refs(
            tx.repo_mut(),
            &stats,
            vec![feature_checkout.id().clone()],
            &RebaseOptions::default(),
        )
        .block_on()
        .unwrap_err();
        assert_matches!(
            error,
            GitSyncError::UnsupportedRemoteChange {
                reason: GitSyncUnsupportedReason::NewTargetConflicted { target },
                ..
            } if target == conflict
        );
    }

    Ok(())
}

#[test]
fn test_sync_ignores_unrelated_unsafe_remote_change() -> TestResult {
    // Imported: staging@origin moved from old_main to unrelated new_main.
    // Selected: feature_selected, which has no ancestry relationship to either.
    //
    // Before sync:
    //
    // feature_selected    old_main    new_main (staging@origin)
    //
    // After sync: unchanged. The unsafe update is ignored because old_main is
    // outside the selected ancestry.
    let test_repo = TestRepo::init();
    let mut tx = test_repo.repo.start_transaction();
    let feature_selected = write_random_commit(tx.repo_mut());
    let old_main = write_random_commit(tx.repo_mut());
    let new_main = write_random_commit(tx.repo_mut());
    let stats = apply_import_updates(
        tx.repo_mut(),
        vec![imported_update("staging", "origin", &old_main, &new_main)],
    );
    assert!(
        sync_imported_refs(
            tx.repo_mut(),
            &stats,
            vec![feature_selected.id().clone()],
            &RebaseOptions::default(),
        )
        .block_on()?
        .is_empty()
    );
    Ok(())
}

#[test]
fn test_sync_rejects_conflicted_old_target() -> TestResult {
    // Imported: main@origin and z-last@origin moved from the conflicted target
    // {old_main, old_release} to new_main.
    // Selected: feature_checkout.
    //
    // Before sync:
    //
    // old_main ─┬─> new_main (main@origin, z-last@origin)
    //           └─> feature_checkout
    // old_release (other conflicted old target)
    //
    // Rejected because old_main makes the conflicted old target relevant to the
    // selection. Updates are sorted, so main@origin is reported in the error.
    let test_repo = TestRepo::init();
    let mut tx = test_repo.repo.start_transaction();
    let old_main = write_random_commit(tx.repo_mut());
    let old_release = write_random_commit(tx.repo_mut());
    let new_main = write_random_commit_with_parents(tx.repo_mut(), &[&old_main]);
    let feature_checkout = write_random_commit_with_parents(tx.repo_mut(), &[&old_main]);
    let conflict =
        RefTarget::from_legacy_form([], [old_main.id().clone(), old_release.id().clone()]);
    let stats = apply_import_updates(
        tx.repo_mut(),
        ["z-last", "main"]
            .into_iter()
            .map(|name| {
                GitImportRefUpdate::new(
                    RemoteRefSymbol {
                        name: name.as_ref(),
                        remote: "origin".as_ref(),
                    }
                    .to_owned(),
                    RemoteRef {
                        target: conflict.clone(),
                        state: RemoteRefState::Tracked,
                    },
                    RefTarget::normal(new_main.id().clone()),
                )
            })
            .collect(),
    );
    let error = sync_imported_refs(
        tx.repo_mut(),
        &stats,
        vec![feature_checkout.id().clone()],
        &RebaseOptions::default(),
    )
    .block_on()
    .unwrap_err();
    assert_matches!(
        error,
        GitSyncError::UnsupportedRemoteChange {
            symbol,
            reason: GitSyncUnsupportedReason::OldTargetConflicted { target },
        } if symbol.name.as_str() == "main" && target == conflict
    );

    Ok(())
}

#[test]
fn test_sync_rejects_ambiguous_import_destinations() -> TestResult {
    // Imported: main@a moved old_main to new_main; main@z moved old_main to
    // new_release. Selected: feature_checkout.
    //
    // Before sync:
    //
    // old_main ─┬─> feature_checkout
    //           ├─> new_main (main@a)
    //           └─> new_release (main@z)
    //
    // Rejected because remotes for the same bookmark name have different
    // destinations. The error reports them in commit-ID order.
    let test_repo = TestRepo::init();
    let mut tx = test_repo.repo.start_transaction();
    let old_main = write_random_commit(tx.repo_mut());
    let new_main = write_random_commit_with_parents(tx.repo_mut(), &[&old_main]);
    let new_release = write_random_commit_with_parents(tx.repo_mut(), &[&old_main]);
    let feature_checkout = write_random_commit_with_parents(tx.repo_mut(), &[&old_main]);
    let repo = tx.commit("setup").block_on()?;
    let mut expected = vec![new_main.id().clone(), new_release.id().clone()];
    expected.sort_unstable();

    {
        let mut tx = repo.start_transaction();
        let stats = apply_import_updates(
            tx.repo_mut(),
            vec![
                imported_update("main", "z", &old_main, &new_release),
                imported_update("main", "a", &old_main, &new_main),
            ],
        );
        let error = sync_imported_refs(
            tx.repo_mut(),
            &stats,
            vec![feature_checkout.id().clone()],
            &RebaseOptions::default(),
        )
        .block_on()
        .unwrap_err();
        assert_matches!(
            error,
            GitSyncError::AmbiguousBookmark { destinations, .. } if destinations == expected
        );
    }

    // Imported: main@origin and staging@origin moved old_main to new_release;
    // release@origin moved old_main to new_main.
    //
    // Before sync:
    //
    // old_main ─┬─> feature_checkout
    //           ├─> new_main (release@origin)
    //           └─> new_release (main@origin, staging@origin)
    //
    // Rejected because different bookmark names still assign the same old
    // boundary to different destinations.
    {
        let mut tx = repo.start_transaction();
        let stats = apply_import_updates(
            tx.repo_mut(),
            vec![
                imported_update("staging", "origin", &old_main, &new_release),
                imported_update("release", "origin", &old_main, &new_main),
                imported_update("main", "origin", &old_main, &new_release),
            ],
        );
        let error = sync_imported_refs(
            tx.repo_mut(),
            &stats,
            vec![feature_checkout.id().clone()],
            &RebaseOptions::default(),
        )
        .block_on()
        .unwrap_err();
        assert_matches!(
            error,
            GitSyncError::AmbiguousBoundary { old, destinations }
                if old == old_main.id().clone() && destinations == expected
        );
    }

    Ok(())
}

#[test]
fn test_sync_rejects_moving_imported_destination() -> TestResult {
    // Imported: main@origin moved old_main to new_main; release@origin moved
    // old_release to new_release. Selected: new_release.
    //
    // Before sync:
    //
    // old_release ──> old_main
    //                 ├─> new_main (main@origin)
    //                 └─> new_release (release@origin)
    //
    // Rejected because the main move would rewrite new_release, which is also
    // the imported destination of the release move.
    let test_repo = TestRepo::init();
    let mut tx = test_repo.repo.start_transaction();
    let old_release = write_random_commit(tx.repo_mut());
    let old_main = write_random_commit_with_parents(tx.repo_mut(), &[&old_release]);
    let new_main = write_random_commit_with_parents(tx.repo_mut(), &[&old_main]);
    let new_release = write_random_commit_with_parents(tx.repo_mut(), &[&old_main]);
    let stats = apply_import_updates(
        tx.repo_mut(),
        vec![
            imported_update("main", "origin", &old_main, &new_main),
            imported_update("release", "origin", &old_release, &new_release),
        ],
    );
    let error = sync_imported_refs(
        tx.repo_mut(),
        &stats,
        vec![new_release.id().clone()],
        &RebaseOptions::default(),
    )
    .block_on()
    .unwrap_err();
    assert_matches!(
        error,
        GitSyncError::MovingDestination { commit_ids }
            if commit_ids == vec![new_release.id().clone()]
    );

    Ok(())
}

#[test]
fn test_sync_rejects_pending_rewrites_before_loading_selection() -> TestResult {
    // Before sync: the transaction already maps old_main to new_main.
    //
    // Rejected before loading the deliberately invalid selected commit ID.
    let test_repo = TestRepo::init();
    let mut tx = test_repo.repo.start_transaction();
    let old_main = write_random_commit(tx.repo_mut());
    let new_main = write_random_commit(tx.repo_mut());
    tx.repo_mut()
        .set_rewritten_commit(old_main.id().clone(), new_main.id().clone());
    assert_matches!(
        sync_imported_refs(
            tx.repo_mut(),
            &GitImportStats::default(),
            vec![CommitId::from_hex("deadbeef")],
            &RebaseOptions::default(),
        )
        .block_on()
        .unwrap_err(),
        GitSyncError::PendingRewrites
    );
    Ok(())
}
