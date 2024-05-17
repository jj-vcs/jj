// Copyright 2020-2023 The Jujutsu Authors
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

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::io::Write as _;

use clap::builder::NonEmptyStringValueParser;
use itertools::Itertools;
use jj_lib::git;
use jj_lib::object_id::ObjectId;
use jj_lib::op_store::{RefTarget, RemoteRef};
use jj_lib::repo::Repo;
use jj_lib::revset::RevsetExpression;
use jj_lib::str_util::StringPattern;
use jj_lib::view::View;

use crate::cli_util::{CommandHelper, RemoteBranchName, RemoteBranchNamePattern, RevisionArg};
use crate::command_error::{user_error, user_error_with_hint, CommandError};
use crate::commit_templater::{CommitTemplateLanguage, RefName};
use crate::ui::Ui;

/// Manage branches.
///
/// For information about branches, see
/// https://github.com/martinvonz/jj/blob/main/docs/branches.md.
#[derive(clap::Subcommand, Clone, Debug)]
pub enum BranchCommand {
    #[command(visible_alias("c"))]
    Create(BranchCreateArgs),
    #[command(visible_alias("d"))]
    Delete(BranchDeleteArgs),
    #[command(visible_alias("f"))]
    Forget(BranchForgetArgs),
    #[command(visible_alias("l"))]
    List(BranchListArgs),
    #[command(visible_alias("r"))]
    Rename(BranchRenameArgs),
    #[command(visible_alias("s"))]
    Set(BranchSetArgs),
    #[command(visible_alias("t"))]
    Track(BranchTrackArgs),
    Untrack(BranchUntrackArgs),
}

/// Create a new branch.
#[derive(clap::Args, Clone, Debug)]
pub struct BranchCreateArgs {
    /// The branch's target revision.
    #[arg(long, short)]
    revision: Option<RevisionArg>,

    /// The branches to create.
    #[arg(required = true, value_parser=NonEmptyStringValueParser::new())]
    names: Vec<String>,
}

/// Delete an existing branch and propagate the deletion to remotes on the
/// next push.
#[derive(clap::Args, Clone, Debug)]
pub struct BranchDeleteArgs {
    /// The branches to delete
    ///
    /// By default, the specified name matches exactly. Use `glob:` prefix to
    /// select branches by wildcard pattern. For details, see
    /// https://github.com/martinvonz/jj/blob/main/docs/revsets.md#string-patterns.
    #[arg(required_unless_present_any(&["glob"]), value_parser = StringPattern::parse)]
    pub names: Vec<StringPattern>,

    /// Deprecated. Please prefix the pattern with `glob:` instead.
    #[arg(long, hide = true, value_parser = StringPattern::glob)]
    pub glob: Vec<StringPattern>,
}

/// List branches and their targets
///
/// By default, a tracking remote branch will be included only if its target is
/// different from the local target. A non-tracking remote branch won't be
/// listed. For a conflicted branch (both local and remote), old target
/// revisions are preceded by a "-" and new target revisions are preceded by a
/// "+".
///
/// For information about branches, see
/// https://github.com/martinvonz/jj/blob/main/docs/branches.md.
#[derive(clap::Args, Clone, Debug)]
pub struct BranchListArgs {
    /// Show all tracking and non-tracking remote branches including the ones
    /// whose targets are synchronized with the local branches.
    #[arg(long, short, alias = "all")]
    all_remotes: bool,

    /// Show remote tracked branches only. Omits local Git-tracking branches by
    /// default.
    #[arg(long, short, conflicts_with_all = ["all_remotes"])]
    tracked: bool,

    /// Show conflicted branches only.
    #[arg(long, short, conflicts_with_all = ["all_remotes"])]
    conflicted: bool,

    /// Show branches whose local name matches
    ///
    /// By default, the specified name matches exactly. Use `glob:` prefix to
    /// select branches by wildcard pattern. For details, see
    /// https://github.com/martinvonz/jj/blob/main/docs/revsets.md#string-patterns.
    #[arg(value_parser = StringPattern::parse)]
    pub names: Vec<StringPattern>,

    /// Show branches whose local targets are in the given revisions.
    ///
    /// Note that `-r deleted_branch` will not work since `deleted_branch`
    /// wouldn't have a local target.
    #[arg(long, short)]
    revisions: Vec<RevisionArg>,

    /// Render each branch using the given template
    ///
    /// All 0-argument methods of the `RefName` type are available as keywords.
    ///
    /// For the syntax, see https://github.com/martinvonz/jj/blob/main/docs/templates.md
    #[arg(long, short = 'T')]
    template: Option<String>,
}

/// Forget everything about a branch, including its local and remote
/// targets.
///
/// A forgotten branch will not impact remotes on future pushes. It will be
/// recreated on future pulls if it still exists in the remote.
#[derive(clap::Args, Clone, Debug)]
pub struct BranchForgetArgs {
    /// The branches to forget
    ///
    /// By default, the specified name matches exactly. Use `glob:` prefix to
    /// select branches by wildcard pattern. For details, see
    /// https://github.com/martinvonz/jj/blob/main/docs/revsets.md#string-patterns.
    #[arg(required_unless_present_any(&["glob"]), value_parser = StringPattern::parse)]
    pub names: Vec<StringPattern>,

    /// Deprecated. Please prefix the pattern with `glob:` instead.
    #[arg(long, hide = true, value_parser = StringPattern::glob)]
    pub glob: Vec<StringPattern>,
}

/// Rename `old` branch name to `new` branch name.
///
/// The new branch name points at the same commit as the old
/// branch name.
#[derive(clap::Args, Clone, Debug)]
pub struct BranchRenameArgs {
    /// The old name of the branch.
    pub old: String,

    /// The new name of the branch.
    pub new: String,
}

/// Update an existing branch to point to a certain commit.
#[derive(clap::Args, Clone, Debug)]
pub struct BranchSetArgs {
    /// The branch's target revision.
    #[arg(long, short)]
    pub revision: Option<RevisionArg>,

    /// Allow moving the branch backwards or sideways.
    #[arg(long, short = 'B')]
    pub allow_backwards: bool,

    /// The branches to update.
    #[arg(required = true)]
    pub names: Vec<String>,
}

/// Start tracking given remote branches
///
/// A tracking remote branch will be imported as a local branch of the same
/// name. Changes to it will propagate to the existing local branch on future
/// pulls.
#[derive(clap::Args, Clone, Debug)]
pub struct BranchTrackArgs {
    /// Remote branches to track
    ///
    /// By default, the specified name matches exactly. Use `glob:` prefix to
    /// select branches by wildcard pattern. For details, see
    /// https://github.com/martinvonz/jj/blob/main/docs/revsets.md#string-patterns.
    ///
    /// Examples: branch@remote, glob:main@*, glob:jjfan-*@upstream
    #[arg(required = true, value_name = "BRANCH@REMOTE")]
    pub names: Vec<RemoteBranchNamePattern>,
}

/// Stop tracking given remote branches
///
/// A non-tracking remote branch is just a pointer to the last-fetched remote
/// branch. It won't be imported as a local branch on future pulls.
#[derive(clap::Args, Clone, Debug)]
pub struct BranchUntrackArgs {
    /// Remote branches to untrack
    ///
    /// By default, the specified name matches exactly. Use `glob:` prefix to
    /// select branches by wildcard pattern. For details, see
    /// https://github.com/martinvonz/jj/blob/main/docs/revsets.md#string-patterns.
    ///
    /// Examples: branch@remote, glob:main@*, glob:jjfan-*@upstream
    #[arg(required = true, value_name = "BRANCH@REMOTE")]
    pub names: Vec<RemoteBranchNamePattern>,
}

fn make_branch_term(branch_names: &[impl fmt::Display]) -> String {
    match branch_names {
        [branch_name] => format!("branch {}", branch_name),
        branch_names => format!("branches {}", branch_names.iter().join(", ")),
    }
}

pub fn cmd_branch(
    ui: &mut Ui,
    command: &CommandHelper,
    subcommand: &BranchCommand,
) -> Result<(), CommandError> {
    match subcommand {
        BranchCommand::Create(sub_args) => cmd_branch_create(ui, command, sub_args),
        BranchCommand::Rename(sub_args) => cmd_branch_rename(ui, command, sub_args),
        BranchCommand::Set(sub_args) => cmd_branch_set(ui, command, sub_args),
        BranchCommand::Delete(sub_args) => cmd_branch_delete(ui, command, sub_args),
        BranchCommand::Forget(sub_args) => cmd_branch_forget(ui, command, sub_args),
        BranchCommand::Track(sub_args) => cmd_branch_track(ui, command, sub_args),
        BranchCommand::Untrack(sub_args) => cmd_branch_untrack(ui, command, sub_args),
        BranchCommand::List(sub_args) => cmd_branch_list(ui, command, sub_args),
    }
}

fn cmd_branch_create(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &BranchCreateArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;
    let target_commit =
        workspace_command.resolve_single_rev(args.revision.as_ref().unwrap_or(&RevisionArg::AT))?;
    let view = workspace_command.repo().view();
    let branch_names = &args.names;
    if let Some(branch_name) = branch_names
        .iter()
        .find(|&name| view.get_local_branch(name).is_present())
    {
        return Err(user_error_with_hint(
            format!("Branch already exists: {branch_name}"),
            "Use `jj branch set` to update it.",
        ));
    }

    if branch_names.len() > 1 {
        writeln!(
            ui.warning_default(),
            "Creating multiple branches: {}",
            branch_names.join(", "),
        )?;
    }

    let mut tx = workspace_command.start_transaction();
    for branch_name in branch_names {
        tx.mut_repo()
            .set_local_branch_target(branch_name, RefTarget::normal(target_commit.id().clone()));
    }
    tx.finish(
        ui,
        format!(
            "create {} pointing to commit {}",
            make_branch_term(branch_names),
            target_commit.id().hex()
        ),
    )?;
    Ok(())
}

fn cmd_branch_rename(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &BranchRenameArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;
    let view = workspace_command.repo().view();
    let old_branch = &args.old;
    let ref_target = view.get_local_branch(old_branch).clone();
    if ref_target.is_absent() {
        return Err(user_error(format!("No such branch: {old_branch}")));
    }

    let new_branch = &args.new;
    if view.get_local_branch(new_branch).is_present() {
        return Err(user_error(format!("Branch already exists: {new_branch}")));
    }

    let mut tx = workspace_command.start_transaction();
    tx.mut_repo()
        .set_local_branch_target(new_branch, ref_target);
    tx.mut_repo()
        .set_local_branch_target(old_branch, RefTarget::absent());
    tx.finish(
        ui,
        format!(
            "rename {} to {}",
            make_branch_term(&[old_branch]),
            make_branch_term(&[new_branch]),
        ),
    )?;

    let view = workspace_command.repo().view();
    if view
        .remote_branches_matching(
            &StringPattern::exact(old_branch),
            &StringPattern::everything(),
        )
        .any(|(_, remote_ref)| remote_ref.is_tracking())
    {
        writeln!(
            ui.warning_default(),
            "Branch {old_branch} has tracking remote branches which were not renamed."
        )?;
        if let Some(mut writer) = ui.hint_default() {
            writeln!(
                writer,
                "to rename the branch on the remote, you can `jj git push --branch {old_branch}` \
                 first (to delete it on the remote), and then `jj git push --branch \
                 {new_branch}`. `jj git push --all` would also be sufficient."
            )?;
        }
    }

    Ok(())
}

fn cmd_branch_set(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &BranchSetArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;
    let target_commit =
        workspace_command.resolve_single_rev(args.revision.as_ref().unwrap_or(&RevisionArg::AT))?;
    let repo = workspace_command.repo().as_ref();
    let is_fast_forward = |old_target: &RefTarget| {
        // Strictly speaking, "all" old targets should be ancestors, but we allow
        // conflict resolution by setting branch to "any" of the old target descendants.
        old_target
            .added_ids()
            .any(|old| repo.index().is_ancestor(old, target_commit.id()))
    };
    let branch_names = &args.names;
    for name in branch_names {
        let old_target = repo.view().get_local_branch(name);
        if old_target.is_absent() {
            return Err(user_error_with_hint(
                format!("No such branch: {name}"),
                "Use `jj branch create` to create it.",
            ));
        }
        if !args.allow_backwards && !is_fast_forward(old_target) {
            return Err(user_error_with_hint(
                format!("Refusing to move branch backwards or sideways: {name}"),
                "Use --allow-backwards to allow it.",
            ));
        }
    }

    if branch_names.len() > 1 {
        writeln!(
            ui.warning_default(),
            "Updating multiple branches: {}",
            branch_names.join(", "),
        )?;
    }

    let mut tx = workspace_command.start_transaction();
    for branch_name in branch_names {
        tx.mut_repo()
            .set_local_branch_target(branch_name, RefTarget::normal(target_commit.id().clone()));
    }
    tx.finish(
        ui,
        format!(
            "point {} to commit {}",
            make_branch_term(branch_names),
            target_commit.id().hex()
        ),
    )?;
    Ok(())
}

fn find_local_branches(
    view: &View,
    name_patterns: &[StringPattern],
) -> Result<Vec<String>, CommandError> {
    find_branches_with(name_patterns, |pattern| {
        view.local_branches_matching(pattern)
            .map(|(name, _)| name.to_owned())
    })
}

fn find_forgettable_branches(
    view: &View,
    name_patterns: &[StringPattern],
) -> Result<Vec<String>, CommandError> {
    find_branches_with(name_patterns, |pattern| {
        view.branches()
            .filter(|(name, _)| pattern.matches(name))
            .map(|(name, _)| name.to_owned())
    })
}

fn find_branches_with<'a, I: Iterator<Item = String>>(
    name_patterns: &'a [StringPattern],
    mut find_matches: impl FnMut(&'a StringPattern) -> I,
) -> Result<Vec<String>, CommandError> {
    let mut matching_branches: Vec<String> = vec![];
    let mut unmatched_patterns = vec![];
    for pattern in name_patterns {
        let mut names = find_matches(pattern).peekable();
        if names.peek().is_none() {
            unmatched_patterns.push(pattern);
        }
        matching_branches.extend(names);
    }
    match &unmatched_patterns[..] {
        [] => {
            matching_branches.sort_unstable();
            matching_branches.dedup();
            Ok(matching_branches)
        }
        [pattern] if pattern.is_exact() => Err(user_error(format!("No such branch: {pattern}"))),
        patterns => Err(user_error(format!(
            "No matching branches for patterns: {}",
            patterns.iter().join(", ")
        ))),
    }
}

fn find_remote_branches<'a>(
    view: &'a View,
    name_patterns: &[RemoteBranchNamePattern],
) -> Result<Vec<(RemoteBranchName, &'a RemoteRef)>, CommandError> {
    let mut matching_branches = vec![];
    let mut unmatched_patterns = vec![];
    for pattern in name_patterns {
        let mut matches = view
            .remote_branches_matching(&pattern.branch, &pattern.remote)
            .map(|((branch, remote), remote_ref)| {
                let name = RemoteBranchName {
                    branch: branch.to_owned(),
                    remote: remote.to_owned(),
                };
                (name, remote_ref)
            })
            .peekable();
        if matches.peek().is_none() {
            unmatched_patterns.push(pattern);
        }
        matching_branches.extend(matches);
    }
    match &unmatched_patterns[..] {
        [] => {
            matching_branches.sort_unstable_by(|(name1, _), (name2, _)| name1.cmp(name2));
            matching_branches.dedup_by(|(name1, _), (name2, _)| name1 == name2);
            Ok(matching_branches)
        }
        [pattern] if pattern.is_exact() => {
            Err(user_error(format!("No such remote branch: {pattern}")))
        }
        patterns => Err(user_error(format!(
            "No matching remote branches for patterns: {}",
            patterns.iter().join(", ")
        ))),
    }
}

fn cmd_branch_delete(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &BranchDeleteArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;
    let view = workspace_command.repo().view();
    if !args.glob.is_empty() {
        writeln!(
            ui.warning_default(),
            "--glob has been deprecated. Please prefix the pattern with `glob:` instead."
        )?;
    }
    let name_patterns = [&args.names[..], &args.glob[..]].concat();
    let names = find_local_branches(view, &name_patterns)?;
    let mut tx = workspace_command.start_transaction();
    for branch_name in names.iter() {
        tx.mut_repo()
            .set_local_branch_target(branch_name, RefTarget::absent());
    }
    tx.finish(ui, format!("delete {}", make_branch_term(&names)))?;
    if names.len() > 1 {
        writeln!(ui.status(), "Deleted {} branches.", names.len())?;
    }
    Ok(())
}

fn cmd_branch_forget(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &BranchForgetArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;
    let view = workspace_command.repo().view();
    if !args.glob.is_empty() {
        writeln!(
            ui.warning_default(),
            "--glob has been deprecated. Please prefix the pattern with `glob:` instead."
        )?;
    }
    let name_patterns = [&args.names[..], &args.glob[..]].concat();
    let names = find_forgettable_branches(view, &name_patterns)?;
    let mut tx = workspace_command.start_transaction();
    for branch_name in names.iter() {
        tx.mut_repo().remove_branch(branch_name);
    }
    tx.finish(ui, format!("forget {}", make_branch_term(&names)))?;
    if names.len() > 1 {
        writeln!(ui.status(), "Forgot {} branches.", names.len())?;
    }
    Ok(())
}

fn cmd_branch_track(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &BranchTrackArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;
    let view = workspace_command.repo().view();
    let mut names = Vec::new();
    for (name, remote_ref) in find_remote_branches(view, &args.names)? {
        if remote_ref.is_tracking() {
            writeln!(
                ui.warning_default(),
                "Remote branch already tracked: {name}"
            )?;
        } else {
            names.push(name);
        }
    }
    let mut tx = workspace_command.start_transaction();
    for name in &names {
        tx.mut_repo()
            .track_remote_branch(&name.branch, &name.remote);
    }
    tx.finish(ui, format!("track remote {}", make_branch_term(&names)))?;
    if names.len() > 1 {
        writeln!(
            ui.status(),
            "Started tracking {} remote branches.",
            names.len()
        )?;
    }

    //show conflicted branches if there are some

    if let Some(mut formatter) = ui.status_formatter() {
        let template = {
            let language = workspace_command.commit_template_language()?;
            let text = command
                .settings()
                .config()
                .get::<String>("templates.branch_list")?;
            workspace_command
                .parse_template(&language, &text, CommitTemplateLanguage::wrap_ref_name)?
                .labeled("branch_list")
        };

        let mut remote_per_branch: HashMap<&str, Vec<&str>> = HashMap::new();
        for n in names.iter() {
            remote_per_branch
                .entry(&n.branch)
                .or_default()
                .push(&n.remote);
        }
        let branches_to_list =
            workspace_command
                .repo()
                .view()
                .branches()
                .filter(|(name, target)| {
                    remote_per_branch.contains_key(name) && target.local_target.has_conflict()
                });

        for (name, branch_target) in branches_to_list {
            let local_target = branch_target.local_target;
            let ref_name = RefName::local(
                name,
                local_target.clone(),
                branch_target.remote_refs.iter().map(|x| x.1),
            );
            template.format(&ref_name, formatter.as_mut())?;

            for (remote_name, remote_ref) in branch_target.remote_refs {
                if remote_per_branch[name].contains(&remote_name) {
                    let ref_name =
                        RefName::remote(name, remote_name, remote_ref.clone(), local_target);
                    template.format(&ref_name, formatter.as_mut())?;
                }
            }
        }
    }
    Ok(())
}

fn cmd_branch_untrack(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &BranchUntrackArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;
    let view = workspace_command.repo().view();
    let mut names = Vec::new();
    for (name, remote_ref) in find_remote_branches(view, &args.names)? {
        if name.remote == git::REMOTE_NAME_FOR_LOCAL_GIT_REPO {
            // This restriction can be lifted if we want to support untracked @git branches.
            writeln!(
                ui.warning_default(),
                "Git-tracking branch cannot be untracked: {name}"
            )?;
        } else if !remote_ref.is_tracking() {
            writeln!(
                ui.warning_default(),
                "Remote branch not tracked yet: {name}"
            )?;
        } else {
            names.push(name);
        }
    }
    let mut tx = workspace_command.start_transaction();
    for name in &names {
        tx.mut_repo()
            .untrack_remote_branch(&name.branch, &name.remote);
    }
    tx.finish(ui, format!("untrack remote {}", make_branch_term(&names)))?;
    if names.len() > 1 {
        writeln!(
            ui.status(),
            "Stopped tracking {} remote branches.",
            names.len()
        )?;
    }
    Ok(())
}

fn cmd_branch_list(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &BranchListArgs,
) -> Result<(), CommandError> {
    let workspace_command = command.workspace_helper(ui)?;
    let repo = workspace_command.repo();
    let view = repo.view();

    // Like cmd_git_push(), names and revisions are OR-ed.
    let branch_names_to_list = if !args.names.is_empty() || !args.revisions.is_empty() {
        let mut branch_names: HashSet<&str> = HashSet::new();
        if !args.names.is_empty() {
            branch_names.extend(
                view.branches()
                    .filter(|&(name, _)| args.names.iter().any(|pattern| pattern.matches(name)))
                    .map(|(name, _)| name),
            );
        }
        if !args.revisions.is_empty() {
            // Match against local targets only, which is consistent with "jj git push".
            let mut expression = workspace_command.parse_union_revsets(&args.revisions)?;
            // Intersects with the set of local branch targets to minimize the lookup space.
            expression.intersect_with(&RevsetExpression::branches(StringPattern::everything()));
            let filtered_targets: HashSet<_> = expression.evaluate_to_commit_ids()?.collect();
            branch_names.extend(
                view.local_branches()
                    .filter(|(_, target)| {
                        target.added_ids().any(|id| filtered_targets.contains(id))
                    })
                    .map(|(name, _)| name),
            );
        }
        Some(branch_names)
    } else {
        None
    };

    let template = {
        let language = workspace_command.commit_template_language()?;
        let text = match &args.template {
            Some(value) => value.to_owned(),
            None => command.settings().config().get("templates.branch_list")?,
        };
        workspace_command
            .parse_template(&language, &text, CommitTemplateLanguage::wrap_ref_name)?
            .labeled("branch_list")
    };

    ui.request_pager();
    let mut formatter = ui.stdout_formatter();

    let mut found_deleted_local_branch = false;
    let mut found_deleted_tracking_local_branch = false;
    let branches_to_list = view.branches().filter(|(name, target)| {
        branch_names_to_list
            .as_ref()
            .map_or(true, |branch_names| branch_names.contains(name))
            && (!args.conflicted || target.local_target.has_conflict())
    });
    for (name, branch_target) in branches_to_list {
        let local_target = branch_target.local_target;
        let remote_refs = branch_target.remote_refs;
        let (mut tracking_remote_refs, untracked_remote_refs) = remote_refs
            .iter()
            .copied()
            .partition::<Vec<_>, _>(|&(_, remote_ref)| remote_ref.is_tracking());

        if args.tracked {
            tracking_remote_refs
                .retain(|&(remote, _)| remote != git::REMOTE_NAME_FOR_LOCAL_GIT_REPO);
        } else if !args.all_remotes {
            tracking_remote_refs.retain(|&(_, remote_ref)| remote_ref.target != *local_target);
        }

        if !args.tracked && local_target.is_present() || !tracking_remote_refs.is_empty() {
            let ref_name = RefName::local(
                name,
                local_target.clone(),
                remote_refs.iter().map(|&(_, remote_ref)| remote_ref),
            );
            template.format(&ref_name, formatter.as_mut())?;
        }

        for &(remote, remote_ref) in &tracking_remote_refs {
            let ref_name = RefName::remote(name, remote, remote_ref.clone(), local_target);
            template.format(&ref_name, formatter.as_mut())?;
        }

        if local_target.is_absent() && !tracking_remote_refs.is_empty() {
            found_deleted_local_branch = true;
            found_deleted_tracking_local_branch |= tracking_remote_refs
                .iter()
                .any(|&(remote, _)| remote != git::REMOTE_NAME_FOR_LOCAL_GIT_REPO);
        }

        if args.all_remotes {
            for &(remote, remote_ref) in &untracked_remote_refs {
                let ref_name = RefName::remote_only(name, remote, remote_ref.target.clone());
                template.format(&ref_name, formatter.as_mut())?;
            }
        }
    }

    drop(formatter);

    // Print only one of these hints. It's not important to mention unexported
    // branches, but user might wonder why deleted branches are still listed.
    if found_deleted_tracking_local_branch {
        if let Some(mut writer) = ui.hint_default() {
            writeln!(
                writer,
                "Branches marked as deleted will be *deleted permanently* on the remote on the \
                 next `jj git push`. Use `jj branch forget` to prevent this."
            )?;
        }
    } else if found_deleted_local_branch {
        if let Some(mut writer) = ui.hint_default() {
            writeln!(
                writer,
                "Branches marked as deleted will be deleted from the underlying Git repo on the \
                 next `jj git export`."
            )?;
        }
    }

    Ok(())
}
