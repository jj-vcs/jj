// Copyright 2021 The Jujutsu Authors
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

use std::iter;
use std::path::Path;

use assert_matches::assert_matches;
use chrono::DateTime;
use itertools::Itertools;
use jj_lib::backend::CommitId;
use jj_lib::backend::MillisSinceEpoch;
use jj_lib::backend::Signature;
use jj_lib::backend::Timestamp;
use jj_lib::commit::Commit;
use jj_lib::fileset::FilesetExpression;
use jj_lib::git;
use jj_lib::git_backend::GitBackend;
use jj_lib::graph::reverse_graph;
use jj_lib::graph::GraphEdge;
use jj_lib::id_prefix::IdPrefixContext;
use jj_lib::object_id::ObjectId;
use jj_lib::op_store::RefTarget;
use jj_lib::op_store::RemoteRef;
use jj_lib::op_store::RemoteRefState;
use jj_lib::op_store::WorkspaceId;
use jj_lib::repo::Repo;
use jj_lib::repo_path::RepoPath;
use jj_lib::repo_path::RepoPathUiConverter;
use jj_lib::revset::parse;
use jj_lib::revset::DefaultSymbolResolver;
use jj_lib::revset::FailingSymbolResolver;
use jj_lib::revset::Revset;
use jj_lib::revset::RevsetAliasesMap;
use jj_lib::revset::RevsetDiagnostics;
use jj_lib::revset::RevsetExpression;
use jj_lib::revset::RevsetExtensions;
use jj_lib::revset::RevsetFilterPredicate;
use jj_lib::revset::RevsetParseContext;
use jj_lib::revset::RevsetResolutionError;
use jj_lib::revset::RevsetWorkspaceContext;
use jj_lib::revset::SymbolResolver;
use jj_lib::revset::SymbolResolverExtension;
use jj_lib::settings::GitSettings;
use jj_lib::workspace::Workspace;
use test_case::test_case;
use testutils::create_random_commit;
use testutils::create_tree;
use testutils::write_random_commit;
use testutils::CommitGraphBuilder;
use testutils::TestRepo;
use testutils::TestRepoBackend;
use testutils::TestWorkspace;

fn resolve_symbol_with_extensions(
    repo: &dyn Repo,
    extensions: &RevsetExtensions,
    symbol: &str,
) -> Result<Vec<CommitId>, RevsetResolutionError> {
    let aliases_map = RevsetAliasesMap::default();
    let now = chrono::Local::now();
    let context = RevsetParseContext::new(&aliases_map, "", now.into(), extensions, None);
    let expression = parse(&mut RevsetDiagnostics::new(), symbol, &context).unwrap();
    assert_matches!(*expression, RevsetExpression::CommitRef(_));
    let symbol_resolver = DefaultSymbolResolver::new(repo, extensions.symbol_resolvers());
    match expression
        .resolve_user_expression(repo, &symbol_resolver)?
        .as_ref()
    {
        RevsetExpression::Commits(commits) => Ok(commits.clone()),
        expression => panic!("symbol resolved to compound expression: {expression:?}"),
    }
}

fn resolve_symbol(repo: &dyn Repo, symbol: &str) -> Result<Vec<CommitId>, RevsetResolutionError> {
    resolve_symbol_with_extensions(repo, &RevsetExtensions::default(), symbol)
}

fn revset_for_commits<'index>(
    repo: &'index dyn Repo,
    commits: &[&Commit],
) -> Box<dyn Revset + 'index> {
    let symbol_resolver =
        DefaultSymbolResolver::new(repo, &([] as [&Box<dyn SymbolResolverExtension>; 0]));
    RevsetExpression::commits(commits.iter().map(|commit| commit.id().clone()).collect())
        .resolve_user_expression(repo, &symbol_resolver)
        .unwrap()
        .evaluate(repo)
        .unwrap()
}

#[test]
fn test_resolve_symbol_empty_string() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    assert_matches!(
        resolve_symbol(repo.as_ref(), r#""""#),
        Err(RevsetResolutionError::EmptyString)
    );
}

#[test]
fn test_resolve_symbol_commit_id() {
    let settings = testutils::user_settings();
    // Test only with git so we can get predictable commit ids
    let test_repo = TestRepo::init_with_backend(TestRepoBackend::Git);
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();
    let signature = Signature {
        name: "test".to_string(),
        email: "test".to_string(),
        timestamp: Timestamp {
            timestamp: MillisSinceEpoch(0),
            tz_offset: 0,
        },
    };

    let mut commits = vec![];
    for i in &[1, 167, 895] {
        let commit = mut_repo
            .new_commit(
                vec![repo.store().root_commit_id().clone()],
                repo.store().empty_merged_tree_id(),
            )
            .set_description(format!("test {i}"))
            .set_author(signature.clone())
            .set_committer(signature.clone())
            .write()
            .unwrap();
        commits.push(commit);
    }
    let repo = tx.commit("test").unwrap();

    // Test the test setup
    assert_eq!(
        commits[0].id().hex(),
        "0454de3cae04c46cda37ba2e8873b4c17ff51dcb"
    );
    assert_eq!(
        commits[1].id().hex(),
        "045f56cd1b17e8abde86771e2705395dcde6a957"
    );
    assert_eq!(
        commits[2].id().hex(),
        "0468f7da8de2ce442f512aacf83411d26cd2e0cf"
    );

    // Change ids should never have prefix "04"
    insta::assert_snapshot!(commits[0].change_id().hex(), @"781199f9d55d18e855a7aa84c5e4b40d");
    insta::assert_snapshot!(commits[1].change_id().hex(), @"a2c96fc88f32e487328f04927f20c4b1");
    insta::assert_snapshot!(commits[2].change_id().hex(), @"4399e4f3123763dfe7d68a2809ecc01b");

    // Test lookup by full commit id
    assert_eq!(
        resolve_symbol(repo.as_ref(), "0454de3cae04c46cda37ba2e8873b4c17ff51dcb",).unwrap(),
        vec![commits[0].id().clone()]
    );
    assert_eq!(
        resolve_symbol(repo.as_ref(), "045f56cd1b17e8abde86771e2705395dcde6a957",).unwrap(),
        vec![commits[1].id().clone()]
    );
    assert_eq!(
        resolve_symbol(repo.as_ref(), "0468f7da8de2ce442f512aacf83411d26cd2e0cf",).unwrap(),
        vec![commits[2].id().clone()]
    );

    // Test commit id prefix
    assert_eq!(
        resolve_symbol(repo.as_ref(), "046").unwrap(),
        vec![commits[2].id().clone()]
    );
    assert_matches!(
        resolve_symbol(repo.as_ref(), "04"),
        Err(RevsetResolutionError::AmbiguousCommitIdPrefix(s)) if s == "04"
    );
    assert_matches!(
        resolve_symbol(repo.as_ref(), "040"),
        Err(RevsetResolutionError::NoSuchRevision{name, candidates}) if name == "040" && candidates.is_empty()
    );

    // Test non-hex string
    assert_matches!(
        resolve_symbol(repo.as_ref(), "foo"),
        Err(RevsetResolutionError::NoSuchRevision{name, candidates}) if name == "foo" && candidates.is_empty()
    );

    // Test present() suppresses only NoSuchRevision error
    assert_eq!(resolve_commit_ids(repo.as_ref(), "present(foo)"), []);
    let symbol_resolver = DefaultSymbolResolver::new(
        repo.as_ref(),
        &([] as [&Box<dyn SymbolResolverExtension>; 0]),
    );
    let aliases_map = RevsetAliasesMap::default();
    let extensions = RevsetExtensions::default();
    let context = RevsetParseContext::new(
        &aliases_map,
        settings.user_email(),
        chrono::Utc::now().fixed_offset().into(),
        &extensions,
        None,
    );
    assert_matches!(
        parse(&mut RevsetDiagnostics::new(), "present(04)", &context).unwrap()
            .resolve_user_expression(repo.as_ref(), &symbol_resolver),
        Err(RevsetResolutionError::AmbiguousCommitIdPrefix(s)) if s == "04"
    );
    assert_eq!(
        resolve_commit_ids(repo.as_ref(), "present(046)"),
        vec![commits[2].id().clone()]
    );
}

#[test_case(false ; "mutable")]
#[test_case(true ; "readonly")]
fn test_resolve_symbol_change_id(readonly: bool) {
    let git_settings = GitSettings::default();
    // Test only with git so we can get predictable change ids
    let test_repo = TestRepo::init_with_backend(TestRepoBackend::Git);
    let repo = &test_repo.repo;

    let git_repo = repo
        .store()
        .backend_impl()
        .downcast_ref::<GitBackend>()
        .unwrap()
        .open_git_repo()
        .unwrap();
    // Add some commits that will end up having change ids with common prefixes
    let empty_tree_id = git_repo.treebuilder(None).unwrap().write().unwrap();
    let git_author = git2::Signature::new(
        "git author",
        "git.author@example.com",
        &git2::Time::new(1000, 60),
    )
    .unwrap();
    let git_committer = git2::Signature::new(
        "git committer",
        "git.committer@example.com",
        &git2::Time::new(2000, -480),
    )
    .unwrap();
    let git_tree = git_repo.find_tree(empty_tree_id).unwrap();
    let mut git_commit_ids = vec![];
    for i in &[133, 664, 840, 5085] {
        let git_commit_id = git_repo
            .commit(
                Some(&format!("refs/heads/bookmark{i}")),
                &git_author,
                &git_committer,
                &format!("test {i}"),
                &git_tree,
                &[],
            )
            .unwrap();
        git_commit_ids.push(git_commit_id);
    }

    let mut tx = repo.start_transaction();
    git::import_refs(tx.repo_mut(), &git_settings).unwrap();

    // Test the test setup
    assert_eq!(
        hex::encode(git_commit_ids[0]),
        // "04e12a5467bba790efb88a9870894ec208b16bf1" reversed
        "8fd68d104372910e19511df709e5dde62a548720"
    );
    assert_eq!(
        hex::encode(git_commit_ids[1]),
        // "040b3ba3a51d8edbc4c5855cbd09de71d4c29cca" reversed
        "5339432b8e7b90bd3aa1a323db71b8a5c5dcd020"
    );
    assert_eq!(
        hex::encode(git_commit_ids[2]),
        // "04e1c7082e4e34f3f371d8a1a46770b861b9b547" reversed
        "e2ad9d861d0ee625851b8ecfcf2c727410e38720"
    );
    assert_eq!(
        hex::encode(git_commit_ids[3]),
        // "911d7e52fd5ba04b8f289e14c3d30b52d38c0020" reversed
        "040031cb4ad0cbc3287914f1d205dabf4a7eb889"
    );

    let _readonly_repo;
    let repo: &dyn Repo = if readonly {
        _readonly_repo = tx.commit("test").unwrap();
        _readonly_repo.as_ref()
    } else {
        tx.repo_mut()
    };

    // Test lookup by full change id
    assert_eq!(
        resolve_symbol(repo, "zvlyxpuvtsoopsqzlkorrpqrszrqvlnx").unwrap(),
        vec![CommitId::from_hex(
            "8fd68d104372910e19511df709e5dde62a548720"
        )]
    );
    assert_eq!(
        resolve_symbol(repo, "zvzowopwpuymrlmonvnuruunomzqmlsy").unwrap(),
        vec![CommitId::from_hex(
            "5339432b8e7b90bd3aa1a323db71b8a5c5dcd020"
        )]
    );
    assert_eq!(
        resolve_symbol(repo, "zvlynszrxlvlwvkwkwsymrpypvtsszor").unwrap(),
        vec![CommitId::from_hex(
            "e2ad9d861d0ee625851b8ecfcf2c727410e38720"
        )]
    );

    // Test change id prefix
    assert_eq!(
        resolve_symbol(repo, "zvlyx").unwrap(),
        vec![CommitId::from_hex(
            "8fd68d104372910e19511df709e5dde62a548720"
        )]
    );
    assert_eq!(
        resolve_symbol(repo, "zvlyn").unwrap(),
        vec![CommitId::from_hex(
            "e2ad9d861d0ee625851b8ecfcf2c727410e38720"
        )]
    );
    assert_matches!(
        resolve_symbol(repo, "zvly"),
        Err(RevsetResolutionError::AmbiguousChangeIdPrefix(s)) if s == "zvly"
    );
    assert_matches!(
        resolve_symbol(repo, "zvlyw"),
        Err(RevsetResolutionError::NoSuchRevision{name, candidates}) if name == "zvlyw" && candidates.is_empty()
    );

    // Test that commit and changed id don't conflict ("040" and "zvz" are the
    // same).
    assert_eq!(
        resolve_symbol(repo, "040").unwrap(),
        vec![CommitId::from_hex(
            "040031cb4ad0cbc3287914f1d205dabf4a7eb889"
        )]
    );
    assert_eq!(
        resolve_symbol(repo, "zvz").unwrap(),
        vec![CommitId::from_hex(
            "5339432b8e7b90bd3aa1a323db71b8a5c5dcd020"
        )]
    );

    // Test non-hex string
    assert_matches!(
        resolve_symbol(repo, "foo"),
        Err(RevsetResolutionError::NoSuchRevision{
            name,
            candidates
        }) if name == "foo" && candidates.is_empty()
    );
}

#[test]
fn test_resolve_symbol_in_different_disambiguation_context() {
    let test_repo = TestRepo::init();
    let repo0 = &test_repo.repo;

    let mut tx = repo0.start_transaction();
    let commit1 = write_random_commit(tx.repo_mut());
    // Create more commits that are likely to conflict with 1-char hex prefix.
    for _ in 0..50 {
        write_random_commit(tx.repo_mut());
    }
    let repo1 = tx.commit("test").unwrap();

    let mut tx = repo1.start_transaction();
    let commit2 = tx.repo_mut().rewrite_commit(&commit1).write().unwrap();
    tx.repo_mut().rebase_descendants().unwrap();
    let repo2 = tx.commit("test").unwrap();

    // Set up disambiguation index which only contains the commit2.id().
    let id_prefix_context = IdPrefixContext::new(Default::default())
        .disambiguate_within(RevsetExpression::commit(commit2.id().clone()));
    let symbol_resolver =
        DefaultSymbolResolver::new(repo2.as_ref(), &[] as &[Box<dyn SymbolResolverExtension>])
            .with_id_prefix_context(&id_prefix_context);

    // Sanity check
    let change_hex = commit2.change_id().reverse_hex();
    assert_eq!(
        symbol_resolver
            .resolve_symbol(repo2.as_ref(), &change_hex[0..1])
            .unwrap(),
        vec![commit2.id().clone()]
    );
    assert_eq!(
        symbol_resolver
            .resolve_symbol(repo2.as_ref(), &commit2.id().hex()[0..1])
            .unwrap(),
        vec![commit2.id().clone()]
    );

    // Change ID is disambiguated within repo2, then resolved in repo1.
    assert_eq!(
        symbol_resolver
            .resolve_symbol(repo1.as_ref(), &change_hex[0..1])
            .unwrap(),
        vec![commit1.id().clone()]
    );

    // Commit ID can be found in the disambiguation index, but doesn't exist in
    // repo1.
    assert_matches!(
        symbol_resolver.resolve_symbol(repo1.as_ref(), &commit2.id().hex()[0..1]),
        Err(RevsetResolutionError::NoSuchRevision { .. })
    );
}

#[test]
fn test_resolve_working_copy() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();

    let commit1 = write_random_commit(mut_repo);
    let commit2 = write_random_commit(mut_repo);

    let ws1 = WorkspaceId::new("ws1".to_string());
    let ws2 = WorkspaceId::new("ws2".to_string());

    // Cannot resolve a working-copy commit for an unknown workspace
    assert_matches!(
        RevsetExpression::working_copy(ws1.clone())
            .resolve_user_expression(mut_repo, &FailingSymbolResolver),
        Err(RevsetResolutionError::WorkspaceMissingWorkingCopy { name }) if name == "ws1"
    );

    // The error can be suppressed by present()
    assert_eq!(
        RevsetExpression::working_copy(ws1.clone())
            .present()
            .resolve_user_expression(mut_repo, &FailingSymbolResolver)
            .unwrap()
            .evaluate(mut_repo)
            .unwrap()
            .iter()
            .map(Result::unwrap)
            .collect_vec(),
        vec![]
    );

    // Add some workspaces
    mut_repo
        .set_wc_commit(ws1.clone(), commit1.id().clone())
        .unwrap();
    mut_repo
        .set_wc_commit(ws2.clone(), commit2.id().clone())
        .unwrap();
    let resolve = |ws_id: WorkspaceId| -> Vec<CommitId> {
        RevsetExpression::working_copy(ws_id)
            .resolve_user_expression(mut_repo, &FailingSymbolResolver)
            .unwrap()
            .evaluate(mut_repo)
            .unwrap()
            .iter()
            .map(Result::unwrap)
            .collect()
    };

    // Can resolve "@" shorthand with a default workspace ID
    assert_eq!(resolve(ws1), vec![commit1.id().clone()]);
    // Can resolve an explicit checkout
    assert_eq!(resolve(ws2), vec![commit2.id().clone()]);
}

#[test]
fn test_resolve_working_copies() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();

    let commit1 = write_random_commit(mut_repo);
    let commit2 = write_random_commit(mut_repo);

    // Add some workspaces
    let ws1 = WorkspaceId::new("ws1".to_string());
    let ws2 = WorkspaceId::new("ws2".to_string());

    // add one commit to each working copy
    mut_repo
        .set_wc_commit(ws1.clone(), commit1.id().clone())
        .unwrap();
    mut_repo
        .set_wc_commit(ws2.clone(), commit2.id().clone())
        .unwrap();
    let resolve = || -> Vec<CommitId> {
        RevsetExpression::working_copies()
            .resolve_user_expression(mut_repo, &FailingSymbolResolver)
            .unwrap()
            .evaluate(mut_repo)
            .unwrap()
            .iter()
            .map(Result::unwrap)
            .collect()
    };

    // ensure our output has those two commits
    assert_eq!(resolve(), vec![commit2.id().clone(), commit1.id().clone()]);
}

#[test]
fn test_resolve_symbol_bookmarks() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;
    let new_remote_ref = |target| RemoteRef {
        target,
        state: RemoteRefState::New,
    };
    let tracking_remote_ref = |target| RemoteRef {
        target,
        state: RemoteRefState::Tracking,
    };
    let normal_tracking_remote_ref =
        |id: &CommitId| tracking_remote_ref(RefTarget::normal(id.clone()));

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();

    let commit1 = write_random_commit(mut_repo);
    let commit2 = write_random_commit(mut_repo);
    let commit3 = write_random_commit(mut_repo);
    let commit4 = write_random_commit(mut_repo);
    let commit5 = write_random_commit(mut_repo);

    mut_repo.set_local_bookmark_target("local", RefTarget::normal(commit1.id().clone()));
    mut_repo.set_remote_bookmark("remote", "origin", normal_tracking_remote_ref(commit2.id()));
    mut_repo.set_local_bookmark_target("local-remote", RefTarget::normal(commit3.id().clone()));
    mut_repo.set_remote_bookmark(
        "local-remote",
        "origin",
        normal_tracking_remote_ref(commit4.id()),
    );
    mut_repo.set_local_bookmark_target(
        "local-remote@origin", // not a remote bookmark
        RefTarget::normal(commit5.id().clone()),
    );
    mut_repo.set_remote_bookmark(
        "local-remote",
        "mirror",
        tracking_remote_ref(mut_repo.get_local_bookmark("local-remote")),
    );
    mut_repo.set_remote_bookmark(
        "local-remote",
        "untracked",
        new_remote_ref(mut_repo.get_local_bookmark("local-remote")),
    );
    mut_repo.set_remote_bookmark(
        "local-remote",
        git::REMOTE_NAME_FOR_LOCAL_GIT_REPO,
        tracking_remote_ref(mut_repo.get_local_bookmark("local-remote")),
    );

    mut_repo.set_local_bookmark_target(
        "local-conflicted",
        RefTarget::from_legacy_form(
            [commit1.id().clone()],
            [commit3.id().clone(), commit2.id().clone()],
        ),
    );
    mut_repo.set_remote_bookmark(
        "remote-conflicted",
        "origin",
        tracking_remote_ref(RefTarget::from_legacy_form(
            [commit3.id().clone()],
            [commit5.id().clone(), commit4.id().clone()],
        )),
    );

    // Local only
    assert_eq!(
        resolve_symbol(mut_repo, "local").unwrap(),
        vec![commit1.id().clone()],
    );
    insta::assert_debug_snapshot!(
        resolve_symbol(mut_repo, "local@origin").unwrap_err(), @r###"
    NoSuchRevision {
        name: "local@origin",
        candidates: [
            "local",
            "local-remote@git",
            "local-remote@mirror",
            "local-remote@origin",
            "remote@origin",
        ],
    }
    "###);

    // Remote only (or locally deleted)
    insta::assert_debug_snapshot!(
        resolve_symbol(mut_repo, "remote").unwrap_err(), @r###"
    NoSuchRevision {
        name: "remote",
        candidates: [
            "remote-conflicted@origin",
            "remote@origin",
        ],
    }
    "###);
    assert_eq!(
        resolve_symbol(mut_repo, "remote@origin").unwrap(),
        vec![commit2.id().clone()],
    );

    // Local/remote/git
    assert_eq!(
        resolve_symbol(mut_repo, "local-remote").unwrap(),
        vec![commit3.id().clone()],
    );
    assert_eq!(
        resolve_symbol(mut_repo, "local-remote@origin").unwrap(),
        vec![commit4.id().clone()],
    );
    assert_eq!(
        resolve_symbol(mut_repo, r#""local-remote@origin""#).unwrap(),
        vec![commit5.id().clone()],
    );
    assert_eq!(
        resolve_symbol(mut_repo, "local-remote@mirror").unwrap(),
        vec![commit3.id().clone()],
    );
    assert_eq!(
        resolve_symbol(mut_repo, "local-remote@git").unwrap(),
        vec![commit3.id().clone()],
    );

    // Conflicted
    assert_eq!(
        resolve_symbol(mut_repo, "local-conflicted").unwrap(),
        vec![commit3.id().clone(), commit2.id().clone()],
    );
    assert_eq!(
        resolve_symbol(mut_repo, "remote-conflicted@origin").unwrap(),
        vec![commit5.id().clone(), commit4.id().clone()],
    );

    // Typo of local/remote bookmark name:
    // For "local-emote" (without @remote part), "local-remote@mirror"/"@git" aren't
    // suggested since they point to the same target as "local-remote". OTOH,
    // "local-remote@untracked" is suggested because non-tracking bookmark is
    // unrelated to the local bookmark of the same name.
    insta::assert_debug_snapshot!(
        resolve_symbol(mut_repo, "local-emote").unwrap_err(), @r###"
    NoSuchRevision {
        name: "local-emote",
        candidates: [
            "local",
            "local-conflicted",
            "local-remote",
            "local-remote@origin",
            "local-remote@untracked",
        ],
    }
    "###);
    insta::assert_debug_snapshot!(
        resolve_symbol(mut_repo, "local-emote@origin").unwrap_err(), @r###"
    NoSuchRevision {
        name: "local-emote@origin",
        candidates: [
            "local",
            "local-remote",
            "local-remote@git",
            "local-remote@mirror",
            "local-remote@origin",
            "local-remote@untracked",
            "remote-conflicted@origin",
            "remote@origin",
        ],
    }
    "###);
    insta::assert_debug_snapshot!(
        resolve_symbol(mut_repo, "local-remote@origine").unwrap_err(), @r###"
    NoSuchRevision {
        name: "local-remote@origine",
        candidates: [
            "local",
            "local-remote",
            "local-remote@git",
            "local-remote@mirror",
            "local-remote@origin",
            "local-remote@untracked",
            "remote-conflicted@origin",
            "remote@origin",
        ],
    }
    "###);
    // "local-remote@mirror" shouldn't be omitted just because it points to the same
    // target as "local-remote".
    insta::assert_debug_snapshot!(
        resolve_symbol(mut_repo, "remote@mirror").unwrap_err(), @r###"
    NoSuchRevision {
        name: "remote@mirror",
        candidates: [
            "local-remote@mirror",
            "remote@origin",
        ],
    }
    "###);

    // Typo of remote-only bookmark name
    insta::assert_debug_snapshot!(
        resolve_symbol(mut_repo, "emote").unwrap_err(), @r###"
    NoSuchRevision {
        name: "emote",
        candidates: [
            "remote-conflicted@origin",
            "remote@origin",
        ],
    }
    "###);
    insta::assert_debug_snapshot!(
        resolve_symbol(mut_repo, "emote@origin").unwrap_err(), @r###"
    NoSuchRevision {
        name: "emote@origin",
        candidates: [
            "local-remote@origin",
            "remote@origin",
        ],
    }
    "###);
    insta::assert_debug_snapshot!(
        resolve_symbol(mut_repo, "remote@origine").unwrap_err(), @r###"
    NoSuchRevision {
        name: "remote@origine",
        candidates: [
            "local-remote@origin",
            "remote-conflicted@origin",
            "remote@origin",
        ],
    }
    "###);
}

#[test]
fn test_resolve_symbol_tags() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();

    let commit1 = write_random_commit(mut_repo);
    let commit2 = write_random_commit(mut_repo);
    let commit3 = write_random_commit(mut_repo);

    mut_repo.set_tag_target("tag-bookmark", RefTarget::normal(commit1.id().clone()));
    mut_repo.set_local_bookmark_target("tag-bookmark", RefTarget::normal(commit2.id().clone()));
    mut_repo.set_git_ref_target(
        "refs/tags/unimported",
        RefTarget::normal(commit3.id().clone()),
    );

    // Tag precedes bookmark
    assert_eq!(
        resolve_symbol(mut_repo, "tag-bookmark").unwrap(),
        vec![commit1.id().clone()],
    );

    assert_matches!(
        resolve_symbol(mut_repo, "unimported"),
        Err(RevsetResolutionError::NoSuchRevision { .. })
    );

    // "@" (quoted) can be resolved, and root is a normal symbol.
    let ws_id = WorkspaceId::default();
    mut_repo
        .set_wc_commit(ws_id.clone(), commit1.id().clone())
        .unwrap();
    mut_repo.set_tag_target("@", RefTarget::normal(commit2.id().clone()));
    mut_repo.set_tag_target("root", RefTarget::normal(commit3.id().clone()));
    assert_eq!(
        resolve_symbol(mut_repo, r#""@""#).unwrap(),
        vec![commit2.id().clone()]
    );
    assert_eq!(
        resolve_symbol(mut_repo, "root").unwrap(),
        vec![commit3.id().clone()]
    );
}

#[test]
fn test_resolve_symbol_git_refs() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();

    // Create some commits and refs to work with and so the repo is not empty
    let commit1 = write_random_commit(mut_repo);
    let commit2 = write_random_commit(mut_repo);
    let commit3 = write_random_commit(mut_repo);
    let commit4 = write_random_commit(mut_repo);
    let commit5 = write_random_commit(mut_repo);
    mut_repo.set_git_ref_target(
        "refs/heads/bookmark1",
        RefTarget::normal(commit1.id().clone()),
    );
    mut_repo.set_git_ref_target(
        "refs/heads/bookmark2",
        RefTarget::normal(commit2.id().clone()),
    );
    mut_repo.set_git_ref_target(
        "refs/heads/conflicted",
        RefTarget::from_legacy_form(
            [commit2.id().clone()],
            [commit1.id().clone(), commit3.id().clone()],
        ),
    );
    mut_repo.set_git_ref_target("refs/tags/tag1", RefTarget::normal(commit2.id().clone()));
    mut_repo.set_git_ref_target(
        "refs/tags/remotes/origin/bookmark1",
        RefTarget::normal(commit3.id().clone()),
    );

    // Nonexistent ref
    assert_matches!(
        resolve_symbol(mut_repo, "nonexistent"),
        Err(RevsetResolutionError::NoSuchRevision{name, candidates})
            if name == "nonexistent" && candidates.is_empty()
    );

    // Full ref
    mut_repo.set_git_ref_target(
        "refs/heads/bookmark",
        RefTarget::normal(commit4.id().clone()),
    );
    assert_eq!(
        resolve_symbol(mut_repo, "refs/heads/bookmark").unwrap(),
        vec![commit4.id().clone()]
    );

    // Qualified with only heads/
    mut_repo.set_git_ref_target(
        "refs/heads/bookmark",
        RefTarget::normal(commit5.id().clone()),
    );
    mut_repo.set_git_ref_target(
        "refs/tags/bookmark",
        RefTarget::normal(commit4.id().clone()),
    );
    // bookmark alone is not recognized
    insta::assert_debug_snapshot!(
        resolve_symbol(mut_repo, "bookmark").unwrap_err(), @r###"
    NoSuchRevision {
        name: "bookmark",
        candidates: [],
    }
    "###);
    // heads/bookmark does get resolved to the git ref refs/heads/bookmark
    assert_eq!(
        resolve_symbol(mut_repo, "heads/bookmark").unwrap(),
        vec![commit5.id().clone()]
    );

    // Unqualified tag name
    mut_repo.set_git_ref_target("refs/tags/tag", RefTarget::normal(commit4.id().clone()));
    assert_matches!(
        resolve_symbol(mut_repo, "tag"),
        Err(RevsetResolutionError::NoSuchRevision { .. })
    );

    // Unqualified remote-tracking bookmark name
    mut_repo.set_git_ref_target(
        "refs/remotes/origin/remote-bookmark",
        RefTarget::normal(commit2.id().clone()),
    );
    assert_matches!(
        resolve_symbol(mut_repo, "origin/remote-bookmark"),
        Err(RevsetResolutionError::NoSuchRevision { .. })
    );

    // Conflicted ref resolves to its "adds"
    assert_eq!(
        resolve_symbol(mut_repo, "refs/heads/conflicted").unwrap(),
        vec![commit1.id().clone(), commit3.id().clone()]
    );
}

fn resolve_commit_ids(repo: &dyn Repo, revset_str: &str) -> Vec<CommitId> {
    try_resolve_commit_ids(repo, revset_str).unwrap()
}

fn try_resolve_commit_ids(
    repo: &dyn Repo,
    revset_str: &str,
) -> Result<Vec<CommitId>, RevsetResolutionError> {
    let settings = testutils::user_settings();
    let aliases_map = RevsetAliasesMap::default();
    let revset_extensions = RevsetExtensions::default();
    let context = RevsetParseContext::new(
        &aliases_map,
        settings.user_email(),
        chrono::Utc::now().fixed_offset().into(),
        &revset_extensions,
        None,
    );
    let expression = parse(&mut RevsetDiagnostics::new(), revset_str, &context).unwrap();
    let symbol_resolver = DefaultSymbolResolver::new(repo, revset_extensions.symbol_resolvers());
    let expression = expression.resolve_user_expression(repo, &symbol_resolver)?;
    Ok(expression
        .evaluate(repo)
        .unwrap()
        .iter()
        .map(Result::unwrap)
        .collect())
}

fn resolve_commit_ids_in_workspace(
    repo: &dyn Repo,
    revset_str: &str,
    workspace: &Workspace,
    cwd: Option<&Path>,
) -> Vec<CommitId> {
    let settings = testutils::user_settings();
    let path_converter = RepoPathUiConverter::Fs {
        cwd: cwd.unwrap_or_else(|| workspace.workspace_root()).to_owned(),
        base: workspace.workspace_root().to_owned(),
    };
    let workspace_ctx = RevsetWorkspaceContext {
        path_converter: &path_converter,
        workspace_id: workspace.workspace_id(),
    };
    let aliases_map = RevsetAliasesMap::default();
    let extensions = RevsetExtensions::default();
    let context = RevsetParseContext::new(
        &aliases_map,
        settings.user_email(),
        chrono::Utc::now().fixed_offset().into(),
        &extensions,
        Some(workspace_ctx),
    );
    let expression = parse(&mut RevsetDiagnostics::new(), revset_str, &context).unwrap();
    let symbol_resolver =
        DefaultSymbolResolver::new(repo, &([] as [&Box<dyn SymbolResolverExtension>; 0]));
    let expression = expression
        .resolve_user_expression(repo, &symbol_resolver)
        .unwrap();
    expression
        .evaluate(repo)
        .unwrap()
        .iter()
        .map(Result::unwrap)
        .collect()
}

#[test]
fn test_evaluate_expression_root_and_checkout() {
    let test_workspace = TestWorkspace::init();
    let repo = &test_workspace.repo;

    let root_operation = repo.loader().root_operation();
    let root_repo = repo.reload_at(&root_operation).unwrap();

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();

    let root_commit = repo.store().root_commit();
    let commit1 = write_random_commit(mut_repo);

    // Can find the root commit
    assert_eq!(
        resolve_commit_ids(mut_repo, "root()"),
        vec![root_commit.id().clone()]
    );

    // Can find the root commit in the root view
    assert_eq!(
        resolve_commit_ids(root_repo.as_ref(), "root()"),
        vec![root_commit.id().clone()]
    );

    // Can find the current working-copy commit
    mut_repo
        .set_wc_commit(WorkspaceId::default(), commit1.id().clone())
        .unwrap();
    assert_eq!(
        resolve_commit_ids_in_workspace(mut_repo, "@", &test_workspace.workspace, None),
        vec![commit1.id().clone()]
    );

    // Shouldn't panic by unindexed commit ID
    let expression = RevsetExpression::commit(commit1.id().clone())
        .resolve_user_expression(tx.repo(), &FailingSymbolResolver)
        .unwrap();
    assert!(expression.evaluate(tx.base_repo().as_ref()).is_err());
}

#[test]
fn test_evaluate_expression_heads() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let root_commit = repo.store().root_commit();
    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();
    let mut graph_builder = CommitGraphBuilder::new(mut_repo);
    let commit1 = graph_builder.initial_commit();
    let commit2 = graph_builder.commit_with_parents(&[&commit1]);
    let commit3 = graph_builder.commit_with_parents(&[&commit2]);
    let commit4 = graph_builder.commit_with_parents(&[&commit1]);

    // Heads of an empty set is an empty set
    assert_eq!(resolve_commit_ids(mut_repo, "heads(none())"), vec![]);

    // Heads of the root is the root
    assert_eq!(
        resolve_commit_ids(mut_repo, "heads(root())"),
        vec![root_commit.id().clone()]
    );

    // Heads of a single commit is that commit
    assert_eq!(
        resolve_commit_ids(mut_repo, &format!("heads({})", commit2.id())),
        vec![commit2.id().clone()]
    );

    // Heads of a parent and a child is the child
    assert_eq!(
        resolve_commit_ids(
            mut_repo,
            &format!("heads({} | {})", commit2.id(), commit3.id())
        ),
        vec![commit3.id().clone()]
    );

    // Heads of a grandparent and a grandchild is the grandchild (unlike Mercurial's
    // heads() revset, which would include both)
    assert_eq!(
        resolve_commit_ids(
            mut_repo,
            &format!("heads({} | {})", commit1.id(), commit3.id())
        ),
        vec![commit3.id().clone()]
    );

    // Heads should be sorted in reverse index position order
    assert_eq!(
        resolve_commit_ids(
            mut_repo,
            &format!("heads({} | {})", commit3.id(), commit4.id())
        ),
        vec![commit4.id().clone(), commit3.id().clone()]
    );

    // Heads of all commits is the set of visible heads in the repo
    assert_eq!(
        resolve_commit_ids(mut_repo, "heads(all())"),
        resolve_commit_ids(mut_repo, "visible_heads()")
    );
}

#[test]
fn test_evaluate_expression_roots() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let root_commit = repo.store().root_commit();
    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();
    let mut graph_builder = CommitGraphBuilder::new(mut_repo);
    let commit1 = graph_builder.initial_commit();
    let commit2 = graph_builder.commit_with_parents(&[&commit1]);
    let commit3 = graph_builder.commit_with_parents(&[&commit2]);

    // Roots of an empty set is an empty set
    assert_eq!(resolve_commit_ids(mut_repo, "roots(none())"), vec![]);

    // Roots of the root is the root
    assert_eq!(
        resolve_commit_ids(mut_repo, "roots(root())"),
        vec![root_commit.id().clone()]
    );

    // Roots of a single commit is that commit
    assert_eq!(
        resolve_commit_ids(mut_repo, &format!("roots({})", commit2.id())),
        vec![commit2.id().clone()]
    );

    // Roots of a parent and a child is the parent
    assert_eq!(
        resolve_commit_ids(
            mut_repo,
            &format!("roots({} | {})", commit2.id(), commit3.id())
        ),
        vec![commit2.id().clone()]
    );

    // Roots of a grandparent and a grandchild is the grandparent (unlike
    // Mercurial's roots() revset, which would include both)
    assert_eq!(
        resolve_commit_ids(
            mut_repo,
            &format!("roots({} | {})", commit1.id(), commit3.id())
        ),
        vec![commit1.id().clone()]
    );

    // Roots of all commits is the root commit
    assert_eq!(
        resolve_commit_ids(mut_repo, "roots(all())"),
        vec![root_commit.id().clone()]
    );
}

#[test]
fn test_evaluate_expression_parents() {
    let test_workspace = TestWorkspace::init();
    let repo = &test_workspace.repo;

    let root_commit = repo.store().root_commit();
    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();
    let mut graph_builder = CommitGraphBuilder::new(mut_repo);
    let commit1 = graph_builder.initial_commit();
    let commit2 = graph_builder.commit_with_parents(&[&commit1]);
    let commit3 = graph_builder.initial_commit();
    let commit4 = graph_builder.commit_with_parents(&[&commit2, &commit3]);
    let commit5 = graph_builder.commit_with_parents(&[&commit2]);

    // The root commit has no parents
    assert_eq!(resolve_commit_ids(mut_repo, "root()-"), vec![]);

    // Can find parents of the current working-copy commit
    mut_repo
        .set_wc_commit(WorkspaceId::default(), commit2.id().clone())
        .unwrap();
    assert_eq!(
        resolve_commit_ids_in_workspace(mut_repo, "@-", &test_workspace.workspace, None,),
        vec![commit1.id().clone()]
    );

    // Can find parents of a merge commit
    assert_eq!(
        resolve_commit_ids(mut_repo, &format!("{}-", commit4.id())),
        vec![commit3.id().clone(), commit2.id().clone()]
    );

    // Parents of all commits in input are returned
    assert_eq!(
        resolve_commit_ids(mut_repo, &format!("({} | {})-", commit2.id(), commit3.id())),
        vec![commit1.id().clone(), root_commit.id().clone()]
    );

    // Parents already in input set are returned
    assert_eq!(
        resolve_commit_ids(mut_repo, &format!("({} | {})-", commit1.id(), commit2.id())),
        vec![commit1.id().clone(), root_commit.id().clone()]
    );

    // Parents shared among commits in input are not repeated
    assert_eq!(
        resolve_commit_ids(mut_repo, &format!("({} | {})-", commit4.id(), commit5.id())),
        vec![commit3.id().clone(), commit2.id().clone()]
    );

    // Can find parents of parents, which may be optimized to single query
    assert_eq!(
        resolve_commit_ids(mut_repo, &format!("{}--", commit4.id())),
        vec![commit1.id().clone(), root_commit.id().clone()]
    );
    assert_eq!(
        resolve_commit_ids(
            mut_repo,
            &format!("({} | {})--", commit4.id(), commit5.id())
        ),
        vec![commit1.id().clone(), root_commit.id().clone()]
    );
    assert_eq!(
        resolve_commit_ids(
            mut_repo,
            &format!("({} | {})--", commit4.id(), commit2.id())
        ),
        vec![commit1.id().clone(), root_commit.id().clone()]
    );
}

#[test]
fn test_evaluate_expression_children() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();

    let commit1 = write_random_commit(mut_repo);
    let commit2 = create_random_commit(mut_repo)
        .set_parents(vec![commit1.id().clone()])
        .write()
        .unwrap();
    let commit3 = create_random_commit(mut_repo)
        .set_parents(vec![commit2.id().clone()])
        .write()
        .unwrap();
    let commit4 = create_random_commit(mut_repo)
        .set_parents(vec![commit1.id().clone()])
        .write()
        .unwrap();
    let commit5 = create_random_commit(mut_repo)
        .set_parents(vec![commit3.id().clone(), commit4.id().clone()])
        .write()
        .unwrap();
    let commit6 = create_random_commit(mut_repo)
        .set_parents(vec![commit5.id().clone()])
        .write()
        .unwrap();

    // Can find children of the root commit
    assert_eq!(
        resolve_commit_ids(mut_repo, "root()+"),
        vec![commit1.id().clone()]
    );

    // Children of all commits in input are returned, including those already in the
    // input set
    assert_eq!(
        resolve_commit_ids(mut_repo, &format!("({} | {})+", commit1.id(), commit2.id())),
        vec![
            commit4.id().clone(),
            commit3.id().clone(),
            commit2.id().clone()
        ]
    );

    // Children shared among commits in input are not repeated
    assert_eq!(
        resolve_commit_ids(mut_repo, &format!("({} | {})+", commit3.id(), commit4.id())),
        vec![commit5.id().clone()]
    );

    // Can find children of children, which may be optimized to single query
    assert_eq!(
        resolve_commit_ids(mut_repo, "root()++"),
        vec![commit4.id().clone(), commit2.id().clone()]
    );
    assert_eq!(
        resolve_commit_ids(mut_repo, &format!("(root() | {})++", commit1.id())),
        vec![
            commit5.id().clone(),
            commit4.id().clone(),
            commit3.id().clone(),
            commit2.id().clone(),
        ]
    );
    assert_eq!(
        resolve_commit_ids(
            mut_repo,
            &format!("({} | {})++", commit4.id(), commit2.id())
        ),
        vec![commit6.id().clone(), commit5.id().clone()]
    );

    // Empty root
    assert_eq!(resolve_commit_ids(mut_repo, "none()+"), vec![]);
}

#[test]
fn test_evaluate_expression_ancestors() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let root_commit = repo.store().root_commit();
    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();
    let mut graph_builder = CommitGraphBuilder::new(mut_repo);
    let commit1 = graph_builder.initial_commit();
    let commit2 = graph_builder.commit_with_parents(&[&commit1]);
    let commit3 = graph_builder.commit_with_parents(&[&commit2]);
    let commit4 = graph_builder.commit_with_parents(&[&commit1, &commit3]);

    // The ancestors of the root commit is just the root commit itself
    assert_eq!(
        resolve_commit_ids(mut_repo, "::root()"),
        vec![root_commit.id().clone()]
    );

    // Can find ancestors of a specific commit. Commits reachable via multiple paths
    // are not repeated.
    assert_eq!(
        resolve_commit_ids(mut_repo, &format!("::{}", commit4.id())),
        vec![
            commit4.id().clone(),
            commit3.id().clone(),
            commit2.id().clone(),
            commit1.id().clone(),
            root_commit.id().clone(),
        ]
    );

    // Can find ancestors of parents or parents of ancestors, which may be optimized
    // to single query
    assert_eq!(
        resolve_commit_ids(mut_repo, &format!("::({}-)", commit4.id())),
        vec![
            commit3.id().clone(),
            commit2.id().clone(),
            commit1.id().clone(),
            root_commit.id().clone(),
        ]
    );
    assert_eq!(
        resolve_commit_ids(
            mut_repo,
            &format!("(::({}|{}))-", commit3.id(), commit2.id()),
        ),
        vec![
            commit2.id().clone(),
            commit1.id().clone(),
            root_commit.id().clone(),
        ]
    );
    assert_eq!(
        resolve_commit_ids(
            mut_repo,
            &format!("::(({}|{})-)", commit3.id(), commit2.id()),
        ),
        vec![
            commit2.id().clone(),
            commit1.id().clone(),
            root_commit.id().clone(),
        ]
    );

    // Can find last n ancestors of a commit
    assert_eq!(
        resolve_commit_ids(mut_repo, &format!("ancestors({}, 0)", commit2.id())),
        vec![]
    );
    assert_eq!(
        resolve_commit_ids(mut_repo, &format!("ancestors({}, 1)", commit3.id())),
        vec![commit3.id().clone()]
    );
    assert_eq!(
        resolve_commit_ids(mut_repo, &format!("ancestors({}, 3)", commit3.id())),
        vec![
            commit3.id().clone(),
            commit2.id().clone(),
            commit1.id().clone(),
        ]
    );
}

#[test]
fn test_evaluate_expression_range() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();
    let mut graph_builder = CommitGraphBuilder::new(mut_repo);
    let commit1 = graph_builder.initial_commit();
    let commit2 = graph_builder.commit_with_parents(&[&commit1]);
    let commit3 = graph_builder.commit_with_parents(&[&commit2]);
    let commit4 = graph_builder.commit_with_parents(&[&commit1, &commit3]);

    // The range from the root to the root is empty (because the left side of the
    // range is exclusive)
    assert_eq!(resolve_commit_ids(mut_repo, "root()..root()"), vec![]);

    // Linear range
    assert_eq!(
        resolve_commit_ids(mut_repo, &format!("{}..{}", commit1.id(), commit3.id())),
        vec![commit3.id().clone(), commit2.id().clone()]
    );

    // Empty range (descendant first)
    assert_eq!(
        resolve_commit_ids(mut_repo, &format!("{}..{}", commit3.id(), commit1.id())),
        vec![]
    );

    // Range including a merge
    assert_eq!(
        resolve_commit_ids(mut_repo, &format!("{}..{}", commit1.id(), commit4.id())),
        vec![
            commit4.id().clone(),
            commit3.id().clone(),
            commit2.id().clone()
        ]
    );

    // Range including merge ancestors: commit4-- == root | commit2
    assert_eq!(
        resolve_commit_ids(mut_repo, &format!("{}--..{}", commit4.id(), commit3.id())),
        vec![commit3.id().clone()]
    );

    // Sibling commits
    assert_eq!(
        resolve_commit_ids(mut_repo, &format!("{}..{}", commit2.id(), commit3.id())),
        vec![commit3.id().clone()]
    );

    // Left operand defaults to root()
    assert_eq!(
        resolve_commit_ids(mut_repo, &format!("..{}", commit2.id())),
        vec![commit2.id().clone(), commit1.id().clone()]
    );

    // Right operand defaults to visible_heads()
    assert_eq!(
        resolve_commit_ids(mut_repo, &format!("{}..", commit2.id())),
        vec![commit4.id().clone(), commit3.id().clone()]
    );

    assert_eq!(
        resolve_commit_ids(mut_repo, ".."),
        vec![
            commit4.id().clone(),
            commit3.id().clone(),
            commit2.id().clone(),
            commit1.id().clone(),
        ]
    );
}

#[test]
fn test_evaluate_expression_dag_range() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let root_commit_id = repo.store().root_commit_id().clone();
    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();
    let mut graph_builder = CommitGraphBuilder::new(mut_repo);
    let commit1 = graph_builder.initial_commit();
    let commit2 = graph_builder.commit_with_parents(&[&commit1]);
    let commit3 = graph_builder.commit_with_parents(&[&commit2]);
    let commit4 = graph_builder.commit_with_parents(&[&commit1]);
    let commit5 = graph_builder.commit_with_parents(&[&commit3, &commit4]);

    // Can get DAG range of just the root commit
    assert_eq!(
        resolve_commit_ids(mut_repo, "root()::root()"),
        vec![root_commit_id.clone()]
    );

    // Linear range
    assert_eq!(
        resolve_commit_ids(mut_repo, &format!("{}::{}", root_commit_id, commit2.id())),
        vec![
            commit2.id().clone(),
            commit1.id().clone(),
            root_commit_id.clone(),
        ]
    );

    // Empty range
    assert_eq!(
        resolve_commit_ids(mut_repo, &format!("{}::{}", commit2.id(), commit4.id())),
        vec![]
    );

    // Empty root
    assert_eq!(
        resolve_commit_ids(mut_repo, &format!("none()::{}", commit5.id())),
        vec![],
    );

    // Multiple root, commit1 shouldn't be hidden by commit2
    assert_eq!(
        resolve_commit_ids(
            mut_repo,
            &format!("({}|{})::{}", commit1.id(), commit2.id(), commit3.id())
        ),
        vec![
            commit3.id().clone(),
            commit2.id().clone(),
            commit1.id().clone()
        ]
    );

    // Including a merge
    assert_eq!(
        resolve_commit_ids(mut_repo, &format!("{}::{}", commit1.id(), commit5.id())),
        vec![
            commit5.id().clone(),
            commit4.id().clone(),
            commit3.id().clone(),
            commit2.id().clone(),
            commit1.id().clone(),
        ]
    );

    // Including a merge, but ancestors only from one side
    assert_eq!(
        resolve_commit_ids(mut_repo, &format!("{}::{}", commit2.id(), commit5.id())),
        vec![
            commit5.id().clone(),
            commit3.id().clone(),
            commit2.id().clone(),
        ]
    );

    // Full range meaning all()
    assert_eq!(
        resolve_commit_ids(mut_repo, "::"),
        vec![
            commit5.id().clone(),
            commit4.id().clone(),
            commit3.id().clone(),
            commit2.id().clone(),
            commit1.id().clone(),
            root_commit_id.clone(),
        ]
    );
}

#[test]
fn test_evaluate_expression_connected() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let root_commit_id = repo.store().root_commit_id().clone();
    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();
    let mut graph_builder = CommitGraphBuilder::new(mut_repo);
    let commit1 = graph_builder.initial_commit();
    let commit2 = graph_builder.commit_with_parents(&[&commit1]);
    let commit3 = graph_builder.commit_with_parents(&[&commit2]);
    let commit4 = graph_builder.commit_with_parents(&[&commit1]);
    let commit5 = graph_builder.commit_with_parents(&[&commit3, &commit4]);

    // Connecting an empty set yields an empty set
    assert_eq!(resolve_commit_ids(mut_repo, "connected(none())"), vec![]);

    // Can connect just the root commit
    assert_eq!(
        resolve_commit_ids(mut_repo, "connected(root())"),
        vec![root_commit_id.clone()]
    );

    // Can connect linearly
    assert_eq!(
        resolve_commit_ids(
            mut_repo,
            &format!("connected({} | {})", root_commit_id, commit2.id())
        ),
        vec![commit2.id().clone(), commit1.id().clone(), root_commit_id]
    );

    // Siblings don't get connected
    assert_eq!(
        resolve_commit_ids(
            mut_repo,
            &format!("connected({} | {})", commit2.id(), commit4.id())
        ),
        vec![commit4.id().clone(), commit2.id().clone()]
    );

    // Including a merge
    assert_eq!(
        resolve_commit_ids(
            mut_repo,
            &format!("connected({} | {})", commit1.id(), commit5.id())
        ),
        vec![
            commit5.id().clone(),
            commit4.id().clone(),
            commit3.id().clone(),
            commit2.id().clone(),
            commit1.id().clone(),
        ]
    );

    // Including a merge, but ancestors only from one side
    assert_eq!(
        resolve_commit_ids(
            mut_repo,
            &format!("connected({} | {})", commit2.id(), commit5.id())
        ),
        vec![
            commit5.id().clone(),
            commit3.id().clone(),
            commit2.id().clone(),
        ]
    );
}

#[test]
fn test_evaluate_expression_reachable() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();

    // Construct 3 separate subgraphs off the root commit.
    // 1 is a chain, 2 is a merge, 3 is a pyramidal monstrosity
    let graph1commit1 = write_random_commit(mut_repo);
    let graph1commit2 = create_random_commit(mut_repo)
        .set_parents(vec![graph1commit1.id().clone()])
        .write()
        .unwrap();
    let graph1commit3 = create_random_commit(mut_repo)
        .set_parents(vec![graph1commit2.id().clone()])
        .write()
        .unwrap();
    let graph2commit1 = write_random_commit(mut_repo);
    let graph2commit2 = write_random_commit(mut_repo);
    let graph2commit3 = create_random_commit(mut_repo)
        .set_parents(vec![graph2commit1.id().clone(), graph2commit2.id().clone()])
        .write()
        .unwrap();
    let graph3commit1 = write_random_commit(mut_repo);
    let graph3commit2 = write_random_commit(mut_repo);
    let graph3commit3 = write_random_commit(mut_repo);
    let graph3commit4 = create_random_commit(mut_repo)
        .set_parents(vec![graph3commit1.id().clone(), graph3commit2.id().clone()])
        .write()
        .unwrap();
    let graph3commit5 = create_random_commit(mut_repo)
        .set_parents(vec![graph3commit2.id().clone(), graph3commit3.id().clone()])
        .write()
        .unwrap();
    let graph3commit6 = create_random_commit(mut_repo)
        .set_parents(vec![graph3commit3.id().clone()])
        .write()
        .unwrap();
    let graph3commit7 = create_random_commit(mut_repo)
        .set_parents(vec![graph3commit4.id().clone(), graph3commit5.id().clone()])
        .write()
        .unwrap();

    // Domain is respected.
    assert_eq!(
        resolve_commit_ids(
            mut_repo,
            &format!(
                "reachable({}, all() ~ ::{})",
                graph1commit2.id(),
                graph1commit1.id()
            )
        ),
        vec![graph1commit3.id().clone(), graph1commit2.id().clone(),]
    );
    assert_eq!(
        resolve_commit_ids(
            mut_repo,
            &format!(
                "reachable({}, all() ~ ::{})",
                graph1commit2.id(),
                graph1commit3.id()
            )
        ),
        vec![]
    );

    // Each graph is identifiable from any node in it.
    for (i, commit) in [&graph1commit1, &graph1commit2, &graph1commit3]
        .iter()
        .enumerate()
    {
        assert_eq!(
            resolve_commit_ids(
                mut_repo,
                &format!("reachable({}, all() ~ root())", commit.id())
            ),
            vec![
                graph1commit3.id().clone(),
                graph1commit2.id().clone(),
                graph1commit1.id().clone(),
            ],
            "commit {}",
            i + 1
        );
    }

    for (i, commit) in [&graph2commit1, &graph2commit2, &graph2commit3]
        .iter()
        .enumerate()
    {
        assert_eq!(
            resolve_commit_ids(
                mut_repo,
                &format!("reachable({}, all() ~ root())", commit.id())
            ),
            vec![
                graph2commit3.id().clone(),
                graph2commit2.id().clone(),
                graph2commit1.id().clone(),
            ],
            "commit {}",
            i + 1
        );
    }

    for (i, commit) in [
        &graph3commit1,
        &graph3commit2,
        &graph3commit3,
        &graph3commit4,
        &graph3commit5,
        &graph3commit6,
        &graph3commit7,
    ]
    .iter()
    .enumerate()
    {
        assert_eq!(
            resolve_commit_ids(
                mut_repo,
                &format!("reachable({}, all() ~ root())", commit.id())
            ),
            vec![
                graph3commit7.id().clone(),
                graph3commit6.id().clone(),
                graph3commit5.id().clone(),
                graph3commit4.id().clone(),
                graph3commit3.id().clone(),
                graph3commit2.id().clone(),
                graph3commit1.id().clone(),
            ],
            "commit {}",
            i + 1
        );
    }

    // Test a split of the pyramidal monstrosity.
    assert_eq!(
        resolve_commit_ids(
            mut_repo,
            &format!(
                "reachable({}, all() ~ ::{})",
                graph3commit4.id(),
                graph3commit5.id()
            )
        ),
        vec![
            graph3commit7.id().clone(),
            graph3commit4.id().clone(),
            graph3commit1.id().clone(),
        ]
    );
}

#[test]
fn test_evaluate_expression_descendants() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();

    let root_commit_id = repo.store().root_commit_id().clone();
    let commit1 = write_random_commit(mut_repo);
    let commit2 = create_random_commit(mut_repo)
        .set_parents(vec![commit1.id().clone()])
        .write()
        .unwrap();
    let commit3 = create_random_commit(mut_repo)
        .set_parents(vec![commit2.id().clone()])
        .write()
        .unwrap();
    let commit4 = create_random_commit(mut_repo)
        .set_parents(vec![commit1.id().clone()])
        .write()
        .unwrap();
    let commit5 = create_random_commit(mut_repo)
        .set_parents(vec![commit3.id().clone(), commit4.id().clone()])
        .write()
        .unwrap();
    let commit6 = create_random_commit(mut_repo)
        .set_parents(vec![commit5.id().clone()])
        .write()
        .unwrap();

    // The descendants of the root commit are all the commits in the repo
    assert_eq!(
        resolve_commit_ids(mut_repo, "root()::"),
        vec![
            commit6.id().clone(),
            commit5.id().clone(),
            commit4.id().clone(),
            commit3.id().clone(),
            commit2.id().clone(),
            commit1.id().clone(),
            root_commit_id,
        ]
    );

    // Can find descendants of a specific commit
    assert_eq!(
        resolve_commit_ids(mut_repo, &format!("{}::", commit2.id())),
        vec![
            commit6.id().clone(),
            commit5.id().clone(),
            commit3.id().clone(),
            commit2.id().clone(),
        ]
    );

    // Can find descendants of children or children of descendants, which may be
    // optimized to single query
    assert_eq!(
        resolve_commit_ids(mut_repo, &format!("({}+)::", commit1.id())),
        vec![
            commit6.id().clone(),
            commit5.id().clone(),
            commit4.id().clone(),
            commit3.id().clone(),
            commit2.id().clone(),
        ]
    );
    assert_eq!(
        resolve_commit_ids(mut_repo, &format!("({}++)::", commit1.id())),
        vec![
            commit6.id().clone(),
            commit5.id().clone(),
            commit3.id().clone(),
        ]
    );
    assert_eq!(
        resolve_commit_ids(
            mut_repo,
            &format!("(({}|{})::)+", commit4.id(), commit2.id()),
        ),
        vec![
            commit6.id().clone(),
            commit5.id().clone(),
            commit3.id().clone(),
        ]
    );
    assert_eq!(
        resolve_commit_ids(
            mut_repo,
            &format!("(({}|{})+)::", commit4.id(), commit2.id()),
        ),
        vec![
            commit6.id().clone(),
            commit5.id().clone(),
            commit3.id().clone(),
        ]
    );

    // Can find next n descendants of a commit
    assert_eq!(
        resolve_commit_ids(mut_repo, &format!("descendants({}, 0)", commit2.id())),
        vec![]
    );
    assert_eq!(
        resolve_commit_ids(mut_repo, &format!("descendants({}, 1)", commit3.id())),
        vec![commit3.id().clone()]
    );
    assert_eq!(
        resolve_commit_ids(mut_repo, &format!("descendants({}, 3)", commit3.id())),
        vec![
            commit6.id().clone(),
            commit5.id().clone(),
            commit3.id().clone(),
        ]
    );
}

#[test]
fn test_evaluate_expression_none() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    // none() is empty (doesn't include the checkout, for example)
    assert_eq!(resolve_commit_ids(repo.as_ref(), "none()"), vec![]);
}

#[test]
fn test_evaluate_expression_all() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();
    let root_commit_id = repo.store().root_commit_id().clone();
    let mut graph_builder = CommitGraphBuilder::new(mut_repo);
    let commit1 = graph_builder.initial_commit();
    let commit2 = graph_builder.commit_with_parents(&[&commit1]);
    let commit3 = graph_builder.commit_with_parents(&[&commit1]);
    let commit4 = graph_builder.commit_with_parents(&[&commit2, &commit3]);

    assert_eq!(
        resolve_commit_ids(mut_repo, "all()"),
        vec![
            commit4.id().clone(),
            commit3.id().clone(),
            commit2.id().clone(),
            commit1.id().clone(),
            root_commit_id,
        ]
    );
}

#[test]
fn test_evaluate_expression_visible_heads() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();
    let mut graph_builder = CommitGraphBuilder::new(mut_repo);
    let commit1 = graph_builder.initial_commit();
    let commit2 = graph_builder.commit_with_parents(&[&commit1]);
    let commit3 = graph_builder.commit_with_parents(&[&commit1]);

    assert_eq!(
        resolve_commit_ids(mut_repo, "visible_heads()"),
        vec![commit3.id().clone(), commit2.id().clone()]
    );
}

#[test]
fn test_evaluate_expression_git_refs() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();

    let commit1 = write_random_commit(mut_repo);
    let commit2 = write_random_commit(mut_repo);
    let commit3 = write_random_commit(mut_repo);
    let commit4 = write_random_commit(mut_repo);

    // Can get git refs when there are none
    assert_eq!(resolve_commit_ids(mut_repo, "git_refs()"), vec![]);
    // Can get a mix of git refs
    mut_repo.set_git_ref_target(
        "refs/heads/bookmark1",
        RefTarget::normal(commit1.id().clone()),
    );
    mut_repo.set_git_ref_target("refs/tags/tag1", RefTarget::normal(commit2.id().clone()));
    assert_eq!(
        resolve_commit_ids(mut_repo, "git_refs()"),
        vec![commit2.id().clone(), commit1.id().clone()]
    );
    // Two refs pointing to the same commit does not result in a duplicate in the
    // revset
    mut_repo.set_git_ref_target("refs/tags/tag2", RefTarget::normal(commit2.id().clone()));
    assert_eq!(
        resolve_commit_ids(mut_repo, "git_refs()"),
        vec![commit2.id().clone(), commit1.id().clone()]
    );
    // Can get git refs when there are conflicted refs
    mut_repo.set_git_ref_target(
        "refs/heads/bookmark1",
        RefTarget::from_legacy_form(
            [commit1.id().clone()],
            [commit2.id().clone(), commit3.id().clone()],
        ),
    );
    mut_repo.set_git_ref_target(
        "refs/tags/tag1",
        RefTarget::from_legacy_form(
            [commit2.id().clone()],
            [commit3.id().clone(), commit4.id().clone()],
        ),
    );
    mut_repo.set_git_ref_target("refs/tags/tag2", RefTarget::absent());
    assert_eq!(
        resolve_commit_ids(mut_repo, "git_refs()"),
        vec![
            commit4.id().clone(),
            commit3.id().clone(),
            commit2.id().clone()
        ]
    );
}

#[test]
fn test_evaluate_expression_git_head() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();

    let commit1 = write_random_commit(mut_repo);

    // Can get git head when it's not set
    assert_eq!(resolve_commit_ids(mut_repo, "git_head()"), vec![]);
    mut_repo.set_git_head_target(RefTarget::normal(commit1.id().clone()));
    assert_eq!(
        resolve_commit_ids(mut_repo, "git_head()"),
        vec![commit1.id().clone()]
    );
}

#[test]
fn test_evaluate_expression_bookmarks() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();

    let commit1 = write_random_commit(mut_repo);
    let commit2 = write_random_commit(mut_repo);
    let commit3 = write_random_commit(mut_repo);
    let commit4 = write_random_commit(mut_repo);

    // Can get bookmarks when there are none
    assert_eq!(resolve_commit_ids(mut_repo, "bookmarks()"), vec![]);
    // Can get a few bookmarks
    mut_repo.set_local_bookmark_target("bookmark1", RefTarget::normal(commit1.id().clone()));
    mut_repo.set_local_bookmark_target("bookmark2", RefTarget::normal(commit2.id().clone()));
    assert_eq!(
        resolve_commit_ids(mut_repo, "bookmarks()"),
        vec![commit2.id().clone(), commit1.id().clone()]
    );
    // Can get bookmarks with matching names
    assert_eq!(
        resolve_commit_ids(mut_repo, "bookmarks(bookmark1)"),
        vec![commit1.id().clone()]
    );
    assert_eq!(
        resolve_commit_ids(mut_repo, "bookmarks(bookmark)"),
        vec![commit2.id().clone(), commit1.id().clone()]
    );
    assert_eq!(
        resolve_commit_ids(mut_repo, "bookmarks(exact:bookmark1)"),
        vec![commit1.id().clone()]
    );
    assert_eq!(
        resolve_commit_ids(mut_repo, r#"bookmarks(glob:"Bookmark?")"#),
        vec![]
    );
    assert_eq!(
        resolve_commit_ids(mut_repo, r#"bookmarks(glob-i:"Bookmark?")"#),
        vec![commit2.id().clone(), commit1.id().clone()]
    );
    assert_eq!(
        resolve_commit_ids(mut_repo, "bookmarks(regex:'ookmark')"),
        vec![commit2.id().clone(), commit1.id().clone()]
    );
    assert_eq!(
        resolve_commit_ids(mut_repo, "bookmarks(regex:'^[Bb]ookmark1$')"),
        vec![commit1.id().clone()]
    );
    // Can silently resolve to an empty set if there's no matches
    assert_eq!(resolve_commit_ids(mut_repo, "bookmarks(bookmark3)"), vec![]);
    assert_eq!(
        resolve_commit_ids(mut_repo, "bookmarks(exact:ookmark1)"),
        vec![]
    );
    // Two bookmarks pointing to the same commit does not result in a duplicate in
    // the revset
    mut_repo.set_local_bookmark_target("bookmark3", RefTarget::normal(commit2.id().clone()));
    assert_eq!(
        resolve_commit_ids(mut_repo, "bookmarks()"),
        vec![commit2.id().clone(), commit1.id().clone()]
    );
    // Can get bookmarks when there are conflicted refs
    mut_repo.set_local_bookmark_target(
        "bookmark1",
        RefTarget::from_legacy_form(
            [commit1.id().clone()],
            [commit2.id().clone(), commit3.id().clone()],
        ),
    );
    mut_repo.set_local_bookmark_target(
        "bookmark2",
        RefTarget::from_legacy_form(
            [commit2.id().clone()],
            [commit3.id().clone(), commit4.id().clone()],
        ),
    );
    mut_repo.set_local_bookmark_target("bookmark3", RefTarget::absent());
    assert_eq!(
        resolve_commit_ids(mut_repo, "bookmarks()"),
        vec![
            commit4.id().clone(),
            commit3.id().clone(),
            commit2.id().clone()
        ]
    );
}

#[test]
fn test_evaluate_expression_remote_bookmarks() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;
    let tracking_remote_ref = |target| RemoteRef {
        target,
        state: RemoteRefState::Tracking,
    };
    let normal_tracking_remote_ref =
        |id: &CommitId| tracking_remote_ref(RefTarget::normal(id.clone()));

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();

    let commit1 = write_random_commit(mut_repo);
    let commit2 = write_random_commit(mut_repo);
    let commit3 = write_random_commit(mut_repo);
    let commit4 = write_random_commit(mut_repo);
    let commit_git_remote = write_random_commit(mut_repo);

    // Can get bookmarks when there are none
    assert_eq!(resolve_commit_ids(mut_repo, "remote_bookmarks()"), vec![]);
    // Bookmark 1 is untracked on remote origin
    mut_repo.set_remote_bookmark(
        "bookmark1",
        "origin",
        RemoteRef {
            target: RefTarget::normal(commit1.id().clone()),
            state: RemoteRefState::New,
        },
    );
    // Bookmark 2 is tracked on remote private
    mut_repo.set_remote_bookmark(
        "bookmark2",
        "private",
        normal_tracking_remote_ref(commit2.id()),
    );
    // Git-tracking bookmarks aren't included
    mut_repo.set_remote_bookmark(
        "bookmark",
        git::REMOTE_NAME_FOR_LOCAL_GIT_REPO,
        normal_tracking_remote_ref(commit_git_remote.id()),
    );
    // Can get a few bookmarks
    assert_eq!(
        resolve_commit_ids(mut_repo, "remote_bookmarks()"),
        vec![commit2.id().clone(), commit1.id().clone()]
    );
    // Can get bookmarks with matching names
    assert_eq!(
        resolve_commit_ids(mut_repo, "remote_bookmarks(bookmark1)"),
        vec![commit1.id().clone()]
    );
    assert_eq!(
        resolve_commit_ids(mut_repo, "remote_bookmarks(bookmark)"),
        vec![commit2.id().clone(), commit1.id().clone()]
    );
    assert_eq!(
        resolve_commit_ids(mut_repo, "remote_bookmarks(exact:bookmark1)"),
        vec![commit1.id().clone()]
    );
    // Can get bookmarks from matching remotes
    assert_eq!(
        resolve_commit_ids(mut_repo, r#"remote_bookmarks("", origin)"#),
        vec![commit1.id().clone()]
    );
    assert_eq!(
        resolve_commit_ids(mut_repo, r#"remote_bookmarks("", ri)"#),
        vec![commit2.id().clone(), commit1.id().clone()]
    );
    assert_eq!(
        resolve_commit_ids(mut_repo, r#"remote_bookmarks("", exact:origin)"#),
        vec![commit1.id().clone()]
    );
    // Can get bookmarks with matching names from matching remotes
    assert_eq!(
        resolve_commit_ids(mut_repo, "remote_bookmarks(bookmark1, ri)"),
        vec![commit1.id().clone()]
    );
    assert_eq!(
        resolve_commit_ids(mut_repo, r#"remote_bookmarks(bookmark, private)"#),
        vec![commit2.id().clone()]
    );
    assert_eq!(
        resolve_commit_ids(
            mut_repo,
            r#"remote_bookmarks(exact:bookmark1, exact:origin)"#
        ),
        vec![commit1.id().clone()]
    );
    // Can filter bookmarks by tracked and untracked
    assert_eq!(
        resolve_commit_ids(mut_repo, "tracked_remote_bookmarks()"),
        vec![commit2.id().clone()]
    );
    assert_eq!(
        resolve_commit_ids(mut_repo, "untracked_remote_bookmarks()"),
        vec![commit1.id().clone()]
    );
    assert_eq!(
        resolve_commit_ids(mut_repo, "untracked_remote_bookmarks(bookmark1, origin)"),
        vec![commit1.id().clone()]
    );
    assert_eq!(
        resolve_commit_ids(mut_repo, "tracked_remote_bookmarks(bookmark2, private)"),
        vec![commit2.id().clone()]
    );
    // Can silently resolve to an empty set if there's no matches
    assert_eq!(
        resolve_commit_ids(mut_repo, "remote_bookmarks(bookmark3)"),
        vec![]
    );
    assert_eq!(
        resolve_commit_ids(mut_repo, r#"remote_bookmarks("", upstream)"#),
        vec![]
    );
    assert_eq!(
        resolve_commit_ids(mut_repo, r#"remote_bookmarks(bookmark1, private)"#),
        vec![]
    );
    assert_eq!(
        resolve_commit_ids(mut_repo, r#"remote_bookmarks(exact:ranch1, exact:origin)"#),
        vec![]
    );
    assert_eq!(
        resolve_commit_ids(mut_repo, r#"remote_bookmarks(exact:bookmark1, exact:orig)"#),
        vec![]
    );
    assert_eq!(
        resolve_commit_ids(mut_repo, "tracked_remote_bookmarks(bookmark1)"),
        vec![]
    );
    assert_eq!(
        resolve_commit_ids(mut_repo, "untracked_remote_bookmarks(bookmark2)"),
        vec![]
    );
    // Two bookmarks pointing to the same commit does not result in a duplicate in
    // the revset
    mut_repo.set_remote_bookmark(
        "bookmark3",
        "origin",
        normal_tracking_remote_ref(commit2.id()),
    );
    assert_eq!(
        resolve_commit_ids(mut_repo, "remote_bookmarks()"),
        vec![commit2.id().clone(), commit1.id().clone()]
    );
    // The commits don't have to be in the current set of heads to be included.
    mut_repo.remove_head(commit2.id());
    assert_eq!(
        resolve_commit_ids(mut_repo, "remote_bookmarks()"),
        vec![commit2.id().clone(), commit1.id().clone()]
    );
    // Can get bookmarks when there are conflicted refs
    mut_repo.set_remote_bookmark(
        "bookmark1",
        "origin",
        tracking_remote_ref(RefTarget::from_legacy_form(
            [commit1.id().clone()],
            [commit2.id().clone(), commit3.id().clone()],
        )),
    );
    mut_repo.set_remote_bookmark(
        "bookmark2",
        "private",
        tracking_remote_ref(RefTarget::from_legacy_form(
            [commit2.id().clone()],
            [commit3.id().clone(), commit4.id().clone()],
        )),
    );
    mut_repo.set_remote_bookmark("bookmark3", "origin", RemoteRef::absent());
    assert_eq!(
        resolve_commit_ids(mut_repo, "remote_bookmarks()"),
        vec![
            commit4.id().clone(),
            commit3.id().clone(),
            commit2.id().clone()
        ]
    );
}

#[test]
fn test_evaluate_expression_tags() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();

    let commit1 = write_random_commit(mut_repo);
    let commit2 = write_random_commit(mut_repo);
    let commit3 = write_random_commit(mut_repo);
    let commit4 = write_random_commit(mut_repo);

    // Can get tags when there are none
    assert_eq!(resolve_commit_ids(mut_repo, "tags()"), vec![]);
    // Can get a few tags
    mut_repo.set_tag_target("tag1", RefTarget::normal(commit1.id().clone()));
    mut_repo.set_tag_target("tag2", RefTarget::normal(commit2.id().clone()));
    assert_eq!(
        resolve_commit_ids(mut_repo, "tags()"),
        vec![commit2.id().clone(), commit1.id().clone()]
    );
    // Can get tags with matching names
    assert_eq!(
        resolve_commit_ids(mut_repo, "tags(tag1)"),
        vec![commit1.id().clone()]
    );
    assert_eq!(
        resolve_commit_ids(mut_repo, "tags(tag)"),
        vec![commit2.id().clone(), commit1.id().clone()]
    );
    assert_eq!(
        resolve_commit_ids(mut_repo, "tags(exact:tag1)"),
        vec![commit1.id().clone()]
    );
    assert_eq!(resolve_commit_ids(mut_repo, r#"tags(glob:"Tag?")"#), vec![]);
    assert_eq!(
        resolve_commit_ids(mut_repo, r#"tags(glob-i:"Tag?")"#),
        vec![commit2.id().clone(), commit1.id().clone()]
    );
    assert_eq!(
        resolve_commit_ids(mut_repo, "tags(regex:'ag')"),
        vec![commit2.id().clone(), commit1.id().clone()]
    );
    assert_eq!(
        resolve_commit_ids(mut_repo, "tags(regex:'^[Tt]ag1$')"),
        vec![commit1.id().clone()]
    );
    // Can silently resolve to an empty set if there's no matches
    assert_eq!(resolve_commit_ids(mut_repo, "tags(tag3)"), vec![]);
    assert_eq!(resolve_commit_ids(mut_repo, "tags(exact:ag1)"), vec![]);
    // Two tags pointing to the same commit does not result in a duplicate in
    // the revset
    mut_repo.set_tag_target("tag3", RefTarget::normal(commit2.id().clone()));
    assert_eq!(
        resolve_commit_ids(mut_repo, "tags()"),
        vec![commit2.id().clone(), commit1.id().clone()]
    );
    // Can get tags when there are conflicted refs
    mut_repo.set_tag_target(
        "tag1",
        RefTarget::from_legacy_form(
            [commit1.id().clone()],
            [commit2.id().clone(), commit3.id().clone()],
        ),
    );
    mut_repo.set_tag_target(
        "tag2",
        RefTarget::from_legacy_form(
            [commit2.id().clone()],
            [commit3.id().clone(), commit4.id().clone()],
        ),
    );
    mut_repo.set_tag_target("tag3", RefTarget::absent());
    assert_eq!(
        resolve_commit_ids(mut_repo, "tags()"),
        vec![
            commit4.id().clone(),
            commit3.id().clone(),
            commit2.id().clone()
        ]
    );
}

#[test]
fn test_evaluate_expression_latest() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();

    let mut write_commit_with_committer_timestamp = |sec: i64| {
        let builder = create_random_commit(mut_repo);
        let mut committer = builder.committer().clone();
        committer.timestamp.timestamp = MillisSinceEpoch(sec * 1000);
        builder.set_committer(committer).write().unwrap()
    };
    let commit1_t3 = write_commit_with_committer_timestamp(3);
    let commit2_t2 = write_commit_with_committer_timestamp(2);
    let commit3_t2 = write_commit_with_committer_timestamp(2);
    let commit4_t1 = write_commit_with_committer_timestamp(1);

    // Pick the latest entry by default (count = 1)
    assert_eq!(
        resolve_commit_ids(mut_repo, "latest(all())"),
        vec![commit1_t3.id().clone()],
    );

    // Should not panic with count = 0 or empty set
    assert_eq!(resolve_commit_ids(mut_repo, "latest(all(), 0)"), vec![]);
    assert_eq!(resolve_commit_ids(mut_repo, "latest(none())"), vec![]);

    assert_eq!(
        resolve_commit_ids(mut_repo, "latest(all(), 1)"),
        vec![commit1_t3.id().clone()],
    );

    // Tie-breaking: pick the later entry in position
    assert_eq!(
        resolve_commit_ids(mut_repo, "latest(all(), 2)"),
        vec![commit3_t2.id().clone(), commit1_t3.id().clone()],
    );

    assert_eq!(
        resolve_commit_ids(mut_repo, "latest(all(), 3)"),
        vec![
            commit3_t2.id().clone(),
            commit2_t2.id().clone(),
            commit1_t3.id().clone(),
        ],
    );

    assert_eq!(
        resolve_commit_ids(mut_repo, "latest(all(), 4)"),
        vec![
            commit4_t1.id().clone(),
            commit3_t2.id().clone(),
            commit2_t2.id().clone(),
            commit1_t3.id().clone(),
        ],
    );

    assert_eq!(
        resolve_commit_ids(mut_repo, "latest(all(), 5)"),
        vec![
            commit4_t1.id().clone(),
            commit3_t2.id().clone(),
            commit2_t2.id().clone(),
            commit1_t3.id().clone(),
            mut_repo.store().root_commit_id().clone(),
        ],
    );

    // Should not panic if count is larger than the candidates size
    assert_eq!(
        resolve_commit_ids(mut_repo, "latest(~root(), 5)"),
        vec![
            commit4_t1.id().clone(),
            commit3_t2.id().clone(),
            commit2_t2.id().clone(),
            commit1_t3.id().clone(),
        ],
    );
}

#[test]
fn test_evaluate_expression_fork_point() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    // 5 6
    // |/|
    // 4 |
    // | |
    // 1 2 3
    // | |/
    // |/
    // 0
    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();
    let mut graph_builder = CommitGraphBuilder::new(mut_repo);
    let root_commit = repo.store().root_commit();
    let commit1 = graph_builder.initial_commit();
    let commit2 = graph_builder.initial_commit();
    let commit3 = graph_builder.initial_commit();
    let commit4 = graph_builder.commit_with_parents(&[&commit1]);
    let commit5 = graph_builder.commit_with_parents(&[&commit4]);
    let commit6 = graph_builder.commit_with_parents(&[&commit4, &commit2]);

    assert_eq!(resolve_commit_ids(mut_repo, "fork_point(none())"), vec![]);
    assert_eq!(
        resolve_commit_ids(mut_repo, "fork_point(root())"),
        vec![root_commit.id().clone()]
    );
    assert_eq!(
        resolve_commit_ids(mut_repo, &format!("fork_point({})", commit1.id())),
        vec![commit1.id().clone()]
    );
    assert_eq!(
        resolve_commit_ids(mut_repo, &format!("fork_point({})", commit2.id())),
        vec![commit2.id().clone()]
    );
    assert_eq!(
        resolve_commit_ids(mut_repo, &format!("fork_point({})", commit3.id())),
        vec![commit3.id().clone()]
    );
    assert_eq!(
        resolve_commit_ids(mut_repo, &format!("fork_point({})", commit4.id())),
        vec![commit4.id().clone()]
    );
    assert_eq!(
        resolve_commit_ids(mut_repo, &format!("fork_point({})", commit5.id())),
        vec![commit5.id().clone()]
    );
    assert_eq!(
        resolve_commit_ids(mut_repo, &format!("fork_point({})", commit6.id())),
        vec![commit6.id().clone()]
    );
    assert_eq!(
        resolve_commit_ids(
            mut_repo,
            &format!("fork_point({} | {})", commit1.id(), commit2.id())
        ),
        vec![root_commit.id().clone()]
    );
    assert_eq!(
        resolve_commit_ids(
            mut_repo,
            &format!("fork_point({} | {})", commit2.id(), commit3.id())
        ),
        vec![root_commit.id().clone()]
    );
    assert_eq!(
        resolve_commit_ids(
            mut_repo,
            &format!(
                "fork_point({} | {} | {})",
                commit1.id(),
                commit2.id(),
                commit3.id()
            )
        ),
        vec![root_commit.id().clone()]
    );
    assert_eq!(
        resolve_commit_ids(
            mut_repo,
            &format!("fork_point({} | {})", commit1.id(), commit4.id())
        ),
        vec![commit1.id().clone()]
    );
    assert_eq!(
        resolve_commit_ids(
            mut_repo,
            &format!("fork_point({} | {})", commit2.id(), commit5.id())
        ),
        vec![root_commit.id().clone()]
    );
    assert_eq!(
        resolve_commit_ids(
            mut_repo,
            &format!("fork_point({} | {})", commit3.id(), commit6.id())
        ),
        vec![root_commit.id().clone()]
    );
    assert_eq!(
        resolve_commit_ids(
            mut_repo,
            &format!("fork_point({} | {})", commit1.id(), commit5.id())
        ),
        vec![commit1.id().clone()]
    );
    assert_eq!(
        resolve_commit_ids(
            mut_repo,
            &format!("fork_point({} | {})", commit4.id(), commit5.id())
        ),
        vec![commit4.id().clone()]
    );
    assert_eq!(
        resolve_commit_ids(
            mut_repo,
            &format!("fork_point({} | {})", commit5.id(), commit6.id())
        ),
        vec![commit4.id().clone()]
    );
}

#[test]
fn test_evaluate_expression_fork_point_criss_cross() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    // 3 4
    // |X|
    // 1 2
    // |/
    // 0
    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();
    let mut graph_builder = CommitGraphBuilder::new(mut_repo);
    let commit1 = graph_builder.initial_commit();
    let commit2 = graph_builder.initial_commit();
    let commit3 = graph_builder.commit_with_parents(&[&commit1, &commit2]);
    let commit4 = graph_builder.commit_with_parents(&[&commit1, &commit2]);

    assert_eq!(
        resolve_commit_ids(
            mut_repo,
            &format!("fork_point({} | {})", commit3.id(), commit4.id())
        ),
        vec![commit2.id().clone(), commit1.id().clone()]
    );
}

#[test]
fn test_evaluate_expression_fork_point_merge_with_ancestor() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    // 4   5
    // |\ /|
    // 1 2 3
    //  \|/
    //   0
    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();
    let mut graph_builder = CommitGraphBuilder::new(mut_repo);
    let commit1 = graph_builder.initial_commit();
    let commit2 = graph_builder.initial_commit();
    let commit3 = graph_builder.initial_commit();
    let commit4 = graph_builder.commit_with_parents(&[&commit1, &commit2]);
    let commit5 = graph_builder.commit_with_parents(&[&commit2, &commit3]);

    assert_eq!(
        resolve_commit_ids(
            mut_repo,
            &format!("fork_point({} | {})", commit4.id(), commit5.id())
        ),
        vec![commit2.id().clone()]
    );
}

#[test]
fn test_evaluate_expression_merges() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();
    let mut graph_builder = CommitGraphBuilder::new(mut_repo);
    let commit1 = graph_builder.initial_commit();
    let commit2 = graph_builder.initial_commit();
    let commit3 = graph_builder.initial_commit();
    let commit4 = graph_builder.commit_with_parents(&[&commit1, &commit2]);
    let commit5 = graph_builder.commit_with_parents(&[&commit1, &commit2, &commit3]);

    // Finds all merges by default
    assert_eq!(
        resolve_commit_ids(mut_repo, "merges()"),
        vec![commit5.id().clone(), commit4.id().clone(),]
    );
    // Searches only among candidates if specified
    assert_eq!(
        resolve_commit_ids(mut_repo, &format!("::{} & merges()", commit5.id())),
        vec![commit5.id().clone()]
    );
}

#[test]
fn test_evaluate_expression_description() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();

    let commit1 = create_random_commit(mut_repo)
        .set_description("commit 1\n")
        .write()
        .unwrap();
    let commit2 = create_random_commit(mut_repo)
        .set_parents(vec![commit1.id().clone()])
        .set_description("commit 2\n\nblah blah...\n")
        .write()
        .unwrap();
    let commit3 = create_random_commit(mut_repo)
        .set_parents(vec![commit2.id().clone()])
        .set_description("commit 3\n")
        .write()
        .unwrap();

    // Can find multiple matches
    assert_eq!(
        resolve_commit_ids(mut_repo, "description(commit)"),
        vec![
            commit3.id().clone(),
            commit2.id().clone(),
            commit1.id().clone()
        ]
    );
    // Can find a unique match
    assert_eq!(
        resolve_commit_ids(mut_repo, "description(\"commit 2\")"),
        vec![commit2.id().clone()]
    );
    // Searches only among candidates if specified
    assert_eq!(
        resolve_commit_ids(mut_repo, "visible_heads() & description(\"commit 2\")"),
        vec![]
    );

    // Exact match
    assert_eq!(
        resolve_commit_ids(mut_repo, r#"description(exact:"commit 1\n")"#),
        vec![commit1.id().clone()]
    );
    assert_eq!(
        resolve_commit_ids(mut_repo, r#"description(exact:"commit 2\n")"#),
        vec![]
    );
    assert_eq!(
        resolve_commit_ids(mut_repo, "description(exact:'')"),
        vec![mut_repo.store().root_commit_id().clone()]
    );

    // Match subject line
    assert_eq!(
        resolve_commit_ids(mut_repo, "subject(glob:'commit ?')"),
        vec![
            commit3.id().clone(),
            commit2.id().clone(),
            commit1.id().clone(),
        ]
    );
    assert_eq!(resolve_commit_ids(mut_repo, "subject('blah')"), vec![]);
    assert_eq!(
        resolve_commit_ids(mut_repo, "subject(exact:'commit 2')"),
        vec![commit2.id().clone()]
    );
    // Empty description should have empty subject line
    assert_eq!(
        resolve_commit_ids(mut_repo, "subject(exact:'')"),
        vec![mut_repo.store().root_commit_id().clone()]
    );
}

#[test]
fn test_evaluate_expression_author() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();

    let timestamp = Timestamp {
        timestamp: MillisSinceEpoch(0),
        tz_offset: 0,
    };
    let commit1 = create_random_commit(mut_repo)
        .set_author(Signature {
            name: "name1".to_string(),
            email: "email1".to_string(),
            timestamp,
        })
        .write()
        .unwrap();
    let commit2 = create_random_commit(mut_repo)
        .set_parents(vec![commit1.id().clone()])
        .set_author(Signature {
            name: "name2".to_string(),
            email: "email2".to_string(),
            timestamp,
        })
        .write()
        .unwrap();
    let commit3 = create_random_commit(mut_repo)
        .set_parents(vec![commit2.id().clone()])
        .set_author(Signature {
            name: "name3".to_string(),
            email: "email3".to_string(),
            timestamp,
        })
        .write()
        .unwrap();

    // Can find multiple matches
    assert_eq!(
        resolve_commit_ids(mut_repo, "author(name)"),
        vec![
            commit3.id().clone(),
            commit2.id().clone(),
            commit1.id().clone()
        ]
    );
    // Can find a unique match by either name or email
    assert_eq!(
        resolve_commit_ids(mut_repo, "author(\"name2\")"),
        vec![commit2.id().clone()]
    );
    assert_eq!(
        resolve_commit_ids(mut_repo, "author(\"email3\")"),
        vec![commit3.id().clone()]
    );
    // Can match case‐insensitively
    assert_eq!(
        resolve_commit_ids(mut_repo, "author(substring-i:Name)"),
        vec![
            commit3.id().clone(),
            commit2.id().clone(),
            commit1.id().clone(),
        ]
    );

    // Can match name or email explicitly
    assert_eq!(
        resolve_commit_ids(mut_repo, "author_name('name2')"),
        vec![commit2.id().clone()]
    );
    assert_eq!(
        resolve_commit_ids(mut_repo, "author_email('name2')"),
        vec![]
    );
    assert_eq!(
        resolve_commit_ids(mut_repo, "author_name('email2')"),
        vec![]
    );
    assert_eq!(
        resolve_commit_ids(mut_repo, "author_email('email2')"),
        vec![commit2.id().clone()]
    );

    // Searches only among candidates if specified
    assert_eq!(
        resolve_commit_ids(mut_repo, "visible_heads() & author(\"name2\")"),
        vec![]
    );
    // Filter by union of pure predicate and set
    assert_eq!(
        resolve_commit_ids(
            mut_repo,
            &format!("root().. & (author(name1) | {})", commit3.id())
        ),
        vec![commit3.id().clone(), commit1.id().clone()]
    );
}

fn parse_timestamp(s: &str) -> Timestamp {
    Timestamp::from_datetime(s.parse::<DateTime<chrono::FixedOffset>>().unwrap())
}

#[test]
fn test_evaluate_expression_author_date() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();

    let timestamp1 = parse_timestamp("2023-03-25T11:30:00Z");
    let timestamp2 = parse_timestamp("2023-03-25T12:30:00Z");
    let timestamp3 = parse_timestamp("2023-03-25T13:30:00Z");

    let root_commit = repo.store().root_commit();
    let commit1 = create_random_commit(mut_repo)
        .set_author(Signature {
            name: "name1".to_string(),
            email: "email1".to_string(),
            timestamp: timestamp1,
        })
        .set_committer(Signature {
            name: "name1".to_string(),
            email: "email1".to_string(),
            timestamp: timestamp2,
        })
        .write()
        .unwrap();
    let commit2 = create_random_commit(mut_repo)
        .set_parents(vec![commit1.id().clone()])
        .set_author(Signature {
            name: "name2".to_string(),
            email: "email2".to_string(),
            timestamp: timestamp2,
        })
        .set_committer(Signature {
            name: "name1".to_string(),
            email: "email1".to_string(),
            timestamp: timestamp2,
        })
        .write()
        .unwrap();
    let commit3 = create_random_commit(mut_repo)
        .set_parents(vec![commit2.id().clone()])
        .set_author(Signature {
            name: "name3".to_string(),
            email: "email3".to_string(),
            timestamp: timestamp3,
        })
        .set_committer(Signature {
            name: "name1".to_string(),
            email: "email1".to_string(),
            timestamp: timestamp2,
        })
        .write()
        .unwrap();

    // Can find multiple matches
    assert_eq!(
        resolve_commit_ids(mut_repo, "author_date(after:'2023-03-25 12:00')"),
        vec![commit3.id().clone(), commit2.id().clone()]
    );
    assert_eq!(
        resolve_commit_ids(mut_repo, "author_date(before:'2023-03-25 12:00')"),
        vec![commit1.id().clone(), root_commit.id().clone()]
    );
}

#[test]
fn test_evaluate_expression_committer_date() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();

    let timestamp1 = parse_timestamp("2023-03-25T11:30:00Z");
    let timestamp2 = parse_timestamp("2023-03-25T12:30:00Z");
    let timestamp3 = parse_timestamp("2023-03-25T13:30:00Z");

    let root_commit = repo.store().root_commit();
    let commit1 = create_random_commit(mut_repo)
        .set_author(Signature {
            name: "name1".to_string(),
            email: "email1".to_string(),
            timestamp: timestamp2,
        })
        .set_committer(Signature {
            name: "name1".to_string(),
            email: "email1".to_string(),
            timestamp: timestamp1,
        })
        .write()
        .unwrap();
    let commit2 = create_random_commit(mut_repo)
        .set_parents(vec![commit1.id().clone()])
        .set_author(Signature {
            name: "name2".to_string(),
            email: "email2".to_string(),
            timestamp: timestamp2,
        })
        .set_committer(Signature {
            name: "name1".to_string(),
            email: "email1".to_string(),
            timestamp: timestamp2,
        })
        .write()
        .unwrap();
    let commit3 = create_random_commit(mut_repo)
        .set_parents(vec![commit2.id().clone()])
        .set_author(Signature {
            name: "name3".to_string(),
            email: "email3".to_string(),
            timestamp: timestamp2,
        })
        .set_committer(Signature {
            name: "name1".to_string(),
            email: "email1".to_string(),
            timestamp: timestamp3,
        })
        .write()
        .unwrap();

    // Can find multiple matches
    assert_eq!(
        resolve_commit_ids(mut_repo, "committer_date(after:'2023-03-25 12:00')"),
        vec![commit3.id().clone(), commit2.id().clone()]
    );
    assert_eq!(
        resolve_commit_ids(mut_repo, "committer_date(before:'2023-03-25 12:00')"),
        vec![commit1.id().clone(), root_commit.id().clone()]
    );
}

#[test]
fn test_evaluate_expression_mine() {
    let settings = testutils::user_settings();
    let test_repo = TestRepo::init_with_settings(&settings);
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();

    let timestamp = Timestamp {
        timestamp: MillisSinceEpoch(0),
        tz_offset: 0,
    };
    let commit1 = create_random_commit(mut_repo)
        .set_author(Signature {
            // Test that the name field doesn't match
            name: settings.user_email().to_owned(),
            email: "email1".to_string(),
            timestamp,
        })
        .write()
        .unwrap();
    let commit2 = create_random_commit(mut_repo)
        .set_parents(vec![commit1.id().clone()])
        .set_author(Signature {
            name: "name2".to_string(),
            email: settings.user_email().to_owned(),
            timestamp,
        })
        .write()
        .unwrap();
    // Can find a unique match
    assert_eq!(
        resolve_commit_ids(mut_repo, "mine()"),
        vec![commit2.id().clone()]
    );
    let commit3 = create_random_commit(mut_repo)
        .set_parents(vec![commit2.id().clone()])
        .set_author(Signature {
            name: "name3".to_string(),
            // Test that matches are case‐insensitive
            email: settings.user_email().to_ascii_uppercase(),
            timestamp,
        })
        .write()
        .unwrap();
    // Can find multiple matches
    assert_eq!(
        resolve_commit_ids(mut_repo, "mine()"),
        vec![commit3.id().clone(), commit2.id().clone()]
    );
    // Searches only among candidates if specified
    assert_eq!(
        resolve_commit_ids(mut_repo, "visible_heads() & mine()"),
        vec![commit3.id().clone()],
    );
    // Filter by union of pure predicate and set
    assert_eq!(
        resolve_commit_ids(mut_repo, &format!("root().. & (mine() | {})", commit1.id())),
        vec![
            commit3.id().clone(),
            commit2.id().clone(),
            commit1.id().clone()
        ]
    );
}

#[test]
fn test_evaluate_expression_committer() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();

    let timestamp = Timestamp {
        timestamp: MillisSinceEpoch(0),
        tz_offset: 0,
    };
    let commit1 = create_random_commit(mut_repo)
        .set_committer(Signature {
            name: "name1".to_string(),
            email: "email1".to_string(),
            timestamp,
        })
        .write()
        .unwrap();
    let commit2 = create_random_commit(mut_repo)
        .set_parents(vec![commit1.id().clone()])
        .set_committer(Signature {
            name: "name2".to_string(),
            email: "email2".to_string(),
            timestamp,
        })
        .write()
        .unwrap();
    let commit3 = create_random_commit(mut_repo)
        .set_parents(vec![commit2.id().clone()])
        .set_committer(Signature {
            name: "name3".to_string(),
            email: "email3".to_string(),
            timestamp,
        })
        .write()
        .unwrap();

    // Can find multiple matches
    assert_eq!(
        resolve_commit_ids(mut_repo, "committer(name)"),
        vec![
            commit3.id().clone(),
            commit2.id().clone(),
            commit1.id().clone()
        ]
    );
    // Can find a unique match by either name or email
    assert_eq!(
        resolve_commit_ids(mut_repo, "committer(\"name2\")"),
        vec![commit2.id().clone()]
    );
    assert_eq!(
        resolve_commit_ids(mut_repo, "committer(\"email3\")"),
        vec![commit3.id().clone()]
    );
    // Can match case‐insensitively
    assert_eq!(
        resolve_commit_ids(mut_repo, "committer(substring-i:Name)"),
        vec![
            commit3.id().clone(),
            commit2.id().clone(),
            commit1.id().clone(),
        ]
    );

    // Can match name or email explicitly
    assert_eq!(
        resolve_commit_ids(mut_repo, "committer_name('name2')"),
        vec![commit2.id().clone()]
    );
    assert_eq!(
        resolve_commit_ids(mut_repo, "committer_email('name2')"),
        vec![]
    );
    assert_eq!(
        resolve_commit_ids(mut_repo, "committer_name('email2')"),
        vec![]
    );
    assert_eq!(
        resolve_commit_ids(mut_repo, "committer_email('email2')"),
        vec![commit2.id().clone()]
    );

    // Searches only among candidates if specified
    assert_eq!(
        resolve_commit_ids(mut_repo, "visible_heads() & committer(\"name2\")"),
        vec![]
    );
}

#[test]
fn test_evaluate_expression_at_operation() {
    let test_repo = TestRepo::init();
    let repo0 = &test_repo.repo;
    let root_commit = repo0.store().root_commit();

    let mut tx = repo0.start_transaction();
    let commit1_op1 = create_random_commit(tx.repo_mut())
        .set_description("commit1@op1")
        .write()
        .unwrap();
    let commit2_op1 = create_random_commit(tx.repo_mut())
        .set_description("commit2@op1")
        .write()
        .unwrap();
    tx.repo_mut()
        .set_local_bookmark_target("commit1_ref", RefTarget::normal(commit1_op1.id().clone()));
    let repo1 = tx.commit("test").unwrap();

    let mut tx = repo1.start_transaction();
    let commit1_op2 = tx
        .repo_mut()
        .rewrite_commit(&commit1_op1)
        .set_description("commit1@op2")
        .write()
        .unwrap();
    let commit3_op2 = create_random_commit(tx.repo_mut())
        .set_description("commit3@op2")
        .write()
        .unwrap();
    tx.repo_mut().rebase_descendants().unwrap();
    let repo2 = tx.commit("test").unwrap();

    let mut tx = repo2.start_transaction();
    let _commit4_op3 = create_random_commit(tx.repo_mut())
        .set_description("commit4@op3")
        .write()
        .unwrap();

    // Symbol resolution:
    assert_eq!(
        resolve_commit_ids(repo2.as_ref(), "at_operation(@, commit1_ref)"),
        vec![commit1_op2.id().clone()]
    );
    assert_eq!(
        resolve_commit_ids(repo2.as_ref(), "at_operation(@-, commit1_ref)"),
        vec![commit1_op1.id().clone()]
    );
    assert_matches!(
        try_resolve_commit_ids(repo2.as_ref(), "at_operation(@--, commit1_ref)"),
        Err(RevsetResolutionError::NoSuchRevision { .. })
    );
    assert_eq!(
        resolve_commit_ids(repo2.as_ref(), "present(at_operation(@--, commit1_ref))"),
        vec![]
    );
    assert_eq!(
        resolve_commit_ids(repo2.as_ref(), "at_operation(@--, present(commit1_ref))"),
        vec![]
    );

    // Visibility resolution:
    assert_eq!(
        resolve_commit_ids(repo2.as_ref(), "at_operation(@, all())"),
        vec![
            commit3_op2.id().clone(),
            commit1_op2.id().clone(),
            commit2_op1.id().clone(),
            root_commit.id().clone(),
        ]
    );
    assert_eq!(
        resolve_commit_ids(repo2.as_ref(), "at_operation(@-, all())"),
        vec![
            commit2_op1.id().clone(),
            commit1_op1.id().clone(),
            root_commit.id().clone(),
        ]
    );

    // Operation is resolved relative to the outer ReadonlyRepo.
    assert_eq!(
        resolve_commit_ids(repo2.as_ref(), "at_operation(@-, at_operation(@-, all()))"),
        resolve_commit_ids(repo2.as_ref(), "at_operation(@--, all())")
    );

    // TODO: It might make more sense to resolve "@" to the current MutableRepo
    // state. However, doing that isn't easy because there's no Operation object
    // representing a MutableRepo state. For now, "@" is resolved to the base
    // operation.
    assert_eq!(
        resolve_commit_ids(tx.repo(), "at_operation(@, all())"),
        vec![
            commit3_op2.id().clone(),
            commit1_op2.id().clone(),
            commit2_op1.id().clone(),
            root_commit.id().clone(),
        ]
    );

    // Filter should be evaluated within the at-op repo. Note that this can
    // populate hidden commits without explicitly referring them by commit refs.
    // It might be better to intersect inner results again with the outer repo.
    assert_eq!(
        resolve_commit_ids(repo2.as_ref(), "at_operation(@-, description('commit'))"),
        vec![commit2_op1.id().clone(), commit1_op1.id().clone()]
    );
    // For the same reason, commit1_op1 isn't filtered out. The following query
    // is effectively evaluated as "description('commit1') & commit1_op1".
    assert_eq!(
        resolve_commit_ids(
            repo2.as_ref(),
            "description('commit1') & at_operation(@-, description('commit'))"
        ),
        vec![commit1_op1.id().clone()]
    );
    // If we have an explicit ::visible_heads(), commit1_op1 is filtered out.
    assert_eq!(
        resolve_commit_ids(
            repo2.as_ref(),
            "::visible_heads() & description('commit1') & at_operation(@-, description('commit'))"
        ),
        vec![]
    );

    // Bad operation:
    // TODO: should we suppress NoSuchOperation error by present()?
    assert_matches!(
        try_resolve_commit_ids(repo2.as_ref(), "at_operation(000000000000-, all())"),
        Err(RevsetResolutionError::Other(_))
    );
}

#[test]
fn test_evaluate_expression_coalesce() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;
    let root_commit_id = repo.store().root_commit_id().clone();

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();
    let mut graph_builder = CommitGraphBuilder::new(mut_repo);
    let commit1 = graph_builder.initial_commit();
    let commit2 = graph_builder.commit_with_parents(&[&commit1]);
    mut_repo.set_local_bookmark_target("commit1", RefTarget::normal(commit1.id().clone()));
    mut_repo.set_local_bookmark_target("commit2", RefTarget::normal(commit2.id().clone()));

    assert_eq!(resolve_commit_ids(mut_repo, "coalesce()"), vec![]);
    assert_eq!(resolve_commit_ids(mut_repo, "coalesce(none())"), vec![]);
    assert_eq!(
        resolve_commit_ids(mut_repo, "coalesce(all())"),
        vec![
            commit2.id().clone(),
            commit1.id().clone(),
            root_commit_id.clone(),
        ]
    );
    assert_eq!(
        resolve_commit_ids(mut_repo, "coalesce(all(), commit1)"),
        vec![
            commit2.id().clone(),
            commit1.id().clone(),
            root_commit_id.clone(),
        ]
    );
    assert_eq!(
        resolve_commit_ids(mut_repo, "coalesce(none(), commit1)"),
        vec![commit1.id().clone()]
    );
    assert_eq!(
        resolve_commit_ids(mut_repo, "coalesce(commit1, commit2)"),
        vec![commit1.id().clone()]
    );
    assert_eq!(
        resolve_commit_ids(mut_repo, "coalesce(none(), none(), commit2)"),
        vec![commit2.id().clone()]
    );
    assert_eq!(
        resolve_commit_ids(mut_repo, "coalesce(none(), commit1, commit2)"),
        vec![commit1.id().clone()]
    );
    // Should resolve invalid symbols regardless of whether a specific revset is
    // evaluated.
    assert_matches!(
        try_resolve_commit_ids(mut_repo, "coalesce(all(), commit1_invalid)"),
        Err(RevsetResolutionError::NoSuchRevision { name, .. })
        if name == "commit1_invalid"
    );
    assert_matches!(
        try_resolve_commit_ids(mut_repo, "coalesce(none(), commit1_invalid)"),
        Err(RevsetResolutionError::NoSuchRevision { name, .. })
        if name == "commit1_invalid"
    );
    assert_matches!(
        try_resolve_commit_ids(mut_repo, "coalesce(all(), commit1, commit2_invalid)"),
        Err(RevsetResolutionError::NoSuchRevision { name, .. })
        if name == "commit2_invalid"
    );
    assert_matches!(
        try_resolve_commit_ids(mut_repo, "coalesce(none(), commit1, commit2_invalid)"),
        Err(RevsetResolutionError::NoSuchRevision { name, .. })
        if name == "commit2_invalid"
    );
    assert_matches!(
        try_resolve_commit_ids(mut_repo, "coalesce(none(), commit1, commit2, commit2_invalid)"),
        Err(RevsetResolutionError::NoSuchRevision { name, .. })
        if name == "commit2_invalid"
    );
}

#[test]
fn test_evaluate_expression_union() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let root_commit = repo.store().root_commit();
    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();
    let mut graph_builder = CommitGraphBuilder::new(mut_repo);
    let commit1 = graph_builder.initial_commit();
    let commit2 = graph_builder.commit_with_parents(&[&commit1]);
    let commit3 = graph_builder.commit_with_parents(&[&commit2]);
    let commit4 = graph_builder.commit_with_parents(&[&commit3]);
    let commit5 = graph_builder.commit_with_parents(&[&commit2]);

    // Union between ancestors
    assert_eq!(
        resolve_commit_ids(
            mut_repo,
            &format!("::{} | ::{}", commit4.id(), commit5.id())
        ),
        vec![
            commit5.id().clone(),
            commit4.id().clone(),
            commit3.id().clone(),
            commit2.id().clone(),
            commit1.id().clone(),
            root_commit.id().clone()
        ]
    );

    // Unioning can add back commits removed by difference
    assert_eq!(
        resolve_commit_ids(
            mut_repo,
            &format!(
                "(::{} ~ ::{}) | ::{}",
                commit4.id(),
                commit2.id(),
                commit5.id()
            )
        ),
        vec![
            commit5.id().clone(),
            commit4.id().clone(),
            commit3.id().clone(),
            commit2.id().clone(),
            commit1.id().clone(),
            root_commit.id().clone(),
        ]
    );

    // Unioning of disjoint sets
    assert_eq!(
        resolve_commit_ids(
            mut_repo,
            &format!(
                "(::{} ~ ::{}) | {}",
                commit4.id(),
                commit2.id(),
                commit5.id(),
            )
        ),
        vec![
            commit5.id().clone(),
            commit4.id().clone(),
            commit3.id().clone()
        ]
    );
}

#[test]
fn test_evaluate_expression_machine_generated_union() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();
    let mut graph_builder = CommitGraphBuilder::new(mut_repo);
    let commit1 = graph_builder.initial_commit();
    let commit2 = graph_builder.commit_with_parents(&[&commit1]);

    // This query shouldn't trigger stack overflow. Here we use "x::y" in case
    // we had optimization path for trivial "commit_id|.." expression.
    let revset_str = iter::repeat(format!("({}::{})", commit1.id(), commit2.id()))
        .take(5000)
        .join("|");
    assert_eq!(
        resolve_commit_ids(mut_repo, &revset_str),
        vec![commit2.id().clone(), commit1.id().clone()]
    );
}

#[test]
fn test_evaluate_expression_intersection() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let root_commit = repo.store().root_commit();
    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();
    let mut graph_builder = CommitGraphBuilder::new(mut_repo);
    let commit1 = graph_builder.initial_commit();
    let commit2 = graph_builder.commit_with_parents(&[&commit1]);
    let commit3 = graph_builder.commit_with_parents(&[&commit2]);
    let commit4 = graph_builder.commit_with_parents(&[&commit3]);
    let commit5 = graph_builder.commit_with_parents(&[&commit2]);

    // Intersection between ancestors
    assert_eq!(
        resolve_commit_ids(
            mut_repo,
            &format!("::{} & ::{}", commit4.id(), commit5.id())
        ),
        vec![
            commit2.id().clone(),
            commit1.id().clone(),
            root_commit.id().clone()
        ]
    );

    // Intersection of disjoint sets
    assert_eq!(
        resolve_commit_ids(mut_repo, &format!("{} & {}", commit4.id(), commit2.id())),
        vec![]
    );
}

#[test]
fn test_evaluate_expression_difference() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let root_commit = repo.store().root_commit();
    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();
    let mut graph_builder = CommitGraphBuilder::new(mut_repo);
    let commit1 = graph_builder.initial_commit();
    let commit2 = graph_builder.commit_with_parents(&[&commit1]);
    let commit3 = graph_builder.commit_with_parents(&[&commit2]);
    let commit4 = graph_builder.commit_with_parents(&[&commit3]);
    let commit5 = graph_builder.commit_with_parents(&[&commit2]);

    // Difference from all
    assert_eq!(
        resolve_commit_ids(mut_repo, &format!("~::{}", commit5.id())),
        vec![commit4.id().clone(), commit3.id().clone()]
    );

    // Difference between ancestors
    assert_eq!(
        resolve_commit_ids(
            mut_repo,
            &format!("::{} ~ ::{}", commit4.id(), commit5.id())
        ),
        vec![commit4.id().clone(), commit3.id().clone()]
    );
    assert_eq!(
        resolve_commit_ids(
            mut_repo,
            &format!("::{} ~ ::{}", commit5.id(), commit4.id())
        ),
        vec![commit5.id().clone()]
    );
    assert_eq!(
        resolve_commit_ids(
            mut_repo,
            &format!("~::{} & ::{}", commit4.id(), commit5.id())
        ),
        vec![commit5.id().clone()]
    );
    assert_eq!(
        resolve_commit_ids(
            mut_repo,
            &format!("::{} ~ ::{}", commit4.id(), commit2.id())
        ),
        vec![commit4.id().clone(), commit3.id().clone()]
    );

    // Associativity
    assert_eq!(
        resolve_commit_ids(
            mut_repo,
            &format!("::{} ~ {} ~ {}", commit4.id(), commit2.id(), commit3.id())
        ),
        vec![
            commit4.id().clone(),
            commit1.id().clone(),
            root_commit.id().clone(),
        ]
    );

    // Subtracting a difference does not add back any commits
    assert_eq!(
        resolve_commit_ids(
            mut_repo,
            &format!(
                "(::{} ~ ::{}) ~ (::{} ~ ::{})",
                commit4.id(),
                commit1.id(),
                commit3.id(),
                commit1.id(),
            )
        ),
        vec![commit4.id().clone()]
    );
}

#[test]
fn test_evaluate_expression_filter_combinator() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();

    let root_commit_id = repo.store().root_commit_id();
    let commit1 = create_random_commit(mut_repo)
        .set_description("commit 1")
        .write()
        .unwrap();
    let commit2 = create_random_commit(mut_repo)
        .set_parents(vec![commit1.id().clone()])
        .set_description("commit 2")
        .write()
        .unwrap();
    let commit3 = create_random_commit(mut_repo)
        .set_parents(vec![commit2.id().clone()])
        .set_description("commit 3")
        .write()
        .unwrap();

    // Not intersected with a set node
    assert_eq!(
        resolve_commit_ids(mut_repo, "~description(1)"),
        vec![
            commit3.id().clone(),
            commit2.id().clone(),
            root_commit_id.clone(),
        ],
    );
    assert_eq!(
        resolve_commit_ids(mut_repo, "description(1) | description(2)"),
        vec![commit2.id().clone(), commit1.id().clone()],
    );
    assert_eq!(
        resolve_commit_ids(
            mut_repo,
            "description(commit) ~ (description(2) | description(3))",
        ),
        vec![commit1.id().clone()],
    );

    // Intersected with a set node
    assert_eq!(
        resolve_commit_ids(mut_repo, "root().. & ~description(1)"),
        vec![commit3.id().clone(), commit2.id().clone()],
    );
    assert_eq!(
        resolve_commit_ids(
            mut_repo,
            &format!("{}.. & (description(1) | description(2))", commit1.id()),
        ),
        vec![commit2.id().clone()],
    );
}

#[test]
fn test_evaluate_expression_file() {
    let test_workspace = TestWorkspace::init();
    let repo = &test_workspace.repo;

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();

    let added_clean_clean = RepoPath::from_internal_string("added_clean_clean");
    let added_modified_clean = RepoPath::from_internal_string("added_modified_clean");
    let added_modified_removed = RepoPath::from_internal_string("added_modified_removed");
    let tree1 = create_tree(
        repo,
        &[
            (added_clean_clean, "1"),
            (added_modified_clean, "1"),
            (added_modified_removed, "1"),
        ],
    );
    let tree2 = create_tree(
        repo,
        &[
            (added_clean_clean, "1"),
            (added_modified_clean, "2"),
            (added_modified_removed, "2"),
        ],
    );
    let tree3 = create_tree(
        repo,
        &[
            (added_clean_clean, "1"),
            (added_modified_clean, "2"),
            // added_modified_removed,
        ],
    );
    let commit1 = mut_repo
        .new_commit(vec![repo.store().root_commit_id().clone()], tree1.id())
        .write()
        .unwrap();
    let commit2 = mut_repo
        .new_commit(vec![commit1.id().clone()], tree2.id())
        .write()
        .unwrap();
    let commit3 = mut_repo
        .new_commit(vec![commit2.id().clone()], tree3.id())
        .write()
        .unwrap();
    let commit4 = mut_repo
        .new_commit(vec![commit3.id().clone()], tree3.id())
        .write()
        .unwrap();

    let resolve = |file_path: &RepoPath| -> Vec<CommitId> {
        let mut_repo = &*mut_repo;
        let expression = RevsetExpression::filter(RevsetFilterPredicate::File(
            FilesetExpression::prefix_path(file_path.to_owned()),
        ));
        let revset = expression.evaluate(mut_repo).unwrap();
        revset.iter().map(Result::unwrap).collect()
    };

    assert_eq!(resolve(added_clean_clean), vec![commit1.id().clone()]);
    assert_eq!(
        resolve(added_modified_clean),
        vec![commit2.id().clone(), commit1.id().clone()]
    );
    assert_eq!(
        resolve(added_modified_removed),
        vec![
            commit3.id().clone(),
            commit2.id().clone(),
            commit1.id().clone()
        ]
    );

    // file() revset:
    assert_eq!(
        resolve_commit_ids_in_workspace(
            mut_repo,
            r#"files("repo/added_clean_clean")"#,
            &test_workspace.workspace,
            Some(test_workspace.workspace.workspace_root().parent().unwrap()),
        ),
        vec![commit1.id().clone()]
    );
    assert_eq!(
        resolve_commit_ids_in_workspace(
            mut_repo,
            r#"files("added_clean_clean"|"added_modified_clean")"#,
            &test_workspace.workspace,
            Some(test_workspace.workspace.workspace_root()),
        ),
        vec![commit2.id().clone(), commit1.id().clone()]
    );
    assert_eq!(
        resolve_commit_ids_in_workspace(
            mut_repo,
            &format!(r#"{}:: & files("added_modified_clean")"#, commit2.id()),
            &test_workspace.workspace,
            Some(test_workspace.workspace.workspace_root()),
        ),
        vec![commit2.id().clone()]
    );

    // empty() revset, which is identical to ~file(".")
    assert_eq!(
        resolve_commit_ids(mut_repo, &format!("{}:: & empty()", commit1.id())),
        vec![commit4.id().clone()]
    );
}

#[test]
fn test_evaluate_expression_diff_contains() {
    let test_workspace = TestWorkspace::init();
    let repo = &test_workspace.repo;

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();

    let empty_clean_inserted_deleted =
        RepoPath::from_internal_string("empty_clean_inserted_deleted");
    let blank_clean_inserted_clean = RepoPath::from_internal_string("blank_clean_inserted_clean");
    let noeol_modified_modified_clean =
        RepoPath::from_internal_string("noeol_modified_modified_clean");
    let normal_inserted_modified_removed =
        RepoPath::from_internal_string("normal_inserted_modified_removed");
    let tree1 = create_tree(
        repo,
        &[
            (empty_clean_inserted_deleted, ""),
            (blank_clean_inserted_clean, "\n"),
            (noeol_modified_modified_clean, "1"),
            (normal_inserted_modified_removed, "1\n"),
        ],
    );
    let tree2 = create_tree(
        repo,
        &[
            (empty_clean_inserted_deleted, ""),
            (blank_clean_inserted_clean, "\n"),
            (noeol_modified_modified_clean, "2"),
            (normal_inserted_modified_removed, "1\n2\n"),
        ],
    );
    let tree3 = create_tree(
        repo,
        &[
            (empty_clean_inserted_deleted, "3"),
            (blank_clean_inserted_clean, "\n3\n"),
            (noeol_modified_modified_clean, "2 3"),
            (normal_inserted_modified_removed, "1 3\n2\n"),
        ],
    );
    let tree4 = create_tree(
        repo,
        &[
            (empty_clean_inserted_deleted, ""),
            (blank_clean_inserted_clean, "\n3\n"),
            (noeol_modified_modified_clean, "2 3"),
            // normal_inserted_modified_removed
        ],
    );
    let commit1 = mut_repo
        .new_commit(vec![repo.store().root_commit_id().clone()], tree1.id())
        .write()
        .unwrap();
    let commit2 = mut_repo
        .new_commit(vec![commit1.id().clone()], tree2.id())
        .write()
        .unwrap();
    let commit3 = mut_repo
        .new_commit(vec![commit2.id().clone()], tree3.id())
        .write()
        .unwrap();
    let commit4 = mut_repo
        .new_commit(vec![commit3.id().clone()], tree4.id())
        .write()
        .unwrap();

    let query = |revset_str: &str| {
        resolve_commit_ids_in_workspace(
            mut_repo,
            revset_str,
            &test_workspace.workspace,
            Some(test_workspace.workspace.workspace_root()),
        )
    };

    // should match both inserted and deleted lines
    assert_eq!(
        query("diff_contains('2')"),
        vec![
            commit4.id().clone(),
            commit3.id().clone(),
            commit2.id().clone(),
        ]
    );
    assert_eq!(
        query("diff_contains('3')"),
        vec![commit4.id().clone(), commit3.id().clone()]
    );
    assert_eq!(query("diff_contains('2 3')"), vec![commit3.id().clone()]);
    assert_eq!(
        query("diff_contains('1 3')"),
        vec![commit4.id().clone(), commit3.id().clone()]
    );

    // should match line with eol
    assert_eq!(
        query(&format!(
            "diff_contains(exact:'1', {normal_inserted_modified_removed:?})",
        )),
        vec![commit3.id().clone(), commit1.id().clone()]
    );

    // should match line without eol
    assert_eq!(
        query(&format!(
            "diff_contains(exact:'1', {noeol_modified_modified_clean:?})",
        )),
        vec![commit2.id().clone(), commit1.id().clone()]
    );

    // exact:'' should match blank line
    assert_eq!(
        query(&format!(
            "diff_contains(exact:'', {empty_clean_inserted_deleted:?})",
        )),
        vec![]
    );
    assert_eq!(
        query(&format!(
            "diff_contains(exact:'', {blank_clean_inserted_clean:?})",
        )),
        vec![commit1.id().clone()]
    );

    // '' should match anything but clean
    assert_eq!(
        query(&format!(
            "diff_contains('', {empty_clean_inserted_deleted:?})",
        )),
        vec![commit4.id().clone(), commit3.id().clone()]
    );
    assert_eq!(
        query(&format!(
            "diff_contains('', {blank_clean_inserted_clean:?})",
        )),
        vec![commit3.id().clone(), commit1.id().clone()]
    );
}

#[test]
fn test_evaluate_expression_file_merged_parents() {
    let test_workspace = TestWorkspace::init();
    let repo = &test_workspace.repo;

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();

    // file2 can be merged automatically, file1 can't.
    let file_path1 = RepoPath::from_internal_string("file1");
    let file_path2 = RepoPath::from_internal_string("file2");
    let tree1 = create_tree(repo, &[(file_path1, "1\n"), (file_path2, "1\n")]);
    let tree2 = create_tree(repo, &[(file_path1, "1\n2\n"), (file_path2, "2\n1\n")]);
    let tree3 = create_tree(repo, &[(file_path1, "1\n3\n"), (file_path2, "1\n3\n")]);
    let tree4 = create_tree(repo, &[(file_path1, "1\n4\n"), (file_path2, "2\n1\n3\n")]);

    let mut create_commit =
        |parent_ids, tree_id| mut_repo.new_commit(parent_ids, tree_id).write().unwrap();
    let commit1 = create_commit(vec![repo.store().root_commit_id().clone()], tree1.id());
    let commit2 = create_commit(vec![commit1.id().clone()], tree2.id());
    let commit3 = create_commit(vec![commit1.id().clone()], tree3.id());
    let commit4 = create_commit(vec![commit2.id().clone(), commit3.id().clone()], tree4.id());

    let query = |revset_str: &str| {
        resolve_commit_ids_in_workspace(
            mut_repo,
            revset_str,
            &test_workspace.workspace,
            Some(test_workspace.workspace.workspace_root()),
        )
    };

    assert_eq!(
        query("files('file1')"),
        vec![
            commit4.id().clone(),
            commit3.id().clone(),
            commit2.id().clone(),
            commit1.id().clone(),
        ]
    );
    assert_eq!(
        query("files('file2')"),
        vec![
            commit3.id().clone(),
            commit2.id().clone(),
            commit1.id().clone(),
        ]
    );

    assert_eq!(
        query("diff_contains(regex:'[1234]', 'file1')"),
        vec![
            commit4.id().clone(),
            commit3.id().clone(),
            commit2.id().clone(),
            commit1.id().clone(),
        ]
    );
    assert_eq!(
        query("diff_contains(regex:'[1234]', 'file2')"),
        vec![
            commit3.id().clone(),
            commit2.id().clone(),
            commit1.id().clone(),
        ]
    );
}

#[test]
fn test_evaluate_expression_conflict() {
    let test_workspace = TestWorkspace::init();
    let repo = &test_workspace.repo;

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();

    // Create a few trees, including one with a conflict in `file1`
    let file_path1 = RepoPath::from_internal_string("file1");
    let file_path2 = RepoPath::from_internal_string("file2");
    let tree1 = create_tree(repo, &[(file_path1, "1"), (file_path2, "1")]);
    let tree2 = create_tree(repo, &[(file_path1, "2"), (file_path2, "2")]);
    let tree3 = create_tree(repo, &[(file_path1, "3"), (file_path2, "1")]);
    let tree4 = tree2.merge(&tree1, &tree3).unwrap();

    let mut create_commit =
        |parent_ids, tree_id| mut_repo.new_commit(parent_ids, tree_id).write().unwrap();
    let commit1 = create_commit(vec![repo.store().root_commit_id().clone()], tree1.id());
    let commit2 = create_commit(vec![commit1.id().clone()], tree2.id());
    let commit3 = create_commit(vec![commit2.id().clone()], tree3.id());
    let commit4 = create_commit(vec![commit3.id().clone()], tree4.id());

    // Only commit4 has a conflict
    assert_eq!(
        resolve_commit_ids(mut_repo, "conflicts()"),
        vec![commit4.id().clone()]
    );
}

#[test]
fn test_reverse_graph() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    // Tests that merges, forks, direct edges, indirect edges, and "missing" edges
    // are correct in reversed graph. "Missing" edges (i.e. edges to commits not
    // in the input set) won't be part of the reversed graph. Conversely, there
    // won't be missing edges to children not in the input.
    //
    //  F
    //  |\
    //  D E
    //  |/
    //  C
    //  |
    //  b
    //  |
    //  A
    //  |
    // root
    let mut tx = repo.start_transaction();
    let mut graph_builder = CommitGraphBuilder::new(tx.repo_mut());
    let commit_a = graph_builder.initial_commit();
    let commit_b = graph_builder.commit_with_parents(&[&commit_a]);
    let commit_c = graph_builder.commit_with_parents(&[&commit_b]);
    let commit_d = graph_builder.commit_with_parents(&[&commit_c]);
    let commit_e = graph_builder.commit_with_parents(&[&commit_c]);
    let commit_f = graph_builder.commit_with_parents(&[&commit_d, &commit_e]);
    let repo = tx.commit("test").unwrap();

    let revset = revset_for_commits(
        repo.as_ref(),
        &[&commit_a, &commit_c, &commit_d, &commit_e, &commit_f],
    );
    let commits = reverse_graph(revset.iter_graph(), |id| id).unwrap();
    assert_eq!(commits.len(), 5);
    assert_eq!(commits[0].0, *commit_a.id());
    assert_eq!(commits[1].0, *commit_c.id());
    assert_eq!(commits[2].0, *commit_d.id());
    assert_eq!(commits[3].0, *commit_e.id());
    assert_eq!(commits[4].0, *commit_f.id());
    assert_eq!(
        commits[0].1,
        vec![GraphEdge::indirect(commit_c.id().clone())]
    );
    assert_eq!(
        commits[1].1,
        vec![
            GraphEdge::direct(commit_e.id().clone()),
            GraphEdge::direct(commit_d.id().clone()),
        ]
    );
    assert_eq!(commits[2].1, vec![GraphEdge::direct(commit_f.id().clone())]);
    assert_eq!(commits[3].1, vec![GraphEdge::direct(commit_f.id().clone())]);
    assert_eq!(commits[4].1, vec![]);
}

#[test]
fn test_no_such_revision_suggestion() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();
    let commit = write_random_commit(mut_repo);

    for bookmark_name in ["foo", "bar", "baz"] {
        mut_repo.set_local_bookmark_target(bookmark_name, RefTarget::normal(commit.id().clone()));
    }

    assert_matches!(resolve_symbol(mut_repo, "bar"), Ok(_));
    assert_matches!(
        resolve_symbol(mut_repo, "bax"),
        Err(RevsetResolutionError::NoSuchRevision { name, candidates })
        if name == "bax" && candidates == vec!["bar".to_string(), "baz".to_string()]
    );
}

#[test]
fn test_revset_containing_fn() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();
    let commit_a = write_random_commit(mut_repo);
    let commit_b = write_random_commit(mut_repo);
    let commit_c = write_random_commit(mut_repo);
    let commit_d = write_random_commit(mut_repo);
    let repo = tx.commit("test").unwrap();

    let revset = revset_for_commits(repo.as_ref(), &[&commit_b, &commit_d]);

    let revset_has_commit = revset.containing_fn();
    assert!(!revset_has_commit(commit_a.id()).unwrap());
    assert!(revset_has_commit(commit_b.id()).unwrap());
    assert!(!revset_has_commit(commit_c.id()).unwrap());
    assert!(revset_has_commit(commit_d.id()).unwrap());
}

#[test]
fn test_evaluate_expression_remote() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;
    let tracking_remote_ref = |target| RemoteRef {
        target,
        state: RemoteRefState::Tracking,
    };
    let normal_tracking_remote_ref =
        |id: &CommitId| tracking_remote_ref(RefTarget::normal(id.clone()));

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();

    let commit1 = write_random_commit(mut_repo);

    mut_repo.set_remote_bookmark(
        "bookmark1",
        "origin",
        normal_tracking_remote_ref(commit1.id()),
    );
    mut_repo.set_local_bookmark_target("bookmark1", RefTarget::normal(commit1.id().clone()));
    mut_repo.track_remote_bookmark("bookmark1@origin", "bookmark1");

    print!("{}", commit1.id());

    assert_eq!(
        resolve_commit_ids(mut_repo, "remote(bookmark1, origin)"),
        vec![commit1.id().clone()]
    );
}
