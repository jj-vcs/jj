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

#![expect(missing_docs)]

use std::any::Any;
use std::collections::HashMap;
use std::collections::hash_map;
use std::convert::Infallible;
use std::fmt;
use std::ops::ControlFlow;
use std::ops::Range;
use std::sync::Arc;
use std::sync::LazyLock;

use itertools::Itertools as _;
use thiserror::Error;

use crate::backend::BackendError;
use crate::backend::ChangeId;
use crate::backend::CommitId;
use crate::commit::Commit;
use crate::dsl_util;
use crate::dsl_util::collect_similar;
use crate::fileset;
use crate::fileset::FilesetDiagnostics;
use crate::fileset::FilesetExpression;
use crate::graph::GraphNode;
use crate::id_prefix::IdPrefixContext;
use crate::id_prefix::IdPrefixIndex;
use crate::object_id::HexPrefix;
use crate::object_id::PrefixResolution;
use crate::op_store::RefTarget;
use crate::op_store::RemoteRefState;
use crate::op_walk;
use crate::ref_name::RemoteName;
use crate::ref_name::RemoteRefSymbol;
use crate::ref_name::RemoteRefSymbolBuf;
use crate::ref_name::WorkspaceName;
use crate::ref_name::WorkspaceNameBuf;
use crate::repo::ReadonlyRepo;
use crate::repo::Repo;
use crate::repo::RepoLoaderError;
use crate::repo_path::RepoPathUiConverter;
use crate::revset_parser;
pub use crate::revset_parser::BinaryOp;
pub use crate::revset_parser::ExpressionKind;
pub use crate::revset_parser::ExpressionNode;
pub use crate::revset_parser::FunctionCallNode;
pub use crate::revset_parser::RevsetAliasesMap;
pub use crate::revset_parser::RevsetDiagnostics;
pub use crate::revset_parser::RevsetParseError;
pub use crate::revset_parser::RevsetParseErrorKind;
pub use crate::revset_parser::UnaryOp;
pub use crate::revset_parser::expect_literal;
pub use crate::revset_parser::parse_program;
pub use crate::revset_parser::parse_program_with_modifier;
pub use crate::revset_parser::parse_symbol;
use crate::store::Store;
use crate::str_util::StringExpression;
use crate::str_util::StringPattern;
use crate::time_util::DatePattern;
use crate::time_util::DatePatternContext;

/// Error occurred during symbol resolution.
#[derive(Debug, Error)]
pub enum RevsetResolutionError {
    #[error("Revision `{name}` doesn't exist")]
    NoSuchRevision {
        name: String,
        candidates: Vec<String>,
    },
    #[error("Workspace `{}` doesn't have a working-copy commit", name.as_symbol())]
    WorkspaceMissingWorkingCopy { name: WorkspaceNameBuf },
    #[error("An empty string is not a valid revision")]
    EmptyString,
    #[error("Commit ID prefix `{0}` is ambiguous")]
    AmbiguousCommitIdPrefix(String),
    #[error("Change ID prefix `{0}` is ambiguous")]
    AmbiguousChangeIdPrefix(String),
    #[error("Change ID `{symbol}` is divergent")]
    DivergentChangeId {
        symbol: String,
        targets: Vec<CommitId>,
    },
    #[error("Name `{symbol}` is conflicted")]
    ConflictedRef {
        kind: &'static str,
        symbol: String,
        targets: Vec<CommitId>,
    },
    #[error("Unexpected error from commit backend")]
    Backend(#[source] BackendError),
    #[error(transparent)]
    Other(#[from] Box<dyn std::error::Error + Send + Sync>),
}

/// Error occurred during revset evaluation.
#[derive(Debug, Error)]
pub enum RevsetEvaluationError {
    #[error("Unexpected error from commit backend")]
    Backend(#[from] BackendError),
    #[error(transparent)]
    Other(Box<dyn std::error::Error + Send + Sync>),
}

impl RevsetEvaluationError {
    // TODO: Create a higher-level error instead of putting non-BackendErrors in a
    // BackendError
    pub fn into_backend_error(self) -> BackendError {
        match self {
            Self::Backend(err) => err,
            Self::Other(err) => BackendError::Other(err),
        }
    }
}

// assumes index has less than u64::MAX entries.
pub const GENERATION_RANGE_FULL: Range<u64> = 0..u64::MAX;
pub const GENERATION_RANGE_EMPTY: Range<u64> = 0..0;

pub const PARENTS_RANGE_FULL: Range<u32> = 0..u32::MAX;

/// Global flag applied to the entire expression.
///
/// The core revset engine doesn't use this value. It's up to caller to
/// interpret it to change the evaluation behavior.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RevsetModifier {
    /// Expression can be evaluated to multiple revisions even if a single
    /// revision is expected by default.
    All,
}

/// Symbol or function to be resolved to `CommitId`s.
#[derive(Clone, Debug)]
pub enum RevsetCommitRef {
    WorkingCopy(WorkspaceNameBuf),
    WorkingCopies,
    Symbol(String),
    RemoteSymbol(RemoteRefSymbolBuf),
    ChangeId(HexPrefix),
    CommitId(HexPrefix),
    Bookmarks(StringExpression),
    RemoteBookmarks {
        bookmark: StringExpression,
        remote: StringExpression,
        remote_ref_state: Option<RemoteRefState>,
    },
    Tags(StringExpression),
    GitRefs,
    GitHead,
}

/// A custom revset filter expression, defined by an extension.
pub trait RevsetFilterExtension: std::fmt::Debug + Any + Send + Sync {
    /// Returns true iff this filter matches the specified commit.
    fn matches_commit(&self, commit: &Commit) -> bool;
}

impl dyn RevsetFilterExtension {
    /// Returns reference of the implementation type.
    pub fn downcast_ref<T: RevsetFilterExtension>(&self) -> Option<&T> {
        (self as &dyn Any).downcast_ref()
    }
}

#[derive(Clone, Debug)]
pub enum RevsetFilterPredicate {
    /// Commits with number of parents in the range.
    ParentCount(Range<u32>),
    /// Commits with description matching the pattern.
    Description(StringExpression),
    /// Commits with first line of the description matching the pattern.
    Subject(StringExpression),
    /// Commits with author name matching the pattern.
    AuthorName(StringExpression),
    /// Commits with author email matching the pattern.
    AuthorEmail(StringExpression),
    /// Commits with author dates matching the given date pattern.
    AuthorDate(DatePattern),
    /// Commits with committer name matching the pattern.
    CommitterName(StringExpression),
    /// Commits with committer email matching the pattern.
    CommitterEmail(StringExpression),
    /// Commits with committer dates matching the given date pattern.
    CommitterDate(DatePattern),
    /// Commits modifying the paths specified by the fileset.
    File(FilesetExpression),
    /// Commits containing diffs matching the `text` pattern within the `files`.
    DiffContains {
        text: StringExpression,
        files: FilesetExpression,
    },
    /// Commits with conflicts
    HasConflict,
    /// Commits that are cryptographically signed.
    Signed,
    /// Custom predicates provided by extensions
    Extension(Arc<dyn RevsetFilterExtension>),
}

mod private {
    /// Defines [`RevsetExpression`] variants depending on resolution state.
    pub trait ExpressionState {
        type CommitRef: Clone;
        type Operation: Clone;
    }

    // Not constructible because these state types just define associated types.
    #[derive(Debug)]
    pub enum UserExpressionState {}
    #[derive(Debug)]
    pub enum ResolvedExpressionState {}
}

use private::ExpressionState;
use private::ResolvedExpressionState;
use private::UserExpressionState;

impl ExpressionState for UserExpressionState {
    type CommitRef = RevsetCommitRef;
    type Operation = String;
}

impl ExpressionState for ResolvedExpressionState {
    type CommitRef = Infallible;
    type Operation = Infallible;
}

/// [`RevsetExpression`] that may contain unresolved commit refs.
pub type UserRevsetExpression = RevsetExpression<UserExpressionState>;
/// [`RevsetExpression`] that never contains unresolved commit refs.
pub type ResolvedRevsetExpression = RevsetExpression<ResolvedExpressionState>;

/// Tree of revset expressions describing DAG operations.
///
/// Use [`UserRevsetExpression`] or [`ResolvedRevsetExpression`] to construct
/// expression of that state.
#[derive(Clone, Debug)]
pub enum RevsetExpression<St: ExpressionState> {
    None,
    All,
    VisibleHeads,
    /// Visible heads and all referenced commits within the current expression
    /// scope. Used as the default of `Range`/`DagRange` heads.
    VisibleHeadsOrReferenced,
    Root,
    Commits(Vec<CommitId>),
    CommitRef(St::CommitRef),
    Ancestors {
        heads: Arc<Self>,
        generation: Range<u64>,
        parents_range: Range<u32>,
    },
    Descendants {
        roots: Arc<Self>,
        generation: Range<u64>,
    },
    // Commits that are ancestors of "heads" but not ancestors of "roots"
    Range {
        roots: Arc<Self>,
        heads: Arc<Self>,
        generation: Range<u64>,
        // Parents range is only used for traversing heads, not roots
        parents_range: Range<u32>,
    },
    // Commits that are descendants of "roots" and ancestors of "heads"
    DagRange {
        roots: Arc<Self>,
        heads: Arc<Self>,
        // TODO: maybe add generation_from_roots/heads?
    },
    // Commits reachable from "sources" within "domain"
    Reachable {
        sources: Arc<Self>,
        domain: Arc<Self>,
    },
    Heads(Arc<Self>),
    /// Heads of the set of commits which are ancestors of `heads` but are not
    /// ancestors of `roots`, and which also are contained in `filter`.
    HeadsRange {
        roots: Arc<Self>,
        heads: Arc<Self>,
        parents_range: Range<u32>,
        filter: Arc<Self>,
    },
    Roots(Arc<Self>),
    ForkPoint(Arc<Self>),
    Bisect(Arc<Self>),
    HasSize {
        candidates: Arc<Self>,
        count: usize,
    },
    Latest {
        candidates: Arc<Self>,
        count: usize,
    },
    Filter(RevsetFilterPredicate),
    /// Marker for subtree that should be intersected as filter.
    AsFilter(Arc<Self>),
    /// Resolves symbols and visibility at the specified operation.
    AtOperation {
        operation: St::Operation,
        candidates: Arc<Self>,
    },
    /// Makes `All` include the commits and their ancestors in addition to the
    /// visible heads.
    WithinReference {
        candidates: Arc<Self>,
        /// Commits explicitly referenced within the scope.
        commits: Vec<CommitId>,
    },
    /// Resolves visibility within the specified repo state.
    WithinVisibility {
        candidates: Arc<Self>,
        /// Copy of `repo.view().heads()` at the operation.
        visible_heads: Vec<CommitId>,
    },
    Coalesce(Arc<Self>, Arc<Self>),
    Present(Arc<Self>),
    NotIn(Arc<Self>),
    Union(Arc<Self>, Arc<Self>),
    Intersection(Arc<Self>, Arc<Self>),
    Difference(Arc<Self>, Arc<Self>),
}

// Leaf expression that never contains unresolved commit refs, which can be
// either user or resolved expression
impl<St: ExpressionState> RevsetExpression<St> {
    pub fn none() -> Arc<Self> {
        Arc::new(Self::None)
    }

    /// Ancestors of visible heads and all referenced commits within the current
    /// expression scope, which may include hidden commits.
    pub fn all() -> Arc<Self> {
        Arc::new(Self::All)
    }

    pub fn visible_heads() -> Arc<Self> {
        Arc::new(Self::VisibleHeads)
    }

    fn visible_heads_or_referenced() -> Arc<Self> {
        Arc::new(Self::VisibleHeadsOrReferenced)
    }

    pub fn root() -> Arc<Self> {
        Arc::new(Self::Root)
    }

    pub fn commit(commit_id: CommitId) -> Arc<Self> {
        Self::commits(vec![commit_id])
    }

    pub fn commits(commit_ids: Vec<CommitId>) -> Arc<Self> {
        Arc::new(Self::Commits(commit_ids))
    }

    pub fn filter(predicate: RevsetFilterPredicate) -> Arc<Self> {
        Arc::new(Self::Filter(predicate))
    }

    /// Find any empty commits.
    pub fn is_empty() -> Arc<Self> {
        Self::filter(RevsetFilterPredicate::File(FilesetExpression::all())).negated()
    }
}

// Leaf expression that represents unresolved commit refs
impl<St: ExpressionState<CommitRef = RevsetCommitRef>> RevsetExpression<St> {
    pub fn working_copy(name: WorkspaceNameBuf) -> Arc<Self> {
        Arc::new(Self::CommitRef(RevsetCommitRef::WorkingCopy(name)))
    }

    pub fn working_copies() -> Arc<Self> {
        Arc::new(Self::CommitRef(RevsetCommitRef::WorkingCopies))
    }

    pub fn symbol(value: String) -> Arc<Self> {
        Arc::new(Self::CommitRef(RevsetCommitRef::Symbol(value)))
    }

    pub fn remote_symbol(value: RemoteRefSymbolBuf) -> Arc<Self> {
        let commit_ref = RevsetCommitRef::RemoteSymbol(value);
        Arc::new(Self::CommitRef(commit_ref))
    }

    pub fn change_id_prefix(prefix: HexPrefix) -> Arc<Self> {
        let commit_ref = RevsetCommitRef::ChangeId(prefix);
        Arc::new(Self::CommitRef(commit_ref))
    }

    pub fn commit_id_prefix(prefix: HexPrefix) -> Arc<Self> {
        let commit_ref = RevsetCommitRef::CommitId(prefix);
        Arc::new(Self::CommitRef(commit_ref))
    }

    pub fn bookmarks(expression: StringExpression) -> Arc<Self> {
        Arc::new(Self::CommitRef(RevsetCommitRef::Bookmarks(expression)))
    }

    pub fn remote_bookmarks(
        bookmark: StringExpression,
        remote: StringExpression,
        remote_ref_state: Option<RemoteRefState>,
    ) -> Arc<Self> {
        Arc::new(Self::CommitRef(RevsetCommitRef::RemoteBookmarks {
            bookmark,
            remote,
            remote_ref_state,
        }))
    }

    pub fn tags(expression: StringExpression) -> Arc<Self> {
        Arc::new(Self::CommitRef(RevsetCommitRef::Tags(expression)))
    }

    pub fn git_refs() -> Arc<Self> {
        Arc::new(Self::CommitRef(RevsetCommitRef::GitRefs))
    }

    pub fn git_head() -> Arc<Self> {
        Arc::new(Self::CommitRef(RevsetCommitRef::GitHead))
    }
}

// Compound expression
impl<St: ExpressionState> RevsetExpression<St> {
    pub fn latest(self: &Arc<Self>, count: usize) -> Arc<Self> {
        Arc::new(Self::Latest {
            candidates: self.clone(),
            count,
        })
    }

    /// Commits in `self` that don't have descendants in `self`.
    pub fn heads(self: &Arc<Self>) -> Arc<Self> {
        Arc::new(Self::Heads(self.clone()))
    }

    /// Commits in `self` that don't have ancestors in `self`.
    pub fn roots(self: &Arc<Self>) -> Arc<Self> {
        Arc::new(Self::Roots(self.clone()))
    }

    /// Parents of `self`.
    pub fn parents(self: &Arc<Self>) -> Arc<Self> {
        self.ancestors_at(1)
    }

    /// Ancestors of `self`, including `self`.
    pub fn ancestors(self: &Arc<Self>) -> Arc<Self> {
        self.ancestors_range(GENERATION_RANGE_FULL)
    }

    /// Ancestors of `self` at an offset of `generation` behind `self`.
    /// The `generation` offset is zero-based starting from `self`.
    pub fn ancestors_at(self: &Arc<Self>, generation: u64) -> Arc<Self> {
        self.ancestors_range(generation..generation.saturating_add(1))
    }

    /// Ancestors of `self` in the given range.
    pub fn ancestors_range(self: &Arc<Self>, generation_range: Range<u64>) -> Arc<Self> {
        Arc::new(Self::Ancestors {
            heads: self.clone(),
            generation: generation_range,
            parents_range: PARENTS_RANGE_FULL,
        })
    }

    /// First-parent ancestors of `self`, including `self`.
    pub fn first_ancestors(self: &Arc<Self>) -> Arc<Self> {
        self.first_ancestors_range(GENERATION_RANGE_FULL)
    }

    /// First-parent ancestors of `self` at an offset of `generation` behind
    /// `self`. The `generation` offset is zero-based starting from `self`.
    pub fn first_ancestors_at(self: &Arc<Self>, generation: u64) -> Arc<Self> {
        self.first_ancestors_range(generation..generation.saturating_add(1))
    }

    /// First-parent ancestors of `self` in the given range.
    pub fn first_ancestors_range(self: &Arc<Self>, generation_range: Range<u64>) -> Arc<Self> {
        Arc::new(Self::Ancestors {
            heads: self.clone(),
            generation: generation_range,
            parents_range: 0..1,
        })
    }

    /// Children of `self`.
    pub fn children(self: &Arc<Self>) -> Arc<Self> {
        self.descendants_at(1)
    }

    /// Descendants of `self`, including `self`.
    pub fn descendants(self: &Arc<Self>) -> Arc<Self> {
        self.descendants_range(GENERATION_RANGE_FULL)
    }

    /// Descendants of `self` at an offset of `generation` ahead of `self`.
    /// The `generation` offset is zero-based starting from `self`.
    pub fn descendants_at(self: &Arc<Self>, generation: u64) -> Arc<Self> {
        self.descendants_range(generation..generation.saturating_add(1))
    }

    /// Descendants of `self` in the given range.
    pub fn descendants_range(self: &Arc<Self>, generation_range: Range<u64>) -> Arc<Self> {
        Arc::new(Self::Descendants {
            roots: self.clone(),
            generation: generation_range,
        })
    }

    /// Fork point (best common ancestors) of `self`.
    pub fn fork_point(self: &Arc<Self>) -> Arc<Self> {
        Arc::new(Self::ForkPoint(self.clone()))
    }

    /// Commits with ~half of the descendants in `self`.
    pub fn bisect(self: &Arc<Self>) -> Arc<Self> {
        Arc::new(Self::Bisect(self.clone()))
    }

    /// Commits in `self`, the number of which must be exactly equal to `count`.
    pub fn has_size(self: &Arc<Self>, count: usize) -> Arc<Self> {
        Arc::new(Self::HasSize {
            candidates: self.clone(),
            count,
        })
    }

    /// Filter all commits by `predicate` in `self`.
    pub fn filtered(self: &Arc<Self>, predicate: RevsetFilterPredicate) -> Arc<Self> {
        self.intersection(&Self::filter(predicate))
    }

    /// Commits that are descendants of `self` and ancestors of `heads`, both
    /// inclusive.
    pub fn dag_range_to(self: &Arc<Self>, heads: &Arc<Self>) -> Arc<Self> {
        Arc::new(Self::DagRange {
            roots: self.clone(),
            heads: heads.clone(),
        })
    }

    /// Connects any ancestors and descendants in the set by adding the commits
    /// between them.
    pub fn connected(self: &Arc<Self>) -> Arc<Self> {
        self.dag_range_to(self)
    }

    /// All commits within `domain` reachable from this set of commits, by
    /// traversing either parent or child edges.
    pub fn reachable(self: &Arc<Self>, domain: &Arc<Self>) -> Arc<Self> {
        Arc::new(Self::Reachable {
            sources: self.clone(),
            domain: domain.clone(),
        })
    }

    /// Commits reachable from `heads` but not from `self`.
    pub fn range(self: &Arc<Self>, heads: &Arc<Self>) -> Arc<Self> {
        Arc::new(Self::Range {
            roots: self.clone(),
            heads: heads.clone(),
            generation: GENERATION_RANGE_FULL,
            parents_range: PARENTS_RANGE_FULL,
        })
    }

    /// Suppresses name resolution error within `self`.
    pub fn present(self: &Arc<Self>) -> Arc<Self> {
        Arc::new(Self::Present(self.clone()))
    }

    /// Commits that are not in `self`, i.e. the complement of `self`.
    pub fn negated(self: &Arc<Self>) -> Arc<Self> {
        Arc::new(Self::NotIn(self.clone()))
    }

    /// Commits that are in `self` or in `other` (or both).
    pub fn union(self: &Arc<Self>, other: &Arc<Self>) -> Arc<Self> {
        Arc::new(Self::Union(self.clone(), other.clone()))
    }

    /// Commits that are in any of the `expressions`.
    pub fn union_all(expressions: &[Arc<Self>]) -> Arc<Self> {
        to_binary_expression(expressions, &Self::none, &Self::union)
    }

    /// Commits that are in `self` and in `other`.
    pub fn intersection(self: &Arc<Self>, other: &Arc<Self>) -> Arc<Self> {
        Arc::new(Self::Intersection(self.clone(), other.clone()))
    }

    /// Commits that are in `self` but not in `other`.
    pub fn minus(self: &Arc<Self>, other: &Arc<Self>) -> Arc<Self> {
        Arc::new(Self::Difference(self.clone(), other.clone()))
    }

    /// Commits that are in the first expression in `expressions` that is not
    /// `none()`.
    pub fn coalesce(expressions: &[Arc<Self>]) -> Arc<Self> {
        to_binary_expression(expressions, &Self::none, &Self::coalesce2)
    }

    fn coalesce2(self: &Arc<Self>, other: &Arc<Self>) -> Arc<Self> {
        Arc::new(Self::Coalesce(self.clone(), other.clone()))
    }
}

impl<St: ExpressionState<CommitRef = RevsetCommitRef>> RevsetExpression<St> {
    /// Returns symbol string if this expression is of that type.
    pub fn as_symbol(&self) -> Option<&str> {
        match self {
            Self::CommitRef(RevsetCommitRef::Symbol(name)) => Some(name),
            _ => None,
        }
    }
}

impl UserRevsetExpression {
    /// Resolve a user-provided expression. Symbols will be resolved using the
    /// provided [`SymbolResolver`].
    pub fn resolve_user_expression(
        &self,
        repo: &dyn Repo,
        symbol_resolver: &SymbolResolver,
    ) -> Result<Arc<ResolvedRevsetExpression>, RevsetResolutionError> {
        resolve_symbols(repo, self, symbol_resolver)
    }
}

impl ResolvedRevsetExpression {
    /// Optimizes and evaluates this expression.
    pub fn evaluate<'index>(
        self: Arc<Self>,
        repo: &'index dyn Repo,
    ) -> Result<Box<dyn Revset + 'index>, RevsetEvaluationError> {
        let expr = optimize(self).to_backend_expression(repo);
        repo.index().evaluate_revset(&expr, repo.store())
    }

    /// Evaluates this expression without optimizing it.
    ///
    /// Use this function if `self` is already optimized, or to debug
    /// optimization pass.
    pub fn evaluate_unoptimized<'index>(
        self: &Arc<Self>,
        repo: &'index dyn Repo,
    ) -> Result<Box<dyn Revset + 'index>, RevsetEvaluationError> {
        // Since referenced commits change the evaluation result, they must be
        // collected no matter if optimization is disabled.
        let expr = resolve_referenced_commits(self)
            .as_ref()
            .unwrap_or(self)
            .to_backend_expression(repo);
        repo.index().evaluate_revset(&expr, repo.store())
    }

    /// Transforms this expression to the form which the `Index` backend will
    /// process.
    pub fn to_backend_expression(&self, repo: &dyn Repo) -> ResolvedExpression {
        resolve_visibility(repo, self)
    }
}

#[derive(Clone, Debug)]
pub enum ResolvedPredicateExpression {
    /// Pure filter predicate.
    Filter(RevsetFilterPredicate),
    /// Set expression to be evaluated as filter. This is typically a subtree
    /// node of `Union` with a pure filter predicate.
    Set(Box<ResolvedExpression>),
    NotIn(Box<Self>),
    Union(Box<Self>, Box<Self>),
    Intersection(Box<Self>, Box<Self>),
}

/// Describes evaluation plan of revset expression.
///
/// Unlike `RevsetExpression`, this doesn't contain unresolved symbols or `View`
/// properties.
///
/// Use `RevsetExpression` API to build a query programmatically.
// TODO: rename to BackendExpression?
#[derive(Clone, Debug)]
pub enum ResolvedExpression {
    Commits(Vec<CommitId>),
    Ancestors {
        heads: Box<Self>,
        generation: Range<u64>,
        parents_range: Range<u32>,
    },
    /// Commits that are ancestors of `heads` but not ancestors of `roots`.
    Range {
        roots: Box<Self>,
        heads: Box<Self>,
        generation: Range<u64>,
        // Parents range is only used for traversing heads, not roots
        parents_range: Range<u32>,
    },
    /// Commits that are descendants of `roots` and ancestors of `heads`.
    DagRange {
        roots: Box<Self>,
        heads: Box<Self>,
        generation_from_roots: Range<u64>,
    },
    /// Commits reachable from `sources` within `domain`.
    Reachable {
        sources: Box<Self>,
        domain: Box<Self>,
    },
    Heads(Box<Self>),
    /// Heads of the set of commits which are ancestors of `heads` but are not
    /// ancestors of `roots`, and which also are contained in `filter`.
    HeadsRange {
        roots: Box<Self>,
        heads: Box<Self>,
        parents_range: Range<u32>,
        filter: Option<ResolvedPredicateExpression>,
    },
    Roots(Box<Self>),
    ForkPoint(Box<Self>),
    Bisect(Box<Self>),
    HasSize {
        candidates: Box<Self>,
        count: usize,
    },
    Latest {
        candidates: Box<Self>,
        count: usize,
    },
    Coalesce(Box<Self>, Box<Self>),
    Union(Box<Self>, Box<Self>),
    /// Intersects `candidates` with `predicate` by filtering.
    FilterWithin {
        candidates: Box<Self>,
        predicate: ResolvedPredicateExpression,
    },
    /// Intersects expressions by merging.
    Intersection(Box<Self>, Box<Self>),
    Difference(Box<Self>, Box<Self>),
}

pub type RevsetFunction = fn(
    &mut RevsetDiagnostics,
    &FunctionCallNode,
    &LoweringContext,
) -> Result<Arc<UserRevsetExpression>, RevsetParseError>;

static BUILTIN_FUNCTION_MAP: LazyLock<HashMap<&str, RevsetFunction>> = LazyLock::new(|| {
    // Not using maplit::hashmap!{} or custom declarative macro here because
    // code completion inside macro is quite restricted.
    let mut map: HashMap<&str, RevsetFunction> = HashMap::new();
    map.insert("parents", |diagnostics, function, context| {
        let ([arg], [depth_opt_arg]) = function.expect_arguments()?;
        let expression = lower_expression(diagnostics, arg, context)?;
        if let Some(depth_arg) = depth_opt_arg {
            let depth = expect_literal("integer", depth_arg)?;
            Ok(expression.ancestors_at(depth))
        } else {
            Ok(expression.parents())
        }
    });
    map.insert("children", |diagnostics, function, context| {
        let ([arg], [depth_opt_arg]) = function.expect_arguments()?;
        let expression = lower_expression(diagnostics, arg, context)?;
        if let Some(depth_arg) = depth_opt_arg {
            let depth = expect_literal("integer", depth_arg)?;
            Ok(expression.descendants_at(depth))
        } else {
            Ok(expression.children())
        }
    });
    map.insert("ancestors", |diagnostics, function, context| {
        let ([heads_arg], [depth_opt_arg]) = function.expect_arguments()?;
        let heads = lower_expression(diagnostics, heads_arg, context)?;
        let generation = if let Some(depth_arg) = depth_opt_arg {
            let depth = expect_literal("integer", depth_arg)?;
            0..depth
        } else {
            GENERATION_RANGE_FULL
        };
        Ok(heads.ancestors_range(generation))
    });
    map.insert("descendants", |diagnostics, function, context| {
        let ([roots_arg], [depth_opt_arg]) = function.expect_arguments()?;
        let roots = lower_expression(diagnostics, roots_arg, context)?;
        let generation = if let Some(depth_arg) = depth_opt_arg {
            let depth = expect_literal("integer", depth_arg)?;
            0..depth
        } else {
            GENERATION_RANGE_FULL
        };
        Ok(roots.descendants_range(generation))
    });
    map.insert("first_parent", |diagnostics, function, context| {
        let ([arg], [depth_opt_arg]) = function.expect_arguments()?;
        let expression = lower_expression(diagnostics, arg, context)?;
        let depth = if let Some(depth_arg) = depth_opt_arg {
            expect_literal("integer", depth_arg)?
        } else {
            1
        };
        Ok(expression.first_ancestors_at(depth))
    });
    map.insert("first_ancestors", |diagnostics, function, context| {
        let ([heads_arg], [depth_opt_arg]) = function.expect_arguments()?;
        let heads = lower_expression(diagnostics, heads_arg, context)?;
        let generation = if let Some(depth_arg) = depth_opt_arg {
            let depth = expect_literal("integer", depth_arg)?;
            0..depth
        } else {
            GENERATION_RANGE_FULL
        };
        Ok(heads.first_ancestors_range(generation))
    });
    map.insert("connected", |diagnostics, function, context| {
        let [arg] = function.expect_exact_arguments()?;
        let candidates = lower_expression(diagnostics, arg, context)?;
        Ok(candidates.connected())
    });
    map.insert("reachable", |diagnostics, function, context| {
        let [source_arg, domain_arg] = function.expect_exact_arguments()?;
        let sources = lower_expression(diagnostics, source_arg, context)?;
        let domain = lower_expression(diagnostics, domain_arg, context)?;
        Ok(sources.reachable(&domain))
    });
    map.insert("none", |_diagnostics, function, _context| {
        function.expect_no_arguments()?;
        Ok(RevsetExpression::none())
    });
    map.insert("all", |_diagnostics, function, _context| {
        function.expect_no_arguments()?;
        Ok(RevsetExpression::all())
    });
    map.insert("working_copies", |_diagnostics, function, _context| {
        function.expect_no_arguments()?;
        Ok(RevsetExpression::working_copies())
    });
    map.insert("heads", |diagnostics, function, context| {
        let [arg] = function.expect_exact_arguments()?;
        let candidates = lower_expression(diagnostics, arg, context)?;
        Ok(candidates.heads())
    });
    map.insert("roots", |diagnostics, function, context| {
        let [arg] = function.expect_exact_arguments()?;
        let candidates = lower_expression(diagnostics, arg, context)?;
        Ok(candidates.roots())
    });
    map.insert("visible_heads", |_diagnostics, function, _context| {
        function.expect_no_arguments()?;
        Ok(RevsetExpression::visible_heads())
    });
    map.insert("root", |_diagnostics, function, _context| {
        function.expect_no_arguments()?;
        Ok(RevsetExpression::root())
    });
    map.insert("change_id", |diagnostics, function, _context| {
        let [arg] = function.expect_exact_arguments()?;
        let prefix = revset_parser::catch_aliases(diagnostics, arg, |_diagnostics, arg| {
            let value = revset_parser::expect_string_literal("change ID prefix", arg)?;
            HexPrefix::try_from_reverse_hex(value)
                .ok_or_else(|| RevsetParseError::expression("Invalid change ID prefix", arg.span))
        })?;
        Ok(RevsetExpression::change_id_prefix(prefix))
    });
    map.insert("commit_id", |diagnostics, function, _context| {
        let [arg] = function.expect_exact_arguments()?;
        let prefix = revset_parser::catch_aliases(diagnostics, arg, |_diagnostics, arg| {
            let value = revset_parser::expect_string_literal("commit ID prefix", arg)?;
            HexPrefix::try_from_hex(value)
                .ok_or_else(|| RevsetParseError::expression("Invalid commit ID prefix", arg.span))
        })?;
        Ok(RevsetExpression::commit_id_prefix(prefix))
    });
    map.insert("bookmarks", |diagnostics, function, _context| {
        let ([], [opt_arg]) = function.expect_arguments()?;
        let expr = if let Some(arg) = opt_arg {
            expect_string_expression(diagnostics, arg)?
        } else {
            StringExpression::all()
        };
        Ok(RevsetExpression::bookmarks(expr))
    });
    map.insert("remote_bookmarks", |diagnostics, function, context| {
        parse_remote_bookmarks_arguments(
            diagnostics,
            function,
            None,
            context.default_ignored_remote,
        )
    });
    map.insert(
        "tracked_remote_bookmarks",
        |diagnostics, function, context| {
            parse_remote_bookmarks_arguments(
                diagnostics,
                function,
                Some(RemoteRefState::Tracked),
                context.default_ignored_remote,
            )
        },
    );
    map.insert(
        "untracked_remote_bookmarks",
        |diagnostics, function, context| {
            parse_remote_bookmarks_arguments(
                diagnostics,
                function,
                Some(RemoteRefState::New),
                context.default_ignored_remote,
            )
        },
    );
    map.insert("tags", |diagnostics, function, _context| {
        let ([], [opt_arg]) = function.expect_arguments()?;
        let expr = if let Some(arg) = opt_arg {
            expect_string_expression(diagnostics, arg)?
        } else {
            StringExpression::all()
        };
        Ok(RevsetExpression::tags(expr))
    });
    map.insert("git_refs", |_diagnostics, function, _context| {
        function.expect_no_arguments()?;
        Ok(RevsetExpression::git_refs())
    });
    map.insert("git_head", |_diagnostics, function, _context| {
        function.expect_no_arguments()?;
        Ok(RevsetExpression::git_head())
    });
    map.insert("latest", |diagnostics, function, context| {
        let ([candidates_arg], [count_opt_arg]) = function.expect_arguments()?;
        let candidates = lower_expression(diagnostics, candidates_arg, context)?;
        let count = if let Some(count_arg) = count_opt_arg {
            expect_literal("integer", count_arg)?
        } else {
            1
        };
        Ok(candidates.latest(count))
    });
    map.insert("fork_point", |diagnostics, function, context| {
        let [expression_arg] = function.expect_exact_arguments()?;
        let expression = lower_expression(diagnostics, expression_arg, context)?;
        Ok(RevsetExpression::fork_point(&expression))
    });
    map.insert("bisect", |diagnostics, function, context| {
        let [expression_arg] = function.expect_exact_arguments()?;
        let expression = lower_expression(diagnostics, expression_arg, context)?;
        Ok(RevsetExpression::bisect(&expression))
    });
    map.insert("exactly", |diagnostics, function, context| {
        let ([candidates_arg, count_arg], []) = function.expect_arguments()?;
        let candidates = lower_expression(diagnostics, candidates_arg, context)?;
        let count = expect_literal("integer", count_arg)?;
        Ok(candidates.has_size(count))
    });
    map.insert("merges", |_diagnostics, function, _context| {
        function.expect_no_arguments()?;
        Ok(RevsetExpression::filter(
            RevsetFilterPredicate::ParentCount(2..u32::MAX),
        ))
    });
    map.insert("description", |diagnostics, function, _context| {
        let [arg] = function.expect_exact_arguments()?;
        let expr = expect_string_expression(diagnostics, arg)?;
        let predicate = RevsetFilterPredicate::Description(expr);
        Ok(RevsetExpression::filter(predicate))
    });
    map.insert("subject", |diagnostics, function, _context| {
        let [arg] = function.expect_exact_arguments()?;
        let expr = expect_string_expression(diagnostics, arg)?;
        let predicate = RevsetFilterPredicate::Subject(expr);
        Ok(RevsetExpression::filter(predicate))
    });
    map.insert("author", |diagnostics, function, _context| {
        let [arg] = function.expect_exact_arguments()?;
        let expr = expect_string_expression(diagnostics, arg)?;
        let name_predicate = RevsetFilterPredicate::AuthorName(expr.clone());
        let email_predicate = RevsetFilterPredicate::AuthorEmail(expr);
        Ok(RevsetExpression::filter(name_predicate)
            .union(&RevsetExpression::filter(email_predicate)))
    });
    map.insert("author_name", |diagnostics, function, _context| {
        let [arg] = function.expect_exact_arguments()?;
        let expr = expect_string_expression(diagnostics, arg)?;
        let predicate = RevsetFilterPredicate::AuthorName(expr);
        Ok(RevsetExpression::filter(predicate))
    });
    map.insert("author_email", |diagnostics, function, _context| {
        let [arg] = function.expect_exact_arguments()?;
        let expr = expect_string_expression(diagnostics, arg)?;
        let predicate = RevsetFilterPredicate::AuthorEmail(expr);
        Ok(RevsetExpression::filter(predicate))
    });
    map.insert("author_date", |diagnostics, function, context| {
        let [arg] = function.expect_exact_arguments()?;
        let pattern = expect_date_pattern(diagnostics, arg, context.date_pattern_context())?;
        Ok(RevsetExpression::filter(RevsetFilterPredicate::AuthorDate(
            pattern,
        )))
    });
    map.insert("signed", |_diagnostics, function, _context| {
        function.expect_no_arguments()?;
        let predicate = RevsetFilterPredicate::Signed;
        Ok(RevsetExpression::filter(predicate))
    });
    map.insert("mine", |_diagnostics, function, context| {
        function.expect_no_arguments()?;
        // Email address domains are inherently case‐insensitive, and the local‐parts
        // are generally (although not universally) treated as case‐insensitive too, so
        // we use a case‐insensitive match here.
        let pattern = StringPattern::exact_i(context.user_email);
        let predicate = RevsetFilterPredicate::AuthorEmail(StringExpression::pattern(pattern));
        Ok(RevsetExpression::filter(predicate))
    });
    map.insert("committer", |diagnostics, function, _context| {
        let [arg] = function.expect_exact_arguments()?;
        let expr = expect_string_expression(diagnostics, arg)?;
        let name_predicate = RevsetFilterPredicate::CommitterName(expr.clone());
        let email_predicate = RevsetFilterPredicate::CommitterEmail(expr);
        Ok(RevsetExpression::filter(name_predicate)
            .union(&RevsetExpression::filter(email_predicate)))
    });
    map.insert("committer_name", |diagnostics, function, _context| {
        let [arg] = function.expect_exact_arguments()?;
        let expr = expect_string_expression(diagnostics, arg)?;
        let predicate = RevsetFilterPredicate::CommitterName(expr);
        Ok(RevsetExpression::filter(predicate))
    });
    map.insert("committer_email", |diagnostics, function, _context| {
        let [arg] = function.expect_exact_arguments()?;
        let expr = expect_string_expression(diagnostics, arg)?;
        let predicate = RevsetFilterPredicate::CommitterEmail(expr);
        Ok(RevsetExpression::filter(predicate))
    });
    map.insert("committer_date", |diagnostics, function, context| {
        let [arg] = function.expect_exact_arguments()?;
        let pattern = expect_date_pattern(diagnostics, arg, context.date_pattern_context())?;
        Ok(RevsetExpression::filter(
            RevsetFilterPredicate::CommitterDate(pattern),
        ))
    });
    map.insert("empty", |_diagnostics, function, _context| {
        function.expect_no_arguments()?;
        Ok(RevsetExpression::is_empty())
    });
    map.insert("files", |diagnostics, function, context| {
        let ctx = context.workspace.as_ref().ok_or_else(|| {
            RevsetParseError::with_span(
                RevsetParseErrorKind::FsPathWithoutWorkspace,
                function.args_span, // TODO: better to use name_span?
            )
        })?;
        let [arg] = function.expect_exact_arguments()?;
        let expr = expect_fileset_expression(diagnostics, arg, ctx.path_converter)?;
        Ok(RevsetExpression::filter(RevsetFilterPredicate::File(expr)))
    });
    map.insert("diff_contains", |diagnostics, function, context| {
        let ([text_arg], [files_opt_arg]) = function.expect_arguments()?;
        let text = expect_string_expression(diagnostics, text_arg)?;
        let files = if let Some(files_arg) = files_opt_arg {
            let ctx = context.workspace.as_ref().ok_or_else(|| {
                RevsetParseError::with_span(
                    RevsetParseErrorKind::FsPathWithoutWorkspace,
                    files_arg.span,
                )
            })?;
            expect_fileset_expression(diagnostics, files_arg, ctx.path_converter)?
        } else {
            // TODO: defaults to CLI path arguments?
            // https://github.com/jj-vcs/jj/issues/2933#issuecomment-1925870731
            FilesetExpression::all()
        };
        Ok(RevsetExpression::filter(
            RevsetFilterPredicate::DiffContains { text, files },
        ))
    });
    map.insert("conflicts", |_diagnostics, function, _context| {
        function.expect_no_arguments()?;
        Ok(RevsetExpression::filter(RevsetFilterPredicate::HasConflict))
    });
    map.insert("present", |diagnostics, function, context| {
        let [arg] = function.expect_exact_arguments()?;
        let expression = lower_expression(diagnostics, arg, context)?;
        Ok(expression.present())
    });
    map.insert("at_operation", |diagnostics, function, context| {
        let [op_arg, cand_arg] = function.expect_exact_arguments()?;
        // TODO: Parse "opset" here if we add proper language support.
        let operation = revset_parser::catch_aliases(diagnostics, op_arg, |_diagnostics, node| {
            Ok(node.span.as_str().to_owned())
        })?;
        let candidates = lower_expression(diagnostics, cand_arg, context)?;
        Ok(Arc::new(RevsetExpression::AtOperation {
            operation,
            candidates,
        }))
    });
    map.insert("coalesce", |diagnostics, function, context| {
        let ([], args) = function.expect_some_arguments()?;
        let expressions: Vec<_> = args
            .iter()
            .map(|arg| lower_expression(diagnostics, arg, context))
            .try_collect()?;
        Ok(RevsetExpression::coalesce(&expressions))
    });
    map
});

/// Parses the given `node` as a fileset expression.
pub fn expect_fileset_expression(
    diagnostics: &mut RevsetDiagnostics,
    node: &ExpressionNode,
    path_converter: &RepoPathUiConverter,
) -> Result<FilesetExpression, RevsetParseError> {
    // Alias handling is a bit tricky. The outermost expression `alias` is
    // substituted, but inner expressions `x & alias` aren't. If this seemed
    // weird, we can either transform AST or turn off revset aliases completely.
    revset_parser::catch_aliases(diagnostics, node, |diagnostics, node| {
        let mut inner_diagnostics = FilesetDiagnostics::new();
        let expression = fileset::parse(&mut inner_diagnostics, node.span.as_str(), path_converter)
            .map_err(|err| {
                RevsetParseError::expression("In fileset expression", node.span).with_source(err)
            })?;
        diagnostics.extend_with(inner_diagnostics, |diag| {
            RevsetParseError::expression("In fileset expression", node.span).with_source(diag)
        });
        Ok(expression)
    })
}

/// Transforms the given `node` into a string expression.
pub fn expect_string_expression(
    diagnostics: &mut RevsetDiagnostics,
    node: &ExpressionNode,
) -> Result<StringExpression, RevsetParseError> {
    revset_parser::catch_aliases(diagnostics, node, |diagnostics, node| {
        let expr_error = || RevsetParseError::expression("Invalid string expression", node.span);
        let pattern_error = || RevsetParseError::expression("Invalid string pattern", node.span);
        match &node.kind {
            ExpressionKind::Identifier(value) => Ok(StringExpression::substring(*value)),
            ExpressionKind::String(value) => Ok(StringExpression::substring(value)),
            ExpressionKind::StringPattern { kind, value } => {
                let pattern = StringPattern::from_str_kind(value, kind)
                    .map_err(|err| pattern_error().with_source(err))?;
                Ok(StringExpression::pattern(pattern))
            }
            ExpressionKind::RemoteSymbol(_)
            | ExpressionKind::AtWorkspace(_)
            | ExpressionKind::AtCurrentWorkspace
            | ExpressionKind::DagRangeAll
            | ExpressionKind::RangeAll => Err(expr_error()),
            ExpressionKind::Unary(op, arg_node) => {
                let arg = expect_string_expression(diagnostics, arg_node)?;
                match op {
                    UnaryOp::Negate => Ok(arg.negated()),
                    UnaryOp::DagRangePre
                    | UnaryOp::DagRangePost
                    | UnaryOp::RangePre
                    | UnaryOp::RangePost
                    | UnaryOp::Parents
                    | UnaryOp::Children => Err(expr_error()),
                }
            }
            ExpressionKind::Binary(op, lhs_node, rhs_node) => {
                let lhs = expect_string_expression(diagnostics, lhs_node)?;
                let rhs = expect_string_expression(diagnostics, rhs_node)?;
                match op {
                    BinaryOp::Intersection => Ok(lhs.intersection(rhs)),
                    BinaryOp::Difference => Ok(lhs.intersection(rhs.negated())),
                    BinaryOp::DagRange | BinaryOp::Range => Err(expr_error()),
                }
            }
            ExpressionKind::UnionAll(nodes) => {
                let expressions = nodes
                    .iter()
                    .map(|node| expect_string_expression(diagnostics, node))
                    .try_collect()?;
                Ok(StringExpression::union_all(expressions))
            }
            ExpressionKind::FunctionCall(_) | ExpressionKind::Modifier(_) => Err(expr_error()),
            ExpressionKind::AliasExpanded(..) => unreachable!(),
        }
    })
}

pub fn expect_date_pattern(
    diagnostics: &mut RevsetDiagnostics,
    node: &ExpressionNode,
    context: &DatePatternContext,
) -> Result<DatePattern, RevsetParseError> {
    revset_parser::catch_aliases(diagnostics, node, |_diagnostics, node| {
        let (value, kind) = revset_parser::expect_string_pattern("date pattern", node)?;
        let kind = kind.ok_or_else(|| {
            RevsetParseError::expression("Date pattern must specify 'after' or 'before'", node.span)
        })?;
        context.parse_relative(value, kind).map_err(|err| {
            RevsetParseError::expression("Invalid date pattern", node.span).with_source(err)
        })
    })
}

fn parse_remote_bookmarks_arguments(
    diagnostics: &mut RevsetDiagnostics,
    function: &FunctionCallNode,
    remote_ref_state: Option<RemoteRefState>,
    default_ignored_remote: Option<&RemoteName>,
) -> Result<Arc<UserRevsetExpression>, RevsetParseError> {
    let ([], [bookmark_opt_arg, remote_opt_arg]) =
        function.expect_named_arguments(&["", "remote"])?;
    let bookmark_expr = if let Some(bookmark_arg) = bookmark_opt_arg {
        expect_string_expression(diagnostics, bookmark_arg)?
    } else {
        StringExpression::all()
    };
    let remote_expr = if let Some(remote_arg) = remote_opt_arg {
        expect_string_expression(diagnostics, remote_arg)?
    } else if let Some(remote) = default_ignored_remote {
        StringExpression::exact(remote).negated()
    } else {
        StringExpression::all()
    };
    Ok(RevsetExpression::remote_bookmarks(
        bookmark_expr,
        remote_expr,
        remote_ref_state,
    ))
}

/// Resolves function call by using the given function map.
fn lower_function_call(
    diagnostics: &mut RevsetDiagnostics,
    function: &FunctionCallNode,
    context: &LoweringContext,
) -> Result<Arc<UserRevsetExpression>, RevsetParseError> {
    let function_map = &context.extensions.function_map;
    if let Some(func) = function_map.get(function.name) {
        func(diagnostics, function, context)
    } else {
        Err(RevsetParseError::with_span(
            RevsetParseErrorKind::NoSuchFunction {
                name: function.name.to_owned(),
                candidates: collect_similar(function.name, function_map.keys()),
            },
            function.name_span,
        ))
    }
}

/// Transforms the given AST `node` into expression that describes DAG
/// operation. Function calls will be resolved at this stage.
pub fn lower_expression(
    diagnostics: &mut RevsetDiagnostics,
    node: &ExpressionNode,
    context: &LoweringContext,
) -> Result<Arc<UserRevsetExpression>, RevsetParseError> {
    revset_parser::catch_aliases(diagnostics, node, |diagnostics, node| match &node.kind {
        ExpressionKind::Identifier(name) => Ok(RevsetExpression::symbol((*name).to_owned())),
        ExpressionKind::String(name) => Ok(RevsetExpression::symbol(name.to_owned())),
        ExpressionKind::StringPattern { .. } => Err(RevsetParseError::with_span(
            RevsetParseErrorKind::NotInfixOperator {
                op: ":".to_owned(),
                similar_op: "::".to_owned(),
                description: "DAG range".to_owned(),
            },
            node.span,
        )),
        ExpressionKind::RemoteSymbol(symbol) => Ok(RevsetExpression::remote_symbol(symbol.clone())),
        ExpressionKind::AtWorkspace(name) => Ok(RevsetExpression::working_copy(name.into())),
        ExpressionKind::AtCurrentWorkspace => {
            let ctx = context.workspace.as_ref().ok_or_else(|| {
                RevsetParseError::with_span(
                    RevsetParseErrorKind::WorkingCopyWithoutWorkspace,
                    node.span,
                )
            })?;
            Ok(RevsetExpression::working_copy(
                ctx.workspace_name.to_owned(),
            ))
        }
        ExpressionKind::DagRangeAll => Ok(RevsetExpression::all()),
        ExpressionKind::RangeAll => Ok(RevsetExpression::root().negated()),
        ExpressionKind::Unary(op, arg_node) => {
            let arg = lower_expression(diagnostics, arg_node, context)?;
            match op {
                UnaryOp::Negate => Ok(arg.negated()),
                UnaryOp::DagRangePre => Ok(arg.ancestors()),
                UnaryOp::DagRangePost => Ok(arg.descendants()),
                UnaryOp::RangePre => Ok(RevsetExpression::root().range(&arg)),
                UnaryOp::RangePost => Ok(arg.ancestors().negated()),
                UnaryOp::Parents => Ok(arg.parents()),
                UnaryOp::Children => Ok(arg.children()),
            }
        }
        ExpressionKind::Binary(op, lhs_node, rhs_node) => {
            let lhs = lower_expression(diagnostics, lhs_node, context)?;
            let rhs = lower_expression(diagnostics, rhs_node, context)?;
            match op {
                BinaryOp::Intersection => Ok(lhs.intersection(&rhs)),
                BinaryOp::Difference => Ok(lhs.minus(&rhs)),
                BinaryOp::DagRange => Ok(lhs.dag_range_to(&rhs)),
                BinaryOp::Range => Ok(lhs.range(&rhs)),
            }
        }
        ExpressionKind::UnionAll(nodes) => {
            let expressions: Vec<_> = nodes
                .iter()
                .map(|node| lower_expression(diagnostics, node, context))
                .try_collect()?;
            Ok(RevsetExpression::union_all(&expressions))
        }
        ExpressionKind::FunctionCall(function) => {
            lower_function_call(diagnostics, function, context)
        }
        ExpressionKind::Modifier(modifier) => {
            let name = modifier.name;
            Err(RevsetParseError::expression(
                format!("Modifier `{name}:` is not allowed in sub expression"),
                modifier.name_span,
            ))
        }
        ExpressionKind::AliasExpanded(..) => unreachable!(),
    })
}

pub fn parse(
    diagnostics: &mut RevsetDiagnostics,
    revset_str: &str,
    context: &RevsetParseContext,
) -> Result<Arc<UserRevsetExpression>, RevsetParseError> {
    let node = parse_program(revset_str)?;
    let node =
        dsl_util::expand_aliases_with_locals(node, context.aliases_map, &context.local_variables)?;
    lower_expression(diagnostics, &node, &context.to_lowering_context())
        .map_err(|err| err.extend_function_candidates(context.aliases_map.function_names()))
}

pub fn parse_with_modifier(
    diagnostics: &mut RevsetDiagnostics,
    revset_str: &str,
    context: &RevsetParseContext,
) -> Result<(Arc<UserRevsetExpression>, Option<RevsetModifier>), RevsetParseError> {
    let node = parse_program_with_modifier(revset_str)?;
    let node =
        dsl_util::expand_aliases_with_locals(node, context.aliases_map, &context.local_variables)?;
    revset_parser::catch_aliases(diagnostics, &node, |diagnostics, node| match &node.kind {
        ExpressionKind::Modifier(modifier) => {
            let parsed_modifier = match modifier.name {
                "all" => {
                    diagnostics.add_warning(RevsetParseError::expression(
                        "Multiple revisions are allowed by default; `all:` is planned for removal",
                        modifier.name_span,
                    ));
                    RevsetModifier::All
                }
                _ => {
                    return Err(RevsetParseError::with_span(
                        RevsetParseErrorKind::NoSuchModifier(modifier.name.to_owned()),
                        modifier.name_span,
                    ));
                }
            };
            let parsed_body =
                lower_expression(diagnostics, &modifier.body, &context.to_lowering_context())?;
            Ok((parsed_body, Some(parsed_modifier)))
        }
        _ => {
            let parsed_body = lower_expression(diagnostics, node, &context.to_lowering_context())?;
            Ok((parsed_body, None))
        }
    })
    .map_err(|err| err.extend_function_candidates(context.aliases_map.function_names()))
}

/// Constructs binary tree from `expressions` list, `unit` node, and associative
/// `binary` operation.
fn to_binary_expression<T: Clone>(
    expressions: &[T],
    unit: &impl Fn() -> T,
    binary: &impl Fn(&T, &T) -> T,
) -> T {
    match expressions {
        [] => unit(),
        [expression] => expression.clone(),
        _ => {
            // Build balanced tree to minimize the recursion depth.
            let (left, right) = expressions.split_at(expressions.len() / 2);
            binary(
                &to_binary_expression(left, unit, binary),
                &to_binary_expression(right, unit, binary),
            )
        }
    }
}

/// `Some` for rewritten expression, or `None` to reuse the original expression.
type TransformedExpression<St> = Option<Arc<RevsetExpression<St>>>;
/// `Break` to not transform subtree recursively. `Continue(Some(rewritten))`
/// isn't allowed because it could be a source of infinite substitution bugs.
type PreTransformedExpression<St> = ControlFlow<TransformedExpression<St>, ()>;

/// Walks `expression` tree and applies `pre`/`post` transformation recursively.
fn transform_expression<St: ExpressionState>(
    expression: &Arc<RevsetExpression<St>>,
    mut pre: impl FnMut(&Arc<RevsetExpression<St>>) -> PreTransformedExpression<St>,
    mut post: impl FnMut(&Arc<RevsetExpression<St>>) -> TransformedExpression<St>,
) -> TransformedExpression<St> {
    let Ok(transformed) =
        try_transform_expression::<St, Infallible>(expression, |x| Ok(pre(x)), |x| Ok(post(x)));
    transformed
}

/// Walks `expression` tree and applies `post` recursively from leaf nodes.
fn transform_expression_bottom_up<St: ExpressionState>(
    expression: &Arc<RevsetExpression<St>>,
    post: impl FnMut(&Arc<RevsetExpression<St>>) -> TransformedExpression<St>,
) -> TransformedExpression<St> {
    transform_expression(expression, |_| ControlFlow::Continue(()), post)
}

/// Walks `expression` tree and applies transformation recursively.
///
/// `pre` is the callback to rewrite subtree including children. It is invoked
/// before visiting the child nodes. If returned `Break`, children won't be
/// visited.
///
/// `post` is the callback to rewrite from leaf nodes. If returned `None`,
/// the original expression node will be reused.
///
/// If no nodes rewritten, this function returns `None`.
/// `std::iter::successors()` could be used if the transformation needs to be
/// applied repeatedly until converged.
fn try_transform_expression<St: ExpressionState, E>(
    expression: &Arc<RevsetExpression<St>>,
    mut pre: impl FnMut(&Arc<RevsetExpression<St>>) -> Result<PreTransformedExpression<St>, E>,
    mut post: impl FnMut(&Arc<RevsetExpression<St>>) -> Result<TransformedExpression<St>, E>,
) -> Result<TransformedExpression<St>, E> {
    fn transform_child_rec<St: ExpressionState, E>(
        expression: &Arc<RevsetExpression<St>>,
        pre: &mut impl FnMut(&Arc<RevsetExpression<St>>) -> Result<PreTransformedExpression<St>, E>,
        post: &mut impl FnMut(&Arc<RevsetExpression<St>>) -> Result<TransformedExpression<St>, E>,
    ) -> Result<TransformedExpression<St>, E> {
        Ok(match expression.as_ref() {
            RevsetExpression::None => None,
            RevsetExpression::All => None,
            RevsetExpression::VisibleHeads => None,
            RevsetExpression::VisibleHeadsOrReferenced => None,
            RevsetExpression::Root => None,
            RevsetExpression::Commits(_) => None,
            RevsetExpression::CommitRef(_) => None,
            RevsetExpression::Ancestors {
                heads,
                generation,
                parents_range,
            } => transform_rec(heads, pre, post)?.map(|heads| RevsetExpression::Ancestors {
                heads,
                generation: generation.clone(),
                parents_range: parents_range.clone(),
            }),
            RevsetExpression::Descendants { roots, generation } => transform_rec(roots, pre, post)?
                .map(|roots| RevsetExpression::Descendants {
                    roots,
                    generation: generation.clone(),
                }),
            RevsetExpression::Range {
                roots,
                heads,
                generation,
                parents_range,
            } => transform_rec_pair((roots, heads), pre, post)?.map(|(roots, heads)| {
                RevsetExpression::Range {
                    roots,
                    heads,
                    generation: generation.clone(),
                    parents_range: parents_range.clone(),
                }
            }),
            RevsetExpression::DagRange { roots, heads } => {
                transform_rec_pair((roots, heads), pre, post)?
                    .map(|(roots, heads)| RevsetExpression::DagRange { roots, heads })
            }
            RevsetExpression::Reachable { sources, domain } => {
                transform_rec_pair((sources, domain), pre, post)?
                    .map(|(sources, domain)| RevsetExpression::Reachable { sources, domain })
            }
            RevsetExpression::Heads(candidates) => {
                transform_rec(candidates, pre, post)?.map(RevsetExpression::Heads)
            }
            RevsetExpression::HeadsRange {
                roots,
                heads,
                parents_range,
                filter,
            } => {
                let transformed_roots = transform_rec(roots, pre, post)?;
                let transformed_heads = transform_rec(heads, pre, post)?;
                let transformed_filter = transform_rec(filter, pre, post)?;
                (transformed_roots.is_some()
                    || transformed_heads.is_some()
                    || transformed_filter.is_some())
                .then(|| RevsetExpression::HeadsRange {
                    roots: transformed_roots.unwrap_or_else(|| roots.clone()),
                    heads: transformed_heads.unwrap_or_else(|| heads.clone()),
                    parents_range: parents_range.clone(),
                    filter: transformed_filter.unwrap_or_else(|| filter.clone()),
                })
            }
            RevsetExpression::Roots(candidates) => {
                transform_rec(candidates, pre, post)?.map(RevsetExpression::Roots)
            }
            RevsetExpression::ForkPoint(expression) => {
                transform_rec(expression, pre, post)?.map(RevsetExpression::ForkPoint)
            }
            RevsetExpression::Bisect(expression) => {
                transform_rec(expression, pre, post)?.map(RevsetExpression::Bisect)
            }
            RevsetExpression::HasSize { candidates, count } => {
                transform_rec(candidates, pre, post)?.map(|candidates| RevsetExpression::HasSize {
                    candidates,
                    count: *count,
                })
            }
            RevsetExpression::Latest { candidates, count } => transform_rec(candidates, pre, post)?
                .map(|candidates| RevsetExpression::Latest {
                    candidates,
                    count: *count,
                }),
            RevsetExpression::Filter(_) => None,
            RevsetExpression::AsFilter(candidates) => {
                transform_rec(candidates, pre, post)?.map(RevsetExpression::AsFilter)
            }
            RevsetExpression::AtOperation {
                operation,
                candidates,
            } => transform_rec(candidates, pre, post)?.map(|candidates| {
                RevsetExpression::AtOperation {
                    operation: operation.clone(),
                    candidates,
                }
            }),
            RevsetExpression::WithinReference {
                candidates,
                commits,
            } => transform_rec(candidates, pre, post)?.map(|candidates| {
                RevsetExpression::WithinReference {
                    candidates,
                    commits: commits.clone(),
                }
            }),
            RevsetExpression::WithinVisibility {
                candidates,
                visible_heads,
            } => transform_rec(candidates, pre, post)?.map(|candidates| {
                RevsetExpression::WithinVisibility {
                    candidates,
                    visible_heads: visible_heads.clone(),
                }
            }),
            RevsetExpression::Coalesce(expression1, expression2) => transform_rec_pair(
                (expression1, expression2),
                pre,
                post,
            )?
            .map(|(expression1, expression2)| RevsetExpression::Coalesce(expression1, expression2)),
            RevsetExpression::Present(candidates) => {
                transform_rec(candidates, pre, post)?.map(RevsetExpression::Present)
            }
            RevsetExpression::NotIn(complement) => {
                transform_rec(complement, pre, post)?.map(RevsetExpression::NotIn)
            }
            RevsetExpression::Union(expression1, expression2) => {
                transform_rec_pair((expression1, expression2), pre, post)?.map(
                    |(expression1, expression2)| RevsetExpression::Union(expression1, expression2),
                )
            }
            RevsetExpression::Intersection(expression1, expression2) => {
                transform_rec_pair((expression1, expression2), pre, post)?.map(
                    |(expression1, expression2)| {
                        RevsetExpression::Intersection(expression1, expression2)
                    },
                )
            }
            RevsetExpression::Difference(expression1, expression2) => {
                transform_rec_pair((expression1, expression2), pre, post)?.map(
                    |(expression1, expression2)| {
                        RevsetExpression::Difference(expression1, expression2)
                    },
                )
            }
        }
        .map(Arc::new))
    }

    #[expect(clippy::type_complexity)]
    fn transform_rec_pair<St: ExpressionState, E>(
        (expression1, expression2): (&Arc<RevsetExpression<St>>, &Arc<RevsetExpression<St>>),
        pre: &mut impl FnMut(&Arc<RevsetExpression<St>>) -> Result<PreTransformedExpression<St>, E>,
        post: &mut impl FnMut(&Arc<RevsetExpression<St>>) -> Result<TransformedExpression<St>, E>,
    ) -> Result<Option<(Arc<RevsetExpression<St>>, Arc<RevsetExpression<St>>)>, E> {
        match (
            transform_rec(expression1, pre, post)?,
            transform_rec(expression2, pre, post)?,
        ) {
            (Some(new_expression1), Some(new_expression2)) => {
                Ok(Some((new_expression1, new_expression2)))
            }
            (Some(new_expression1), None) => Ok(Some((new_expression1, expression2.clone()))),
            (None, Some(new_expression2)) => Ok(Some((expression1.clone(), new_expression2))),
            (None, None) => Ok(None),
        }
    }

    fn transform_rec<St: ExpressionState, E>(
        expression: &Arc<RevsetExpression<St>>,
        pre: &mut impl FnMut(&Arc<RevsetExpression<St>>) -> Result<PreTransformedExpression<St>, E>,
        post: &mut impl FnMut(&Arc<RevsetExpression<St>>) -> Result<TransformedExpression<St>, E>,
    ) -> Result<TransformedExpression<St>, E> {
        if let ControlFlow::Break(transformed) = pre(expression)? {
            return Ok(transformed);
        }
        if let Some(new_expression) = transform_child_rec(expression, pre, post)? {
            // must propagate new expression tree
            Ok(Some(post(&new_expression)?.unwrap_or(new_expression)))
        } else {
            post(expression)
        }
    }

    transform_rec(expression, &mut pre, &mut post)
}

/// Visitor-like interface to transform [`RevsetExpression`] state recursively.
///
/// This is similar to [`try_transform_expression()`], but is supposed to
/// transform the resolution state from `InSt` to `OutSt`.
trait ExpressionStateFolder<InSt: ExpressionState, OutSt: ExpressionState> {
    type Error;

    /// Transforms the `expression`. By default, inner items are transformed
    /// recursively.
    fn fold_expression(
        &mut self,
        expression: &RevsetExpression<InSt>,
    ) -> Result<Arc<RevsetExpression<OutSt>>, Self::Error> {
        fold_child_expression_state(self, expression)
    }

    /// Transforms commit ref such as symbol.
    fn fold_commit_ref(
        &mut self,
        commit_ref: &InSt::CommitRef,
    ) -> Result<Arc<RevsetExpression<OutSt>>, Self::Error>;

    /// Transforms `at_operation(operation, candidates)` expression.
    fn fold_at_operation(
        &mut self,
        operation: &InSt::Operation,
        candidates: &RevsetExpression<InSt>,
    ) -> Result<Arc<RevsetExpression<OutSt>>, Self::Error>;
}

/// Transforms inner items of the `expression` by using the `folder`.
fn fold_child_expression_state<InSt, OutSt, F>(
    folder: &mut F,
    expression: &RevsetExpression<InSt>,
) -> Result<Arc<RevsetExpression<OutSt>>, F::Error>
where
    InSt: ExpressionState,
    OutSt: ExpressionState,
    F: ExpressionStateFolder<InSt, OutSt> + ?Sized,
{
    let expression: Arc<_> = match expression {
        RevsetExpression::None => RevsetExpression::None.into(),
        RevsetExpression::All => RevsetExpression::All.into(),
        RevsetExpression::VisibleHeads => RevsetExpression::VisibleHeads.into(),
        RevsetExpression::VisibleHeadsOrReferenced => {
            RevsetExpression::VisibleHeadsOrReferenced.into()
        }
        RevsetExpression::Root => RevsetExpression::Root.into(),
        RevsetExpression::Commits(ids) => RevsetExpression::Commits(ids.clone()).into(),
        RevsetExpression::CommitRef(commit_ref) => folder.fold_commit_ref(commit_ref)?,
        RevsetExpression::Ancestors {
            heads,
            generation,
            parents_range,
        } => {
            let heads = folder.fold_expression(heads)?;
            let generation = generation.clone();
            let parents_range = parents_range.clone();
            RevsetExpression::Ancestors {
                heads,
                generation,
                parents_range,
            }
            .into()
        }
        RevsetExpression::Descendants { roots, generation } => {
            let roots = folder.fold_expression(roots)?;
            let generation = generation.clone();
            RevsetExpression::Descendants { roots, generation }.into()
        }
        RevsetExpression::Range {
            roots,
            heads,
            generation,
            parents_range,
        } => {
            let roots = folder.fold_expression(roots)?;
            let heads = folder.fold_expression(heads)?;
            let generation = generation.clone();
            let parents_range = parents_range.clone();
            RevsetExpression::Range {
                roots,
                heads,
                generation,
                parents_range,
            }
            .into()
        }
        RevsetExpression::DagRange { roots, heads } => {
            let roots = folder.fold_expression(roots)?;
            let heads = folder.fold_expression(heads)?;
            RevsetExpression::DagRange { roots, heads }.into()
        }
        RevsetExpression::Reachable { sources, domain } => {
            let sources = folder.fold_expression(sources)?;
            let domain = folder.fold_expression(domain)?;
            RevsetExpression::Reachable { sources, domain }.into()
        }
        RevsetExpression::Heads(heads) => {
            let heads = folder.fold_expression(heads)?;
            RevsetExpression::Heads(heads).into()
        }
        RevsetExpression::HeadsRange {
            roots,
            heads,
            parents_range,
            filter,
        } => {
            let roots = folder.fold_expression(roots)?;
            let heads = folder.fold_expression(heads)?;
            let parents_range = parents_range.clone();
            let filter = folder.fold_expression(filter)?;
            RevsetExpression::HeadsRange {
                roots,
                heads,
                parents_range,
                filter,
            }
            .into()
        }
        RevsetExpression::Roots(roots) => {
            let roots = folder.fold_expression(roots)?;
            RevsetExpression::Roots(roots).into()
        }
        RevsetExpression::ForkPoint(expression) => {
            let expression = folder.fold_expression(expression)?;
            RevsetExpression::ForkPoint(expression).into()
        }
        RevsetExpression::Bisect(expression) => {
            let expression = folder.fold_expression(expression)?;
            RevsetExpression::Bisect(expression).into()
        }
        RevsetExpression::HasSize { candidates, count } => {
            let candidates = folder.fold_expression(candidates)?;
            RevsetExpression::HasSize {
                candidates,
                count: *count,
            }
            .into()
        }
        RevsetExpression::Latest { candidates, count } => {
            let candidates = folder.fold_expression(candidates)?;
            let count = *count;
            RevsetExpression::Latest { candidates, count }.into()
        }
        RevsetExpression::Filter(predicate) => RevsetExpression::Filter(predicate.clone()).into(),
        RevsetExpression::AsFilter(candidates) => {
            let candidates = folder.fold_expression(candidates)?;
            RevsetExpression::AsFilter(candidates).into()
        }
        RevsetExpression::AtOperation {
            operation,
            candidates,
        } => folder.fold_at_operation(operation, candidates)?,
        RevsetExpression::WithinReference {
            candidates,
            commits,
        } => {
            let candidates = folder.fold_expression(candidates)?;
            let commits = commits.clone();
            RevsetExpression::WithinReference {
                candidates,
                commits,
            }
            .into()
        }
        RevsetExpression::WithinVisibility {
            candidates,
            visible_heads,
        } => {
            let candidates = folder.fold_expression(candidates)?;
            let visible_heads = visible_heads.clone();
            RevsetExpression::WithinVisibility {
                candidates,
                visible_heads,
            }
            .into()
        }
        RevsetExpression::Coalesce(expression1, expression2) => {
            let expression1 = folder.fold_expression(expression1)?;
            let expression2 = folder.fold_expression(expression2)?;
            RevsetExpression::Coalesce(expression1, expression2).into()
        }
        RevsetExpression::Present(candidates) => {
            let candidates = folder.fold_expression(candidates)?;
            RevsetExpression::Present(candidates).into()
        }
        RevsetExpression::NotIn(complement) => {
            let complement = folder.fold_expression(complement)?;
            RevsetExpression::NotIn(complement).into()
        }
        RevsetExpression::Union(expression1, expression2) => {
            let expression1 = folder.fold_expression(expression1)?;
            let expression2 = folder.fold_expression(expression2)?;
            RevsetExpression::Union(expression1, expression2).into()
        }
        RevsetExpression::Intersection(expression1, expression2) => {
            let expression1 = folder.fold_expression(expression1)?;
            let expression2 = folder.fold_expression(expression2)?;
            RevsetExpression::Intersection(expression1, expression2).into()
        }
        RevsetExpression::Difference(expression1, expression2) => {
            let expression1 = folder.fold_expression(expression1)?;
            let expression2 = folder.fold_expression(expression2)?;
            RevsetExpression::Difference(expression1, expression2).into()
        }
    };
    Ok(expression)
}

/// Collects explicitly-referenced commits, inserts marker nodes.
///
/// User symbols and `at_operation()` scopes should have been resolved.
fn resolve_referenced_commits<St: ExpressionState>(
    expression: &Arc<RevsetExpression<St>>,
) -> TransformedExpression<St> {
    // Trust precomputed value if any
    if matches!(
        expression.as_ref(),
        RevsetExpression::WithinReference { .. }
    ) {
        return None;
    }

    // Use separate Vec to get around borrowing issue
    let mut inner_commits = Vec::new();
    let mut outer_commits = Vec::new();
    let transformed = transform_expression(
        expression,
        |expression| match expression.as_ref() {
            // Trust precomputed value
            RevsetExpression::WithinReference { commits, .. } => {
                inner_commits.extend_from_slice(commits);
                ControlFlow::Break(None)
            }
            // at_operation() scope shouldn't be affected by outer
            RevsetExpression::WithinVisibility {
                candidates,
                visible_heads,
            } => {
                // ::visible_heads shouldn't be filtered out by outer
                inner_commits.extend_from_slice(visible_heads);
                let transformed = resolve_referenced_commits(candidates);
                // Referenced commits shouldn't be filtered out by outer
                if let RevsetExpression::WithinReference { commits, .. } =
                    transformed.as_deref().unwrap_or(candidates)
                {
                    inner_commits.extend_from_slice(commits);
                }
                ControlFlow::Break(transformed.map(|candidates| {
                    Arc::new(RevsetExpression::WithinVisibility {
                        candidates,
                        visible_heads: visible_heads.clone(),
                    })
                }))
            }
            _ => ControlFlow::Continue(()),
        },
        |expression| {
            if let RevsetExpression::Commits(commits) = expression.as_ref() {
                outer_commits.extend_from_slice(commits);
            }
            None
        },
    );

    // Commits could be deduplicated here, but they'll be concatenated with
    // the visible heads later, which may have duplicates.
    outer_commits.extend(inner_commits);
    if outer_commits.is_empty() {
        // Omit empty node to keep test/debug output concise
        return transformed;
    }
    Some(Arc::new(RevsetExpression::WithinReference {
        candidates: transformed.unwrap_or_else(|| expression.clone()),
        commits: outer_commits,
    }))
}

/// Flatten all intersections to be left-recursive. For instance, transforms
/// `(a & b) & (c & d)` into `((a & b) & c) & d`.
fn flatten_intersections<St: ExpressionState>(
    expression: &Arc<RevsetExpression<St>>,
) -> TransformedExpression<St> {
    fn flatten<St: ExpressionState>(
        expression1: &Arc<RevsetExpression<St>>,
        expression2: &Arc<RevsetExpression<St>>,
    ) -> TransformedExpression<St> {
        let recurse = |a, b| flatten(a, b).unwrap_or_else(|| a.intersection(b));

        match expression2.as_ref() {
            // flatten(a & (b & c)) -> flatten(a & b) & c
            RevsetExpression::Intersection(inner1, inner2) => {
                Some(recurse(expression1, inner1).intersection(inner2))
            }
            _ => None,
        }
    }

    transform_expression_bottom_up(expression, |expression| match expression.as_ref() {
        RevsetExpression::Intersection(expression1, expression2) => {
            flatten(expression1, expression2)
        }
        _ => None,
    })
}

/// Intersects `expression` with `base`, maintaining sorted order using the
/// provided key. If `base` is an intersection, it must be left-recursive, and
/// it must already be in sorted order.
fn sort_intersection_by_key<St: ExpressionState, T: Ord>(
    base: &Arc<RevsetExpression<St>>,
    expression: &Arc<RevsetExpression<St>>,
    mut get_key: impl FnMut(&RevsetExpression<St>) -> T,
) -> TransformedExpression<St> {
    // We only want to compute the key for `expression` once instead of computing it
    // on every iteration.
    fn sort_intersection_helper<St: ExpressionState, T: Ord>(
        base: &Arc<RevsetExpression<St>>,
        expression: &Arc<RevsetExpression<St>>,
        expression_key: T,
        mut get_key: impl FnMut(&RevsetExpression<St>) -> T,
    ) -> TransformedExpression<St> {
        if let RevsetExpression::Intersection(inner1, inner2) = base.as_ref() {
            // sort_intersection(a & b, c) -> sort_intersection(a, c) & b
            (expression_key < get_key(inner2)).then(|| {
                sort_intersection_helper(inner1, expression, expression_key, get_key)
                    .unwrap_or_else(|| inner1.intersection(expression))
                    .intersection(inner2)
            })
        } else {
            // a & b -> b & a
            (expression_key < get_key(base)).then(|| expression.intersection(base))
        }
    }

    sort_intersection_helper(base, expression, get_key(expression), get_key)
}

/// Push `ancestors(x)` and `~ancestors(x)` down (to the left) in intersections.
/// All `~ancestors(x)` will be moved before `ancestors(x)`, since negated
/// ancestors can be converted to ranges. All other negations are moved to the
/// right, since these negations can usually be evaluated better as differences.
fn sort_negations_and_ancestors<St: ExpressionState>(
    expression: &Arc<RevsetExpression<St>>,
) -> TransformedExpression<St> {
    #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    enum AncestorsOrder {
        NegatedAncestors,
        Ancestors,
        Other,
        NegatedOther,
    }

    transform_expression_bottom_up(expression, |expression| match expression.as_ref() {
        RevsetExpression::Intersection(expression1, expression2) => {
            sort_intersection_by_key(expression1, expression2, |expression| match expression {
                RevsetExpression::Ancestors {
                    heads: _,
                    generation: Range { end: u64::MAX, .. },
                    parents_range: _,
                } => AncestorsOrder::Ancestors,
                RevsetExpression::NotIn(complement) => match complement.as_ref() {
                    RevsetExpression::Ancestors {
                        heads: _,
                        generation: Range { end: u64::MAX, .. },
                        // We only want to move negated ancestors with a full parents range, since
                        // these are the only negated ancestors which can be converted to a range.
                        parents_range: PARENTS_RANGE_FULL,
                    } => AncestorsOrder::NegatedAncestors,
                    _ => AncestorsOrder::NegatedOther,
                },
                _ => AncestorsOrder::Other,
            })
        }
        _ => None,
    })
}

/// Transforms filter expressions, by applying the following rules.
///
/// a. Moves as many sets to left of filter intersection as possible, to
///    minimize the filter inputs.
/// b. TODO: Rewrites set operations to and/or/not of predicates, to
///    help further optimization (e.g. combine `file(_)` matchers.)
/// c. Wraps union of filter and set (e.g. `author(_) | heads()`), to
///    ensure inner filter wouldn't need to evaluate all the input sets.
fn internalize_filter<St: ExpressionState>(
    expression: &Arc<RevsetExpression<St>>,
) -> TransformedExpression<St> {
    fn get_filter<St: ExpressionState>(
        expression: &Arc<RevsetExpression<St>>,
    ) -> Option<&Arc<RevsetExpression<St>>> {
        match expression.as_ref() {
            RevsetExpression::Filter(_) => Some(expression),
            RevsetExpression::AsFilter(candidates) => Some(candidates),
            _ => None,
        }
    }

    fn mark_filter<St: ExpressionState>(
        expression: Arc<RevsetExpression<St>>,
    ) -> Arc<RevsetExpression<St>> {
        Arc::new(RevsetExpression::AsFilter(expression))
    }

    transform_expression_bottom_up(expression, |expression| match expression.as_ref() {
        // Mark expression as filter if any of the child nodes are filter.
        RevsetExpression::Present(e) => get_filter(e).map(|f| mark_filter(f.present())),
        RevsetExpression::NotIn(e) => get_filter(e).map(|f| mark_filter(f.negated())),
        RevsetExpression::Union(e1, e2) => {
            let f1 = get_filter(e1);
            let f2 = get_filter(e2);
            (f1.is_some() || f2.is_some())
                .then(|| mark_filter(f1.unwrap_or(e1).union(f2.unwrap_or(e2))))
        }
        // Bottom-up pass pulls up-right filter node from leaf '(c & f) & e' ->
        // '(c & e) & f', so that an intersection of filter node can be found as
        // a direct child of another intersection node. Suppose intersection is
        // left-recursive, e2 shouldn't be an intersection node. e1 may be set,
        // filter, (set & filter), ((set & set) & filter), ...
        RevsetExpression::Intersection(e1, e2) => match (get_filter(e1), get_filter(e2)) {
            // f1 & f2 -> filter(f1 & f2)
            (Some(f1), Some(f2)) => Some(mark_filter(f1.intersection(f2))),
            // f1 & s2 -> s2 & filter(f1)
            (Some(_), None) => Some(e2.intersection(e1)),
            // (s1a & f1b) & f2 -> s1a & filter(f1b & f2)
            (None, Some(f2)) => match e1.as_ref() {
                RevsetExpression::Intersection(e1a, e1b) => {
                    get_filter(e1b).map(|f1b| e1a.intersection(&mark_filter(f1b.intersection(f2))))
                }
                _ => None,
            },
            // (s1a & f1b) & s2 -> (s1a & s2) & filter(f1b)
            (None, None) => match e1.as_ref() {
                RevsetExpression::Intersection(e1a, e1b) => {
                    get_filter(e1b).map(|_| e1a.intersection(e2).intersection(e1b))
                }
                _ => None,
            },
        },
        // Difference(e1, e2) should have been unfolded to Intersection(e1, NotIn(e2)).
        _ => None,
    })
}

/// Eliminates redundant nodes like `x & all()`, `~~x`.
///
/// Since this function rewrites `x & none()` to `none()`, user symbols should
/// have been resolved. Otherwise, an invalid symbol could be optimized out.
fn fold_redundant_expression<St: ExpressionState>(
    expression: &Arc<RevsetExpression<St>>,
) -> TransformedExpression<St> {
    transform_expression_bottom_up(expression, |expression| match expression.as_ref() {
        RevsetExpression::Commits(commits) if commits.is_empty() => Some(RevsetExpression::none()),
        RevsetExpression::NotIn(outer) => match outer.as_ref() {
            RevsetExpression::NotIn(inner) => Some(inner.clone()),
            RevsetExpression::None => Some(RevsetExpression::all()),
            RevsetExpression::All => Some(RevsetExpression::none()),
            _ => None,
        },
        RevsetExpression::Union(expression1, expression2) => {
            match (expression1.as_ref(), expression2.as_ref()) {
                (_, RevsetExpression::None) => Some(expression1.clone()),
                (RevsetExpression::None, _) => Some(expression2.clone()),
                (RevsetExpression::All, _) => Some(RevsetExpression::all()),
                (_, RevsetExpression::All) => Some(RevsetExpression::all()),
                _ => None,
            }
        }
        RevsetExpression::Intersection(expression1, expression2) => {
            match (expression1.as_ref(), expression2.as_ref()) {
                (RevsetExpression::None, _) => Some(RevsetExpression::none()),
                (_, RevsetExpression::None) => Some(RevsetExpression::none()),
                (_, RevsetExpression::All) => Some(expression1.clone()),
                (RevsetExpression::All, _) => Some(expression2.clone()),
                _ => None,
            }
        }
        _ => None,
    })
}

/// Extracts `heads` from a revset expression `ancestors(heads)`. Unfolds
/// generations as necessary, so `ancestors(heads, 2..)` would return
/// `ancestors(heads, 2..3)`, which is equivalent to `heads--`.
fn ancestors_to_heads<St: ExpressionState>(
    expression: &RevsetExpression<St>,
) -> Result<Arc<RevsetExpression<St>>, ()> {
    match ancestors_to_heads_and_parents_range(expression) {
        Ok((heads, PARENTS_RANGE_FULL)) => Ok(heads),
        _ => Err(()),
    }
}

fn ancestors_to_heads_and_parents_range<St: ExpressionState>(
    expression: &RevsetExpression<St>,
) -> Result<(Arc<RevsetExpression<St>>, Range<u32>), ()> {
    match expression {
        RevsetExpression::Ancestors {
            heads,
            generation: GENERATION_RANGE_FULL,
            parents_range,
        } => Ok((heads.clone(), parents_range.clone())),
        RevsetExpression::Ancestors {
            heads,
            generation: Range {
                start,
                end: u64::MAX,
            },
            parents_range,
        } => Ok((
            Arc::new(RevsetExpression::Ancestors {
                heads: heads.clone(),
                generation: (*start)..start.saturating_add(1),
                parents_range: parents_range.clone(),
            }),
            parents_range.clone(),
        )),
        _ => Err(()),
    }
}

/// Folds `::x | ::y` into `::(x | y)`, and `~::x & ~::y` into `~::(x | y)`.
/// Does not fold intersections of negations involving non-ancestors
/// expressions, since this can result in less efficient evaluation, such as for
/// `~::x & ~y`, which should be `x.. ~ y` instead of `~(::x | y)`.
fn fold_ancestors_union<St: ExpressionState>(
    expression: &Arc<RevsetExpression<St>>,
) -> TransformedExpression<St> {
    fn union_ancestors<St: ExpressionState>(
        expression1: &Arc<RevsetExpression<St>>,
        expression2: &Arc<RevsetExpression<St>>,
    ) -> TransformedExpression<St> {
        let heads1 = ancestors_to_heads(expression1).ok()?;
        let heads2 = ancestors_to_heads(expression2).ok()?;
        Some(heads1.union(&heads2).ancestors())
    }

    transform_expression_bottom_up(expression, |expression| match expression.as_ref() {
        RevsetExpression::Union(expression1, expression2) => {
            // ::x | ::y -> ::(x | y)
            union_ancestors(expression1, expression2)
        }
        RevsetExpression::Intersection(expression1, expression2) => {
            match (expression1.as_ref(), expression2.as_ref()) {
                // ~::x & ~::y -> ~(::x | ::y) -> ~::(x | y)
                (RevsetExpression::NotIn(complement1), RevsetExpression::NotIn(complement2)) => {
                    union_ancestors(complement1, complement2).map(|expression| expression.negated())
                }
                _ => None,
            }
        }
        _ => None,
    })
}

/// Transforms expressions like `heads(roots..heads & filters)` into a combined
/// operation where possible. Also optimizes the heads of ancestors expressions
/// involving ranges or filters such as `::(foo..bar)` or `::mine()`.
///
/// Ancestors and negated ancestors should have already been moved to the left
/// in intersections, and negated ancestors should have been combined already.
fn fold_heads_range<St: ExpressionState>(
    expression: &Arc<RevsetExpression<St>>,
) -> TransformedExpression<St> {
    // Represents `roots..heads & filter`
    struct FilteredRange<St: ExpressionState> {
        roots: Arc<RevsetExpression<St>>,
        heads_and_parents_range: Option<(Arc<RevsetExpression<St>>, Range<u32>)>,
        filter: Arc<RevsetExpression<St>>,
    }

    impl<St: ExpressionState> FilteredRange<St> {
        fn new(roots: Arc<RevsetExpression<St>>) -> Self {
            // roots.. & all()
            Self {
                roots,
                heads_and_parents_range: None,
                filter: RevsetExpression::all(),
            }
        }

        fn add(mut self, expression: &Arc<RevsetExpression<St>>) -> Self {
            if self.heads_and_parents_range.is_none() {
                // x.. & ::y -> x..y
                if let Ok(heads_and_parents_range) =
                    ancestors_to_heads_and_parents_range(expression)
                {
                    self.heads_and_parents_range = Some(heads_and_parents_range);
                    return self;
                }
            }
            self.add_filter(expression)
        }

        fn add_filter(mut self, expression: &Arc<RevsetExpression<St>>) -> Self {
            self.filter = if let RevsetExpression::All = self.filter.as_ref() {
                // x..y & all() & f -> x..y & f
                expression.clone()
            } else {
                self.filter.intersection(expression)
            };
            self
        }
    }

    fn to_filtered_range<St: ExpressionState>(
        expression: &Arc<RevsetExpression<St>>,
    ) -> Option<FilteredRange<St>> {
        // If the first expression is `ancestors(x)`, then we already know the range
        // must be `none()..x`, since any roots would've been moved to the left by an
        // earlier pass.
        if let Ok(heads_and_parents_range) = ancestors_to_heads_and_parents_range(expression) {
            return Some(FilteredRange {
                roots: RevsetExpression::none(),
                heads_and_parents_range: Some(heads_and_parents_range),
                filter: RevsetExpression::all(),
            });
        }
        match expression.as_ref() {
            // All roots should have been moved to the start of the intersection by an earlier pass,
            // so we can set the roots based on the first expression in the intersection.
            RevsetExpression::NotIn(complement) => {
                if let Ok(roots) = ancestors_to_heads(complement) {
                    Some(FilteredRange::new(roots))
                } else {
                    // If the first expression is a non-ancestors negation, we still want to use
                    // `HeadsRange` since `~x` is equivalent to `::visible_heads() ~ x`.
                    Some(FilteredRange::new(RevsetExpression::none()).add_filter(expression))
                }
            }
            // We also want to optimize `heads()` if the first expression is `all()` or a filter.
            RevsetExpression::All | RevsetExpression::Filter(_) | RevsetExpression::AsFilter(_) => {
                Some(FilteredRange::new(RevsetExpression::none()).add_filter(expression))
            }
            // We only need to handle intersections recursively. Differences will have been
            // unfolded already.
            RevsetExpression::Intersection(expression1, expression2) => {
                to_filtered_range(expression1).map(|filtered_range| filtered_range.add(expression2))
            }
            _ => None,
        }
    }

    fn to_heads_range<St: ExpressionState>(
        candidates: &Arc<RevsetExpression<St>>,
    ) -> Option<Arc<RevsetExpression<St>>> {
        to_filtered_range(candidates).map(|filtered_range| {
            let (heads, parents_range) =
                filtered_range.heads_and_parents_range.unwrap_or_else(|| {
                    (
                        RevsetExpression::visible_heads_or_referenced(),
                        PARENTS_RANGE_FULL,
                    )
                });
            RevsetExpression::HeadsRange {
                roots: filtered_range.roots,
                heads,
                parents_range,
                filter: filtered_range.filter,
            }
            .into()
        })
    }

    transform_expression_bottom_up(expression, |expression| match expression.as_ref() {
        // ::(x..y & filter) -> ::heads_range(x, y, filter)
        // ::filter -> ::heads_range(none(), visible_heads_or_referenced(), filter)
        RevsetExpression::Ancestors {
            heads,
            // This optimization is only valid for full generation and parents ranges, since
            // otherwise adding `heads()` would change the result.
            generation: GENERATION_RANGE_FULL,
            parents_range: PARENTS_RANGE_FULL,
        } => to_heads_range(heads).map(|heads| heads.ancestors()),
        // heads(x..y & filter) -> heads_range(x, y, filter)
        // heads(filter) -> heads_range(none(), visible_heads_or_referenced(), filter)
        RevsetExpression::Heads(candidates) => to_heads_range(candidates),
        _ => None,
    })
}

fn to_difference_range<St: ExpressionState>(
    expression: &Arc<RevsetExpression<St>>,
    complement: &Arc<RevsetExpression<St>>,
) -> TransformedExpression<St> {
    let RevsetExpression::Ancestors {
        heads,
        generation,
        parents_range,
    } = expression.as_ref()
    else {
        return None;
    };
    let roots = ancestors_to_heads(complement).ok()?;
    // ::heads & ~(::roots) -> roots..heads
    // ::heads & ~(::roots-) -> ::heads & ~ancestors(roots, 1..) -> roots-..heads
    Some(Arc::new(RevsetExpression::Range {
        roots,
        heads: heads.clone(),
        generation: generation.clone(),
        parents_range: parents_range.clone(),
    }))
}

/// Transforms negative intersection to difference. Redundant intersections like
/// `all() & e` should have been removed.
fn fold_difference<St: ExpressionState>(
    expression: &Arc<RevsetExpression<St>>,
) -> TransformedExpression<St> {
    fn to_difference<St: ExpressionState>(
        expression: &Arc<RevsetExpression<St>>,
        complement: &Arc<RevsetExpression<St>>,
    ) -> Arc<RevsetExpression<St>> {
        to_difference_range(expression, complement).unwrap_or_else(|| expression.minus(complement))
    }

    transform_expression_bottom_up(expression, |expression| match expression.as_ref() {
        RevsetExpression::Intersection(expression1, expression2) => {
            match (expression1.as_ref(), expression2.as_ref()) {
                // For '~x & f', don't move filter node 'f' left
                (_, RevsetExpression::Filter(_) | RevsetExpression::AsFilter(_)) => None,
                (_, RevsetExpression::NotIn(complement)) => {
                    Some(to_difference(expression1, complement))
                }
                (RevsetExpression::NotIn(complement), _) => {
                    Some(to_difference(expression2, complement))
                }
                _ => None,
            }
        }
        _ => None,
    })
}

/// Transforms remaining negated ancestors `~(::h)` to range `h..`.
///
/// Since this rule inserts redundant `visible_heads()`, negative intersections
/// should have been transformed.
fn fold_not_in_ancestors<St: ExpressionState>(
    expression: &Arc<RevsetExpression<St>>,
) -> TransformedExpression<St> {
    transform_expression_bottom_up(expression, |expression| match expression.as_ref() {
        RevsetExpression::NotIn(complement)
            if matches!(complement.as_ref(), RevsetExpression::Ancestors { .. }) =>
        {
            // ~(::heads) -> heads..
            // ~(::heads-) -> ~ancestors(heads, 1..) -> heads-..
            to_difference_range(
                &RevsetExpression::visible_heads_or_referenced().ancestors(),
                complement,
            )
        }
        _ => None,
    })
}

/// Transforms binary difference to more primitive negative intersection.
///
/// For example, `all() ~ e` will become `all() & ~e`, which can be simplified
/// further by `fold_redundant_expression()`.
fn unfold_difference<St: ExpressionState>(
    expression: &Arc<RevsetExpression<St>>,
) -> TransformedExpression<St> {
    transform_expression_bottom_up(expression, |expression| match expression.as_ref() {
        // roots..heads -> ::heads & ~(::roots)
        RevsetExpression::Range {
            roots,
            heads,
            parents_range,
            generation,
        } => {
            let heads_ancestors = Arc::new(RevsetExpression::Ancestors {
                heads: heads.clone(),
                generation: generation.clone(),
                parents_range: parents_range.clone(),
            });
            Some(heads_ancestors.intersection(&roots.ancestors().negated()))
        }
        RevsetExpression::Difference(expression1, expression2) => {
            Some(expression1.intersection(&expression2.negated()))
        }
        _ => None,
    })
}

/// Transforms nested `ancestors()`/`parents()`/`descendants()`/`children()`
/// like `h---`/`r+++`.
fn fold_generation<St: ExpressionState>(
    expression: &Arc<RevsetExpression<St>>,
) -> TransformedExpression<St> {
    fn add_generation(generation1: &Range<u64>, generation2: &Range<u64>) -> Range<u64> {
        // For any (g1, g2) in (generation1, generation2), g1 + g2.
        if generation1.is_empty() || generation2.is_empty() {
            GENERATION_RANGE_EMPTY
        } else {
            let start = u64::saturating_add(generation1.start, generation2.start);
            let end = u64::saturating_add(generation1.end, generation2.end - 1);
            start..end
        }
    }

    transform_expression_bottom_up(expression, |expression| match expression.as_ref() {
        RevsetExpression::Ancestors {
            heads,
            generation: generation1,
            parents_range: parents1,
        } => {
            match heads.as_ref() {
                // (h-)- -> ancestors(ancestors(h, 1), 1) -> ancestors(h, 2)
                // ::(h-) -> ancestors(ancestors(h, 1), ..) -> ancestors(h, 1..)
                // (::h)- -> ancestors(ancestors(h, ..), 1) -> ancestors(h, 1..)
                RevsetExpression::Ancestors {
                    heads,
                    generation: generation2,
                    parents_range: parents2,
                } if parents2 == parents1 => Some(Arc::new(RevsetExpression::Ancestors {
                    heads: heads.clone(),
                    generation: add_generation(generation1, generation2),
                    parents_range: parents1.clone(),
                })),
                _ => None,
            }
        }
        RevsetExpression::Descendants {
            roots,
            generation: generation1,
        } => {
            match roots.as_ref() {
                // (r+)+ -> descendants(descendants(r, 1), 1) -> descendants(r, 2)
                // (r+):: -> descendants(descendants(r, 1), ..) -> descendants(r, 1..)
                // (r::)+ -> descendants(descendants(r, ..), 1) -> descendants(r, 1..)
                RevsetExpression::Descendants {
                    roots,
                    generation: generation2,
                } => Some(Arc::new(RevsetExpression::Descendants {
                    roots: roots.clone(),
                    generation: add_generation(generation1, generation2),
                })),
                _ => None,
            }
        }
        // Range should have been unfolded to intersection of Ancestors.
        _ => None,
    })
}

/// Rewrites the given `expression` tree to reduce evaluation cost. Returns new
/// tree.
pub fn optimize<St: ExpressionState>(
    expression: Arc<RevsetExpression<St>>,
) -> Arc<RevsetExpression<St>> {
    // Since fold_redundant_expression() can remove hidden commits that look
    // redundant, referenced commits should be collected earlier.
    let expression = resolve_referenced_commits(&expression).unwrap_or(expression);
    let expression = unfold_difference(&expression).unwrap_or(expression);
    let expression = fold_redundant_expression(&expression).unwrap_or(expression);
    let expression = fold_generation(&expression).unwrap_or(expression);
    let expression = flatten_intersections(&expression).unwrap_or(expression);
    let expression = sort_negations_and_ancestors(&expression).unwrap_or(expression);
    let expression = fold_ancestors_union(&expression).unwrap_or(expression);
    let expression = internalize_filter(&expression).unwrap_or(expression);
    let expression = fold_heads_range(&expression).unwrap_or(expression);
    let expression = fold_difference(&expression).unwrap_or(expression);
    fold_not_in_ancestors(&expression).unwrap_or(expression)
}

// TODO: find better place to host this function (or add compile-time revset
// parsing and resolution like
// `revset!("{unwanted}..{wanted}").evaluate(repo)`?)
pub fn walk_revs<'index>(
    repo: &'index dyn Repo,
    wanted: &[CommitId],
    unwanted: &[CommitId],
) -> Result<Box<dyn Revset + 'index>, RevsetEvaluationError> {
    RevsetExpression::commits(unwanted.to_vec())
        .range(&RevsetExpression::commits(wanted.to_vec()))
        .evaluate(repo)
}

fn reload_repo_at_operation(
    repo: &dyn Repo,
    op_str: &str,
) -> Result<Arc<ReadonlyRepo>, RevsetResolutionError> {
    // TODO: Maybe we should ensure that the resolved operation is an ancestor
    // of the current operation. If it weren't, there might be commits unknown
    // to the outer repo.
    let base_repo = repo.base_repo();
    let operation = op_walk::resolve_op_with_repo(base_repo, op_str)
        .map_err(|err| RevsetResolutionError::Other(err.into()))?;
    base_repo.reload_at(&operation).map_err(|err| match err {
        RepoLoaderError::Backend(err) => RevsetResolutionError::Backend(err),
        RepoLoaderError::Index(_)
        | RepoLoaderError::IndexStore(_)
        | RepoLoaderError::OpHeadResolution(_)
        | RepoLoaderError::OpHeadsStoreError(_)
        | RepoLoaderError::OpStore(_)
        | RepoLoaderError::TransactionCommit(_) => RevsetResolutionError::Other(err.into()),
    })
}

fn resolve_remote_bookmark(
    repo: &dyn Repo,
    symbol: RemoteRefSymbol<'_>,
) -> Result<CommitId, RevsetResolutionError> {
    let target = &repo.view().get_remote_bookmark(symbol).target;
    to_resolved_ref("remote_bookmark", symbol, target)?
        .ok_or_else(|| make_no_such_symbol_error(repo, symbol.to_string()))
}

fn to_resolved_ref(
    kind: &'static str,
    symbol: impl ToString,
    target: &RefTarget,
) -> Result<Option<CommitId>, RevsetResolutionError> {
    match target.as_resolved() {
        Some(Some(id)) => Ok(Some(id.clone())),
        Some(None) => Ok(None),
        None => Err(RevsetResolutionError::ConflictedRef {
            kind,
            symbol: symbol.to_string(),
            targets: target.added_ids().cloned().collect(),
        }),
    }
}

fn all_formatted_bookmark_symbols(
    repo: &dyn Repo,
    include_synced_remotes: bool,
) -> impl Iterator<Item = String> {
    let view = repo.view();
    view.bookmarks().flat_map(move |(name, bookmark_target)| {
        let local_target = bookmark_target.local_target;
        let local_symbol = local_target
            .is_present()
            .then(|| format_symbol(name.as_str()));
        let remote_symbols = bookmark_target
            .remote_refs
            .into_iter()
            .filter(move |&(_, remote_ref)| {
                include_synced_remotes
                    || !remote_ref.is_tracked()
                    || remote_ref.target != *local_target
            })
            .map(move |(remote, _)| format_remote_symbol(name.as_str(), remote.as_str()));
        local_symbol.into_iter().chain(remote_symbols)
    })
}

fn make_no_such_symbol_error(repo: &dyn Repo, name: String) -> RevsetResolutionError {
    // TODO: include tags?
    let bookmark_names = all_formatted_bookmark_symbols(repo, name.contains('@'));
    let candidates = collect_similar(&name, bookmark_names);
    RevsetResolutionError::NoSuchRevision { name, candidates }
}

/// A symbol resolver for a specific namespace of labels.
///
/// Returns None if it cannot handle the symbol.
pub trait PartialSymbolResolver {
    fn resolve_symbol(
        &self,
        repo: &dyn Repo,
        symbol: &str,
    ) -> Result<Option<CommitId>, RevsetResolutionError>;
}

struct TagResolver;

impl PartialSymbolResolver for TagResolver {
    fn resolve_symbol(
        &self,
        repo: &dyn Repo,
        symbol: &str,
    ) -> Result<Option<CommitId>, RevsetResolutionError> {
        let target = repo.view().get_local_tag(symbol.as_ref());
        to_resolved_ref("tag", symbol, target)
    }
}

struct BookmarkResolver;

impl PartialSymbolResolver for BookmarkResolver {
    fn resolve_symbol(
        &self,
        repo: &dyn Repo,
        symbol: &str,
    ) -> Result<Option<CommitId>, RevsetResolutionError> {
        let target = repo.view().get_local_bookmark(symbol.as_ref());
        to_resolved_ref("bookmark", symbol, target)
    }
}

struct GitRefResolver;

impl PartialSymbolResolver for GitRefResolver {
    fn resolve_symbol(
        &self,
        repo: &dyn Repo,
        symbol: &str,
    ) -> Result<Option<CommitId>, RevsetResolutionError> {
        let view = repo.view();
        for git_ref_prefix in &["", "refs/"] {
            let target = view.get_git_ref([git_ref_prefix, symbol].concat().as_ref());
            if let Some(id) = to_resolved_ref("git_ref", symbol, target)? {
                return Ok(Some(id));
            }
        }
        Ok(None)
    }
}

const DEFAULT_RESOLVERS: &[&dyn PartialSymbolResolver] =
    &[&TagResolver, &BookmarkResolver, &GitRefResolver];

struct CommitPrefixResolver<'a> {
    context_repo: &'a dyn Repo,
    context: Option<&'a IdPrefixContext>,
}

impl CommitPrefixResolver<'_> {
    fn try_resolve(
        &self,
        repo: &dyn Repo,
        prefix: &HexPrefix,
    ) -> Result<Option<CommitId>, RevsetResolutionError> {
        let index = self
            .context
            .map(|ctx| ctx.populate(self.context_repo))
            .transpose()
            .map_err(|err| RevsetResolutionError::Other(err.into()))?
            .unwrap_or(IdPrefixIndex::empty());
        match index
            .resolve_commit_prefix(repo, prefix)
            .map_err(|err| RevsetResolutionError::Other(err.into()))?
        {
            PrefixResolution::AmbiguousMatch => {
                Err(RevsetResolutionError::AmbiguousCommitIdPrefix(prefix.hex()))
            }
            PrefixResolution::SingleMatch(id) => Ok(Some(id)),
            PrefixResolution::NoMatch => Ok(None),
        }
    }
}

impl PartialSymbolResolver for CommitPrefixResolver<'_> {
    fn resolve_symbol(
        &self,
        repo: &dyn Repo,
        symbol: &str,
    ) -> Result<Option<CommitId>, RevsetResolutionError> {
        if let Some(prefix) = HexPrefix::try_from_hex(symbol) {
            self.try_resolve(repo, &prefix)
        } else {
            Ok(None)
        }
    }
}

struct ChangePrefixResolver<'a> {
    context_repo: &'a dyn Repo,
    context: Option<&'a IdPrefixContext>,
}

impl ChangePrefixResolver<'_> {
    fn try_resolve(
        &self,
        repo: &dyn Repo,
        prefix: &HexPrefix,
    ) -> Result<Option<Vec<CommitId>>, RevsetResolutionError> {
        let index = self
            .context
            .map(|ctx| ctx.populate(self.context_repo))
            .transpose()
            .map_err(|err| RevsetResolutionError::Other(err.into()))?
            .unwrap_or(IdPrefixIndex::empty());
        match index
            .resolve_change_prefix(repo, prefix)
            .map_err(|err| RevsetResolutionError::Other(err.into()))?
        {
            PrefixResolution::AmbiguousMatch => Err(
                RevsetResolutionError::AmbiguousChangeIdPrefix(prefix.reverse_hex()),
            ),
            PrefixResolution::SingleMatch(ids) => Ok(Some(ids)),
            PrefixResolution::NoMatch => Ok(None),
        }
    }
}

impl PartialSymbolResolver for ChangePrefixResolver<'_> {
    fn resolve_symbol(
        &self,
        repo: &dyn Repo,
        symbol: &str,
    ) -> Result<Option<CommitId>, RevsetResolutionError> {
        if let Some(prefix) = HexPrefix::try_from_reverse_hex(symbol) {
            match self.try_resolve(repo, &prefix)? {
                Some(targets) if targets.len() == 1 => Ok(targets.into_iter().next()),
                Some(targets) => Err(RevsetResolutionError::DivergentChangeId {
                    symbol: symbol.to_owned(),
                    targets,
                }),
                None => Ok(None),
            }
        } else {
            Ok(None)
        }
    }
}

/// An extension of the [`SymbolResolver`].
///
/// Each PartialSymbolResolver will be invoked in order, its result used if one
/// is provided. Native resolvers are always invoked first. In the future, we
/// may provide a way for extensions to override native resolvers like tags and
/// bookmarks.
pub trait SymbolResolverExtension: Send + Sync {
    /// PartialSymbolResolvers can initialize some global data by using the
    /// `context_repo`, but the `context_repo` may point to a different
    /// operation from the `repo` passed into `resolve_symbol()`. For
    /// resolution, the latter `repo` should be used.
    fn new_resolvers<'a>(
        &self,
        context_repo: &'a dyn Repo,
    ) -> Vec<Box<dyn PartialSymbolResolver + 'a>>;
}

/// Resolves bookmarks, remote bookmarks, tags, git refs, and full and
/// abbreviated commit and change ids.
pub struct SymbolResolver<'a> {
    commit_id_resolver: CommitPrefixResolver<'a>,
    change_id_resolver: ChangePrefixResolver<'a>,
    extensions: Vec<Box<dyn PartialSymbolResolver + 'a>>,
}

impl<'a> SymbolResolver<'a> {
    /// Creates new symbol resolver that will first disambiguate short ID
    /// prefixes within the given `context_repo` if configured.
    pub fn new(
        context_repo: &'a dyn Repo,
        extensions: &[impl AsRef<dyn SymbolResolverExtension>],
    ) -> Self {
        SymbolResolver {
            commit_id_resolver: CommitPrefixResolver {
                context_repo,
                context: None,
            },
            change_id_resolver: ChangePrefixResolver {
                context_repo,
                context: None,
            },
            extensions: extensions
                .iter()
                .flat_map(|ext| ext.as_ref().new_resolvers(context_repo))
                .collect(),
        }
    }

    pub fn with_id_prefix_context(mut self, id_prefix_context: &'a IdPrefixContext) -> Self {
        self.commit_id_resolver.context = Some(id_prefix_context);
        self.change_id_resolver.context = Some(id_prefix_context);
        self
    }

    fn partial_resolvers(&self) -> impl Iterator<Item = &(dyn PartialSymbolResolver + 'a)> {
        let prefix_resolvers: [&dyn PartialSymbolResolver; 2] =
            [&self.commit_id_resolver, &self.change_id_resolver];
        itertools::chain!(
            DEFAULT_RESOLVERS.iter().copied(),
            prefix_resolvers,
            self.extensions.iter().map(|e| e.as_ref())
        )
    }

    /// Looks up `symbol` in the given `repo`.
    pub fn resolve_symbol(
        &self,
        repo: &dyn Repo,
        symbol: &str,
    ) -> Result<CommitId, RevsetResolutionError> {
        if symbol.is_empty() {
            return Err(RevsetResolutionError::EmptyString);
        }

        for partial_resolver in self.partial_resolvers() {
            if let Some(id) = partial_resolver.resolve_symbol(repo, symbol)? {
                return Ok(id);
            }
        }

        Err(make_no_such_symbol_error(repo, format_symbol(symbol)))
    }
}

fn resolve_commit_ref(
    repo: &dyn Repo,
    commit_ref: &RevsetCommitRef,
    symbol_resolver: &SymbolResolver,
) -> Result<Vec<CommitId>, RevsetResolutionError> {
    match commit_ref {
        RevsetCommitRef::Symbol(symbol) => {
            let commit_id = symbol_resolver.resolve_symbol(repo, symbol)?;
            Ok(vec![commit_id])
        }
        RevsetCommitRef::RemoteSymbol(symbol) => {
            let commit_id = resolve_remote_bookmark(repo, symbol.as_ref())?;
            Ok(vec![commit_id])
        }
        RevsetCommitRef::WorkingCopy(name) => {
            if let Some(commit_id) = repo.view().get_wc_commit_id(name) {
                Ok(vec![commit_id.clone()])
            } else {
                Err(RevsetResolutionError::WorkspaceMissingWorkingCopy { name: name.clone() })
            }
        }
        RevsetCommitRef::WorkingCopies => {
            let wc_commits = repo.view().wc_commit_ids().values().cloned().collect_vec();
            Ok(wc_commits)
        }
        RevsetCommitRef::ChangeId(prefix) => {
            let resolver = &symbol_resolver.change_id_resolver;
            Ok(resolver.try_resolve(repo, prefix)?.unwrap_or_else(Vec::new))
        }
        RevsetCommitRef::CommitId(prefix) => {
            let resolver = &symbol_resolver.commit_id_resolver;
            Ok(resolver.try_resolve(repo, prefix)?.into_iter().collect())
        }
        RevsetCommitRef::Bookmarks(expression) => {
            let commit_ids = repo
                .view()
                .local_bookmarks_matching(&expression.to_matcher())
                .flat_map(|(_, target)| target.added_ids())
                .cloned()
                .collect();
            Ok(commit_ids)
        }
        RevsetCommitRef::RemoteBookmarks {
            bookmark,
            remote,
            remote_ref_state,
        } => {
            let bookmark_matcher = bookmark.to_matcher();
            let remote_matcher = remote.to_matcher();
            let commit_ids = repo
                .view()
                .remote_bookmarks_matching(&bookmark_matcher, &remote_matcher)
                .filter(|(_, remote_ref)| {
                    remote_ref_state.is_none_or(|state| remote_ref.state == state)
                })
                .flat_map(|(_, remote_ref)| remote_ref.target.added_ids())
                .cloned()
                .collect();
            Ok(commit_ids)
        }
        RevsetCommitRef::Tags(expression) => {
            let commit_ids = repo
                .view()
                .local_tags_matching(&expression.to_matcher())
                .flat_map(|(_, target)| target.added_ids())
                .cloned()
                .collect();
            Ok(commit_ids)
        }
        RevsetCommitRef::GitRefs => {
            let mut commit_ids = vec![];
            for ref_target in repo.view().git_refs().values() {
                commit_ids.extend(ref_target.added_ids().cloned());
            }
            Ok(commit_ids)
        }
        RevsetCommitRef::GitHead => Ok(repo.view().git_head().added_ids().cloned().collect()),
    }
}

/// Resolves symbols and commit refs recursively.
struct ExpressionSymbolResolver<'a, 'b> {
    base_repo: &'a dyn Repo,
    repo_stack: Vec<Arc<ReadonlyRepo>>,
    symbol_resolver: &'a SymbolResolver<'b>,
}

impl<'a, 'b> ExpressionSymbolResolver<'a, 'b> {
    fn new(base_repo: &'a dyn Repo, symbol_resolver: &'a SymbolResolver<'b>) -> Self {
        Self {
            base_repo,
            repo_stack: vec![],
            symbol_resolver,
        }
    }

    fn repo(&self) -> &dyn Repo {
        self.repo_stack
            .last()
            .map_or(self.base_repo, |repo| repo.as_ref())
    }
}

impl ExpressionStateFolder<UserExpressionState, ResolvedExpressionState>
    for ExpressionSymbolResolver<'_, '_>
{
    type Error = RevsetResolutionError;

    fn fold_expression(
        &mut self,
        expression: &UserRevsetExpression,
    ) -> Result<Arc<ResolvedRevsetExpression>, Self::Error> {
        match expression {
            // 'present(x)' opens new symbol resolution scope to map error to 'none()'
            RevsetExpression::Present(candidates) => {
                self.fold_expression(candidates).or_else(|err| match err {
                    RevsetResolutionError::NoSuchRevision { .. }
                    | RevsetResolutionError::WorkspaceMissingWorkingCopy { .. } => {
                        Ok(RevsetExpression::none())
                    }
                    RevsetResolutionError::EmptyString
                    | RevsetResolutionError::AmbiguousCommitIdPrefix(_)
                    | RevsetResolutionError::AmbiguousChangeIdPrefix(_)
                    | RevsetResolutionError::DivergentChangeId { .. }
                    | RevsetResolutionError::ConflictedRef { .. }
                    | RevsetResolutionError::Backend(_)
                    | RevsetResolutionError::Other(_) => Err(err),
                })
            }
            _ => fold_child_expression_state(self, expression),
        }
    }

    fn fold_commit_ref(
        &mut self,
        commit_ref: &RevsetCommitRef,
    ) -> Result<Arc<ResolvedRevsetExpression>, Self::Error> {
        let commit_ids = resolve_commit_ref(self.repo(), commit_ref, self.symbol_resolver)?;
        Ok(RevsetExpression::commits(commit_ids))
    }

    fn fold_at_operation(
        &mut self,
        operation: &String,
        candidates: &UserRevsetExpression,
    ) -> Result<Arc<ResolvedRevsetExpression>, Self::Error> {
        let repo = reload_repo_at_operation(self.repo(), operation)?;
        self.repo_stack.push(repo);
        let candidates = self.fold_expression(candidates)?;
        let visible_heads = self.repo().view().heads().iter().cloned().collect();
        self.repo_stack.pop();
        Ok(Arc::new(RevsetExpression::WithinVisibility {
            candidates,
            visible_heads,
        }))
    }
}

fn resolve_symbols(
    repo: &dyn Repo,
    expression: &UserRevsetExpression,
    symbol_resolver: &SymbolResolver,
) -> Result<Arc<ResolvedRevsetExpression>, RevsetResolutionError> {
    let mut resolver = ExpressionSymbolResolver::new(repo, symbol_resolver);
    resolver.fold_expression(expression)
}

/// Inserts implicit `all()` and `visible_heads()` nodes to the `expression`.
///
/// Symbols and commit refs in the `expression` should have been resolved.
///
/// This is a separate step because a symbol-resolved `expression` may be
/// transformed further to e.g. combine OR-ed `Commits(_)`, or to collect
/// commit ids to make `all()` include hidden-but-specified commits. The
/// return type `ResolvedExpression` is stricter than `RevsetExpression`,
/// and isn't designed for such transformation.
fn resolve_visibility(
    repo: &dyn Repo,
    expression: &ResolvedRevsetExpression,
) -> ResolvedExpression {
    let context = VisibilityResolutionContext {
        referenced_commits: &[],
        visible_heads: &repo.view().heads().iter().cloned().collect_vec(),
        root: repo.store().root_commit_id(),
    };
    context.resolve(expression)
}

#[derive(Clone, Debug)]
struct VisibilityResolutionContext<'a> {
    referenced_commits: &'a [CommitId],
    visible_heads: &'a [CommitId],
    root: &'a CommitId,
}

impl VisibilityResolutionContext<'_> {
    /// Resolves expression tree as set.
    fn resolve(&self, expression: &ResolvedRevsetExpression) -> ResolvedExpression {
        match expression {
            RevsetExpression::None => ResolvedExpression::Commits(vec![]),
            RevsetExpression::All => self.resolve_all(),
            RevsetExpression::VisibleHeads => self.resolve_visible_heads(),
            RevsetExpression::VisibleHeadsOrReferenced => {
                self.resolve_visible_heads_or_referenced()
            }
            RevsetExpression::Root => self.resolve_root(),
            RevsetExpression::Commits(commit_ids) => {
                ResolvedExpression::Commits(commit_ids.clone())
            }
            RevsetExpression::CommitRef(commit_ref) => match *commit_ref {},
            RevsetExpression::Ancestors {
                heads,
                generation,
                parents_range,
            } => ResolvedExpression::Ancestors {
                heads: self.resolve(heads).into(),
                generation: generation.clone(),
                parents_range: parents_range.clone(),
            },
            RevsetExpression::Descendants { roots, generation } => ResolvedExpression::DagRange {
                roots: self.resolve(roots).into(),
                heads: self.resolve_visible_heads_or_referenced().into(),
                generation_from_roots: generation.clone(),
            },
            RevsetExpression::Range {
                roots,
                heads,
                generation,
                parents_range,
            } => ResolvedExpression::Range {
                roots: self.resolve(roots).into(),
                heads: self.resolve(heads).into(),
                generation: generation.clone(),
                parents_range: parents_range.clone(),
            },
            RevsetExpression::DagRange { roots, heads } => ResolvedExpression::DagRange {
                roots: self.resolve(roots).into(),
                heads: self.resolve(heads).into(),
                generation_from_roots: GENERATION_RANGE_FULL,
            },
            RevsetExpression::Reachable { sources, domain } => ResolvedExpression::Reachable {
                sources: self.resolve(sources).into(),
                domain: self.resolve(domain).into(),
            },
            RevsetExpression::Heads(candidates) => {
                ResolvedExpression::Heads(self.resolve(candidates).into())
            }
            RevsetExpression::HeadsRange {
                roots,
                heads,
                parents_range,
                filter,
            } => ResolvedExpression::HeadsRange {
                roots: self.resolve(roots).into(),
                heads: self.resolve(heads).into(),
                parents_range: parents_range.clone(),
                filter: (!matches!(filter.as_ref(), RevsetExpression::All))
                    .then(|| self.resolve_predicate(filter)),
            },
            RevsetExpression::Roots(candidates) => {
                ResolvedExpression::Roots(self.resolve(candidates).into())
            }
            RevsetExpression::ForkPoint(expression) => {
                ResolvedExpression::ForkPoint(self.resolve(expression).into())
            }
            RevsetExpression::Bisect(expression) => {
                ResolvedExpression::Bisect(self.resolve(expression).into())
            }
            RevsetExpression::Latest { candidates, count } => ResolvedExpression::Latest {
                candidates: self.resolve(candidates).into(),
                count: *count,
            },
            RevsetExpression::HasSize { candidates, count } => ResolvedExpression::HasSize {
                candidates: self.resolve(candidates).into(),
                count: *count,
            },
            RevsetExpression::Filter(_) | RevsetExpression::AsFilter(_) => {
                // Top-level filter without intersection: e.g. "~author(_)" is represented as
                // `AsFilter(NotIn(Filter(Author(_))))`.
                ResolvedExpression::FilterWithin {
                    candidates: self.resolve_all().into(),
                    predicate: self.resolve_predicate(expression),
                }
            }
            RevsetExpression::AtOperation { operation, .. } => match *operation {},
            RevsetExpression::WithinReference {
                candidates,
                commits,
            } => {
                let context = VisibilityResolutionContext {
                    referenced_commits: commits,
                    visible_heads: self.visible_heads,
                    root: self.root,
                };
                context.resolve(candidates)
            }
            RevsetExpression::WithinVisibility {
                candidates,
                visible_heads,
            } => {
                let context = VisibilityResolutionContext {
                    referenced_commits: self.referenced_commits,
                    visible_heads,
                    root: self.root,
                };
                context.resolve(candidates)
            }
            RevsetExpression::Coalesce(expression1, expression2) => ResolvedExpression::Coalesce(
                self.resolve(expression1).into(),
                self.resolve(expression2).into(),
            ),
            // present(x) is noop if x doesn't contain any commit refs.
            RevsetExpression::Present(candidates) => self.resolve(candidates),
            RevsetExpression::NotIn(complement) => ResolvedExpression::Difference(
                self.resolve_all().into(),
                self.resolve(complement).into(),
            ),
            RevsetExpression::Union(expression1, expression2) => ResolvedExpression::Union(
                self.resolve(expression1).into(),
                self.resolve(expression2).into(),
            ),
            RevsetExpression::Intersection(expression1, expression2) => {
                match expression2.as_ref() {
                    RevsetExpression::Filter(_) | RevsetExpression::AsFilter(_) => {
                        ResolvedExpression::FilterWithin {
                            candidates: self.resolve(expression1).into(),
                            predicate: self.resolve_predicate(expression2),
                        }
                    }
                    _ => ResolvedExpression::Intersection(
                        self.resolve(expression1).into(),
                        self.resolve(expression2).into(),
                    ),
                }
            }
            RevsetExpression::Difference(expression1, expression2) => {
                ResolvedExpression::Difference(
                    self.resolve(expression1).into(),
                    self.resolve(expression2).into(),
                )
            }
        }
    }

    fn resolve_all(&self) -> ResolvedExpression {
        ResolvedExpression::Ancestors {
            heads: self.resolve_visible_heads_or_referenced().into(),
            generation: GENERATION_RANGE_FULL,
            parents_range: PARENTS_RANGE_FULL,
        }
    }

    fn resolve_visible_heads(&self) -> ResolvedExpression {
        ResolvedExpression::Commits(self.visible_heads.to_owned())
    }

    fn resolve_visible_heads_or_referenced(&self) -> ResolvedExpression {
        // The referenced commits may be hidden. If they weren't included in
        // `all()`, some of the logical transformation rules might subtly change
        // the evaluated set. For example, `all() & x` wouldn't be `x` if `x`
        // were hidden and if not included in `all()`.
        let commits = itertools::chain(self.referenced_commits, self.visible_heads)
            .cloned()
            .collect();
        ResolvedExpression::Commits(commits)
    }

    fn resolve_root(&self) -> ResolvedExpression {
        ResolvedExpression::Commits(vec![self.root.to_owned()])
    }

    /// Resolves expression tree as filter predicate.
    ///
    /// For filter expression, this never inserts a hidden `all()` since a
    /// filter predicate doesn't need to produce revisions to walk.
    fn resolve_predicate(
        &self,
        expression: &ResolvedRevsetExpression,
    ) -> ResolvedPredicateExpression {
        match expression {
            RevsetExpression::None
            | RevsetExpression::All
            | RevsetExpression::VisibleHeads
            | RevsetExpression::VisibleHeadsOrReferenced
            | RevsetExpression::Root
            | RevsetExpression::Commits(_)
            | RevsetExpression::CommitRef(_)
            | RevsetExpression::Ancestors { .. }
            | RevsetExpression::Descendants { .. }
            | RevsetExpression::Range { .. }
            | RevsetExpression::DagRange { .. }
            | RevsetExpression::Reachable { .. }
            | RevsetExpression::Heads(_)
            | RevsetExpression::HeadsRange { .. }
            | RevsetExpression::Roots(_)
            | RevsetExpression::ForkPoint(_)
            | RevsetExpression::Bisect(_)
            | RevsetExpression::HasSize { .. }
            | RevsetExpression::Latest { .. } => {
                ResolvedPredicateExpression::Set(self.resolve(expression).into())
            }
            RevsetExpression::Filter(predicate) => {
                ResolvedPredicateExpression::Filter(predicate.clone())
            }
            RevsetExpression::AsFilter(candidates) => self.resolve_predicate(candidates),
            RevsetExpression::AtOperation { operation, .. } => match *operation {},
            // Filters should be intersected with all() within the at-op repo.
            RevsetExpression::WithinReference { .. }
            | RevsetExpression::WithinVisibility { .. } => {
                ResolvedPredicateExpression::Set(self.resolve(expression).into())
            }
            RevsetExpression::Coalesce(_, _) => {
                ResolvedPredicateExpression::Set(self.resolve(expression).into())
            }
            // present(x) is noop if x doesn't contain any commit refs.
            RevsetExpression::Present(candidates) => self.resolve_predicate(candidates),
            RevsetExpression::NotIn(complement) => {
                ResolvedPredicateExpression::NotIn(self.resolve_predicate(complement).into())
            }
            RevsetExpression::Union(expression1, expression2) => {
                let predicate1 = self.resolve_predicate(expression1);
                let predicate2 = self.resolve_predicate(expression2);
                ResolvedPredicateExpression::Union(predicate1.into(), predicate2.into())
            }
            RevsetExpression::Intersection(expression1, expression2) => {
                let predicate1 = self.resolve_predicate(expression1);
                let predicate2 = self.resolve_predicate(expression2);
                ResolvedPredicateExpression::Intersection(predicate1.into(), predicate2.into())
            }
            RevsetExpression::Difference(expression1, expression2) => {
                let predicate1 = self.resolve_predicate(expression1);
                let predicate2 = self.resolve_predicate(expression2);
                let predicate2 = ResolvedPredicateExpression::NotIn(predicate2.into());
                ResolvedPredicateExpression::Intersection(predicate1.into(), predicate2.into())
            }
        }
    }
}

pub trait Revset: fmt::Debug {
    /// Iterate in topological order with children before parents.
    fn iter<'a>(&self) -> Box<dyn Iterator<Item = Result<CommitId, RevsetEvaluationError>> + 'a>
    where
        Self: 'a;

    /// Iterates commit/change id pairs in topological order.
    fn commit_change_ids<'a>(
        &self,
    ) -> Box<dyn Iterator<Item = Result<(CommitId, ChangeId), RevsetEvaluationError>> + 'a>
    where
        Self: 'a;

    fn iter_graph<'a>(
        &self,
    ) -> Box<dyn Iterator<Item = Result<GraphNode<CommitId>, RevsetEvaluationError>> + 'a>
    where
        Self: 'a;

    /// Returns true if iterator will emit no commit nor error.
    fn is_empty(&self) -> bool;

    /// Inclusive lower bound and, optionally, inclusive upper bound of how many
    /// commits are in the revset. The implementation can use its discretion as
    /// to how much effort should be put into the estimation, and how accurate
    /// the resulting estimate should be.
    fn count_estimate(&self) -> Result<(usize, Option<usize>), RevsetEvaluationError>;

    /// Returns a closure that checks if a commit is contained within the
    /// revset.
    ///
    /// The implementation may construct and maintain any necessary internal
    /// context to optimize the performance of the check.
    fn containing_fn<'a>(&self) -> Box<RevsetContainingFn<'a>>
    where
        Self: 'a;
}

/// Function that checks if a commit is contained within the revset.
pub type RevsetContainingFn<'a> = dyn Fn(&CommitId) -> Result<bool, RevsetEvaluationError> + 'a;

pub trait RevsetIteratorExt<I> {
    fn commits(self, store: &Arc<Store>) -> RevsetCommitIterator<I>;
}

impl<I: Iterator<Item = Result<CommitId, RevsetEvaluationError>>> RevsetIteratorExt<I> for I {
    fn commits(self, store: &Arc<Store>) -> RevsetCommitIterator<I> {
        RevsetCommitIterator {
            iter: self,
            store: store.clone(),
        }
    }
}

pub struct RevsetCommitIterator<I> {
    store: Arc<Store>,
    iter: I,
}

impl<I: Iterator<Item = Result<CommitId, RevsetEvaluationError>>> Iterator
    for RevsetCommitIterator<I>
{
    type Item = Result<Commit, RevsetEvaluationError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.iter.next().map(|commit_id| {
            let commit_id = commit_id?;
            self.store
                .get_commit(&commit_id)
                .map_err(RevsetEvaluationError::Backend)
        })
    }
}

/// A set of extensions for revset evaluation.
pub struct RevsetExtensions {
    symbol_resolvers: Vec<Box<dyn SymbolResolverExtension>>,
    function_map: HashMap<&'static str, RevsetFunction>,
}

impl Default for RevsetExtensions {
    fn default() -> Self {
        Self::new()
    }
}

impl RevsetExtensions {
    pub fn new() -> Self {
        Self {
            symbol_resolvers: vec![],
            function_map: BUILTIN_FUNCTION_MAP.clone(),
        }
    }

    pub fn symbol_resolvers(&self) -> &[Box<dyn SymbolResolverExtension>] {
        &self.symbol_resolvers
    }

    pub fn add_symbol_resolver(&mut self, symbol_resolver: Box<dyn SymbolResolverExtension>) {
        self.symbol_resolvers.push(symbol_resolver);
    }

    pub fn add_custom_function(&mut self, name: &'static str, func: RevsetFunction) {
        match self.function_map.entry(name) {
            hash_map::Entry::Occupied(_) => {
                panic!("Conflict registering revset function '{name}'")
            }
            hash_map::Entry::Vacant(v) => v.insert(func),
        };
    }
}

/// Information needed to parse revset expression.
#[derive(Clone)]
pub struct RevsetParseContext<'a> {
    pub aliases_map: &'a RevsetAliasesMap,
    pub local_variables: HashMap<&'a str, ExpressionNode<'a>>,
    pub user_email: &'a str,
    pub date_pattern_context: DatePatternContext,
    /// Special remote that should be ignored by default. (e.g. "git")
    pub default_ignored_remote: Option<&'a RemoteName>,
    pub extensions: &'a RevsetExtensions,
    pub workspace: Option<RevsetWorkspaceContext<'a>>,
}

impl<'a> RevsetParseContext<'a> {
    fn to_lowering_context(&self) -> LoweringContext<'a> {
        let RevsetParseContext {
            aliases_map: _,
            local_variables: _,
            user_email,
            date_pattern_context,
            default_ignored_remote,
            extensions,
            workspace,
        } = *self;
        LoweringContext {
            user_email,
            date_pattern_context,
            default_ignored_remote,
            extensions,
            workspace,
        }
    }
}

/// Information needed to transform revset AST into `UserRevsetExpression`.
#[derive(Clone)]
pub struct LoweringContext<'a> {
    user_email: &'a str,
    date_pattern_context: DatePatternContext,
    default_ignored_remote: Option<&'a RemoteName>,
    extensions: &'a RevsetExtensions,
    workspace: Option<RevsetWorkspaceContext<'a>>,
}

impl<'a> LoweringContext<'a> {
    pub fn user_email(&self) -> &'a str {
        self.user_email
    }

    pub fn date_pattern_context(&self) -> &DatePatternContext {
        &self.date_pattern_context
    }

    pub fn symbol_resolvers(&self) -> &'a [impl AsRef<dyn SymbolResolverExtension> + use<>] {
        self.extensions.symbol_resolvers()
    }
}

/// Workspace information needed to parse revset expression.
#[derive(Clone, Copy, Debug)]
pub struct RevsetWorkspaceContext<'a> {
    pub path_converter: &'a RepoPathUiConverter,
    pub workspace_name: &'a WorkspaceName,
}

/// Formats a string as symbol by quoting and escaping it if necessary.
///
/// Note that symbols may be substituted to user aliases. Use
/// [`format_string()`] to ensure that the provided string is resolved as a
/// tag/bookmark name, commit/change ID prefix, etc.
pub fn format_symbol(literal: &str) -> String {
    if revset_parser::is_identifier(literal) {
        literal.to_string()
    } else {
        format_string(literal)
    }
}

/// Formats a string by quoting and escaping it.
pub fn format_string(literal: &str) -> String {
    format!(r#""{}""#, dsl_util::escape_string(literal))
}

/// Formats a `name@remote` symbol, applies quoting and escaping if necessary.
pub fn format_remote_symbol(name: &str, remote: &str) -> String {
    let name = format_symbol(name);
    let remote = format_symbol(remote);
    format!("{name}@{remote}")
}

#[cfg(test)]
#[rustversion::attr(
    since(1.89),
    expect(clippy::cloned_ref_to_slice_refs, reason = "makes tests more readable")
)]
mod tests {
    use std::path::PathBuf;

    use assert_matches::assert_matches;

    use super::*;

    fn parse(revset_str: &str) -> Result<Arc<UserRevsetExpression>, RevsetParseError> {
        parse_with_aliases(revset_str, [] as [(&str, &str); 0])
    }

    fn parse_with_workspace(
        revset_str: &str,
        workspace_name: &WorkspaceName,
    ) -> Result<Arc<UserRevsetExpression>, RevsetParseError> {
        parse_with_aliases_and_workspace(revset_str, [] as [(&str, &str); 0], workspace_name)
    }

    fn parse_with_aliases(
        revset_str: &str,
        aliases: impl IntoIterator<Item = (impl AsRef<str>, impl Into<String>)>,
    ) -> Result<Arc<UserRevsetExpression>, RevsetParseError> {
        let mut aliases_map = RevsetAliasesMap::new();
        for (decl, defn) in aliases {
            aliases_map.insert(decl, defn).unwrap();
        }
        let context = RevsetParseContext {
            aliases_map: &aliases_map,
            local_variables: HashMap::new(),
            user_email: "test.user@example.com",
            date_pattern_context: chrono::Utc::now().fixed_offset().into(),
            default_ignored_remote: Some("ignored".as_ref()),
            extensions: &RevsetExtensions::default(),
            workspace: None,
        };
        super::parse(&mut RevsetDiagnostics::new(), revset_str, &context)
    }

    fn parse_with_aliases_and_workspace(
        revset_str: &str,
        aliases: impl IntoIterator<Item = (impl AsRef<str>, impl Into<String>)>,
        workspace_name: &WorkspaceName,
    ) -> Result<Arc<UserRevsetExpression>, RevsetParseError> {
        // Set up pseudo context to resolve `workspace_name@` and `file(path)`
        let path_converter = RepoPathUiConverter::Fs {
            cwd: PathBuf::from("/"),
            base: PathBuf::from("/"),
        };
        let workspace_ctx = RevsetWorkspaceContext {
            path_converter: &path_converter,
            workspace_name,
        };
        let mut aliases_map = RevsetAliasesMap::new();
        for (decl, defn) in aliases {
            aliases_map.insert(decl, defn).unwrap();
        }
        let context = RevsetParseContext {
            aliases_map: &aliases_map,
            local_variables: HashMap::new(),
            user_email: "test.user@example.com",
            date_pattern_context: chrono::Utc::now().fixed_offset().into(),
            default_ignored_remote: Some("ignored".as_ref()),
            extensions: &RevsetExtensions::default(),
            workspace: Some(workspace_ctx),
        };
        super::parse(&mut RevsetDiagnostics::new(), revset_str, &context)
    }

    fn parse_with_modifier(
        revset_str: &str,
    ) -> Result<(Arc<UserRevsetExpression>, Option<RevsetModifier>), RevsetParseError> {
        parse_with_aliases_and_modifier(revset_str, [] as [(&str, &str); 0])
    }

    fn parse_with_aliases_and_modifier(
        revset_str: &str,
        aliases: impl IntoIterator<Item = (impl AsRef<str>, impl Into<String>)>,
    ) -> Result<(Arc<UserRevsetExpression>, Option<RevsetModifier>), RevsetParseError> {
        let mut aliases_map = RevsetAliasesMap::new();
        for (decl, defn) in aliases {
            aliases_map.insert(decl, defn).unwrap();
        }
        let context = RevsetParseContext {
            aliases_map: &aliases_map,
            local_variables: HashMap::new(),
            user_email: "test.user@example.com",
            date_pattern_context: chrono::Utc::now().fixed_offset().into(),
            default_ignored_remote: Some("ignored".as_ref()),
            extensions: &RevsetExtensions::default(),
            workspace: None,
        };
        super::parse_with_modifier(&mut RevsetDiagnostics::new(), revset_str, &context)
    }

    fn insta_settings() -> insta::Settings {
        let mut settings = insta::Settings::clone_current();
        // Collapse short "Thing(_,)" repeatedly to save vertical space and make
        // the output more readable.
        for _ in 0..4 {
            settings.add_filter(
                r"(?x)
                \b([A-Z]\w*)\(\n
                    \s*(.{1,60}),\n
                \s*\)",
                "$1($2)",
            );
        }
        settings
    }

    #[test]
    #[expect(clippy::redundant_clone)] // allow symbol.clone()
    fn test_revset_expression_building() {
        let settings = insta_settings();
        let _guard = settings.bind_to_scope();
        let current_wc = UserRevsetExpression::working_copy(WorkspaceName::DEFAULT.to_owned());
        let foo_symbol = UserRevsetExpression::symbol("foo".to_string());
        let bar_symbol = UserRevsetExpression::symbol("bar".to_string());
        let baz_symbol = UserRevsetExpression::symbol("baz".to_string());

        insta::assert_debug_snapshot!(
            current_wc,
            @r#"CommitRef(WorkingCopy(WorkspaceNameBuf("default")))"#);
        insta::assert_debug_snapshot!(
            current_wc.heads(),
            @r#"Heads(CommitRef(WorkingCopy(WorkspaceNameBuf("default"))))"#);
        insta::assert_debug_snapshot!(
            current_wc.roots(),
            @r#"Roots(CommitRef(WorkingCopy(WorkspaceNameBuf("default"))))"#);
        insta::assert_debug_snapshot!(
            current_wc.parents(), @r#"
        Ancestors {
            heads: CommitRef(WorkingCopy(WorkspaceNameBuf("default"))),
            generation: 1..2,
            parents_range: 0..4294967295,
        }
        "#);
        insta::assert_debug_snapshot!(
            current_wc.ancestors(), @r#"
        Ancestors {
            heads: CommitRef(WorkingCopy(WorkspaceNameBuf("default"))),
            generation: 0..18446744073709551615,
            parents_range: 0..4294967295,
        }
        "#);
        insta::assert_debug_snapshot!(
            foo_symbol.children(), @r#"
        Descendants {
            roots: CommitRef(Symbol("foo")),
            generation: 1..2,
        }
        "#);
        insta::assert_debug_snapshot!(
            foo_symbol.descendants(), @r#"
        Descendants {
            roots: CommitRef(Symbol("foo")),
            generation: 0..18446744073709551615,
        }
        "#);
        insta::assert_debug_snapshot!(
            foo_symbol.dag_range_to(&current_wc), @r#"
        DagRange {
            roots: CommitRef(Symbol("foo")),
            heads: CommitRef(WorkingCopy(WorkspaceNameBuf("default"))),
        }
        "#);
        insta::assert_debug_snapshot!(
            foo_symbol.connected(), @r#"
        DagRange {
            roots: CommitRef(Symbol("foo")),
            heads: CommitRef(Symbol("foo")),
        }
        "#);
        insta::assert_debug_snapshot!(
            foo_symbol.range(&current_wc), @r#"
        Range {
            roots: CommitRef(Symbol("foo")),
            heads: CommitRef(WorkingCopy(WorkspaceNameBuf("default"))),
            generation: 0..18446744073709551615,
            parents_range: 0..4294967295,
        }
        "#);
        insta::assert_debug_snapshot!(
            foo_symbol.negated(),
            @r#"NotIn(CommitRef(Symbol("foo")))"#);
        insta::assert_debug_snapshot!(
            foo_symbol.union(&current_wc), @r#"
        Union(
            CommitRef(Symbol("foo")),
            CommitRef(WorkingCopy(WorkspaceNameBuf("default"))),
        )
        "#);
        insta::assert_debug_snapshot!(
            UserRevsetExpression::union_all(&[]),
            @"None");
        insta::assert_debug_snapshot!(
            RevsetExpression::union_all(&[current_wc.clone()]),
            @r#"CommitRef(WorkingCopy(WorkspaceNameBuf("default")))"#);
        insta::assert_debug_snapshot!(
            RevsetExpression::union_all(&[current_wc.clone(), foo_symbol.clone()]),
            @r#"
        Union(
            CommitRef(WorkingCopy(WorkspaceNameBuf("default"))),
            CommitRef(Symbol("foo")),
        )
        "#);
        insta::assert_debug_snapshot!(
            RevsetExpression::union_all(&[
                current_wc.clone(),
                foo_symbol.clone(),
                bar_symbol.clone(),
            ]),
            @r#"
        Union(
            CommitRef(WorkingCopy(WorkspaceNameBuf("default"))),
            Union(
                CommitRef(Symbol("foo")),
                CommitRef(Symbol("bar")),
            ),
        )
        "#);
        insta::assert_debug_snapshot!(
            RevsetExpression::union_all(&[
                current_wc.clone(),
                foo_symbol.clone(),
                bar_symbol.clone(),
                baz_symbol.clone(),
            ]),
            @r#"
        Union(
            Union(
                CommitRef(WorkingCopy(WorkspaceNameBuf("default"))),
                CommitRef(Symbol("foo")),
            ),
            Union(
                CommitRef(Symbol("bar")),
                CommitRef(Symbol("baz")),
            ),
        )
        "#);
        insta::assert_debug_snapshot!(
            foo_symbol.intersection(&current_wc), @r#"
        Intersection(
            CommitRef(Symbol("foo")),
            CommitRef(WorkingCopy(WorkspaceNameBuf("default"))),
        )
        "#);
        insta::assert_debug_snapshot!(
            foo_symbol.minus(&current_wc), @r#"
        Difference(
            CommitRef(Symbol("foo")),
            CommitRef(WorkingCopy(WorkspaceNameBuf("default"))),
        )
        "#);
        insta::assert_debug_snapshot!(
            UserRevsetExpression::coalesce(&[]),
            @"None");
        insta::assert_debug_snapshot!(
            RevsetExpression::coalesce(&[current_wc.clone()]),
            @r#"CommitRef(WorkingCopy(WorkspaceNameBuf("default")))"#);
        insta::assert_debug_snapshot!(
            RevsetExpression::coalesce(&[current_wc.clone(), foo_symbol.clone()]),
            @r#"
        Coalesce(
            CommitRef(WorkingCopy(WorkspaceNameBuf("default"))),
            CommitRef(Symbol("foo")),
        )
        "#);
        insta::assert_debug_snapshot!(
            RevsetExpression::coalesce(&[
                current_wc.clone(),
                foo_symbol.clone(),
                bar_symbol.clone(),
            ]),
            @r#"
        Coalesce(
            CommitRef(WorkingCopy(WorkspaceNameBuf("default"))),
            Coalesce(
                CommitRef(Symbol("foo")),
                CommitRef(Symbol("bar")),
            ),
        )
        "#);
    }

    #[test]
    fn test_parse_revset() {
        let settings = insta_settings();
        let _guard = settings.bind_to_scope();
        let main_workspace_name = WorkspaceNameBuf::from("main");
        let other_workspace_name = WorkspaceNameBuf::from("other");

        // Parse "@" (the current working copy)
        insta::assert_debug_snapshot!(
            parse("@").unwrap_err().kind(),
            @"WorkingCopyWithoutWorkspace");
        insta::assert_debug_snapshot!(
            parse("main@").unwrap(),
            @r#"CommitRef(WorkingCopy(WorkspaceNameBuf("main")))"#);
        insta::assert_debug_snapshot!(
            parse_with_workspace("@", &main_workspace_name).unwrap(),
            @r#"CommitRef(WorkingCopy(WorkspaceNameBuf("main")))"#);
        insta::assert_debug_snapshot!(
            parse_with_workspace("main@", &other_workspace_name).unwrap(),
            @r#"CommitRef(WorkingCopy(WorkspaceNameBuf("main")))"#);
        // "@" in function argument must be quoted
        insta::assert_debug_snapshot!(
            parse("author_name(foo@)").unwrap_err().kind(),
            @r#"Expression("Invalid string expression")"#);
        insta::assert_debug_snapshot!(
            parse(r#"author_name("foo@")"#).unwrap(),
            @r#"Filter(AuthorName(Pattern(Substring("foo@"))))"#);
        // Parse a single symbol
        insta::assert_debug_snapshot!(
            parse("foo").unwrap(),
            @r#"CommitRef(Symbol("foo"))"#);
        // Default arguments for *bookmarks() are all ""
        insta::assert_debug_snapshot!(
            parse("bookmarks()").unwrap(),
            @r#"CommitRef(Bookmarks(Pattern(Substring(""))))"#);
        // Default argument for tags() is ""
        insta::assert_debug_snapshot!(
            parse("tags()").unwrap(),
            @r#"CommitRef(Tags(Pattern(Substring(""))))"#);
        insta::assert_debug_snapshot!(parse("remote_bookmarks()").unwrap(), @r#"
        CommitRef(
            RemoteBookmarks {
                bookmark: Pattern(Substring("")),
                remote: NotIn(Pattern(Exact("ignored"))),
                remote_ref_state: None,
            },
        )
        "#);
        insta::assert_debug_snapshot!(parse("tracked_remote_bookmarks()").unwrap(), @r#"
        CommitRef(
            RemoteBookmarks {
                bookmark: Pattern(Substring("")),
                remote: NotIn(Pattern(Exact("ignored"))),
                remote_ref_state: Some(Tracked),
            },
        )
        "#);
        insta::assert_debug_snapshot!(parse("untracked_remote_bookmarks()").unwrap(), @r#"
        CommitRef(
            RemoteBookmarks {
                bookmark: Pattern(Substring("")),
                remote: NotIn(Pattern(Exact("ignored"))),
                remote_ref_state: Some(New),
            },
        )
        "#);
        // Parse a quoted symbol
        insta::assert_debug_snapshot!(
            parse("'foo'").unwrap(),
            @r#"CommitRef(Symbol("foo"))"#);
        // Parse the "parents" operator
        insta::assert_debug_snapshot!(parse("foo-").unwrap(), @r#"
        Ancestors {
            heads: CommitRef(Symbol("foo")),
            generation: 1..2,
            parents_range: 0..4294967295,
        }
        "#);
        // Parse the "children" operator
        insta::assert_debug_snapshot!(parse("foo+").unwrap(), @r#"
        Descendants {
            roots: CommitRef(Symbol("foo")),
            generation: 1..2,
        }
        "#);
        // Parse the "ancestors" operator
        insta::assert_debug_snapshot!(parse("::foo").unwrap(), @r#"
        Ancestors {
            heads: CommitRef(Symbol("foo")),
            generation: 0..18446744073709551615,
            parents_range: 0..4294967295,
        }
        "#);
        // Parse the "descendants" operator
        insta::assert_debug_snapshot!(parse("foo::").unwrap(), @r#"
        Descendants {
            roots: CommitRef(Symbol("foo")),
            generation: 0..18446744073709551615,
        }
        "#);
        // Parse the "dag range" operator
        insta::assert_debug_snapshot!(parse("foo::bar").unwrap(), @r#"
        DagRange {
            roots: CommitRef(Symbol("foo")),
            heads: CommitRef(Symbol("bar")),
        }
        "#);
        // Parse the nullary "dag range" operator
        insta::assert_debug_snapshot!(parse("::").unwrap(), @"All");
        // Parse the "range" prefix operator
        insta::assert_debug_snapshot!(parse("..foo").unwrap(), @r#"
        Range {
            roots: Root,
            heads: CommitRef(Symbol("foo")),
            generation: 0..18446744073709551615,
            parents_range: 0..4294967295,
        }
        "#);
        insta::assert_debug_snapshot!(parse("foo..").unwrap(), @r#"
        NotIn(
            Ancestors {
                heads: CommitRef(Symbol("foo")),
                generation: 0..18446744073709551615,
                parents_range: 0..4294967295,
            },
        )
        "#);
        insta::assert_debug_snapshot!(parse("foo..bar").unwrap(), @r#"
        Range {
            roots: CommitRef(Symbol("foo")),
            heads: CommitRef(Symbol("bar")),
            generation: 0..18446744073709551615,
            parents_range: 0..4294967295,
        }
        "#);
        // Parse the nullary "range" operator
        insta::assert_debug_snapshot!(parse("..").unwrap(), @r#"NotIn(Root)"#);
        // Parse the "negate" operator
        insta::assert_debug_snapshot!(
            parse("~ foo").unwrap(),
            @r#"NotIn(CommitRef(Symbol("foo")))"#);
        // Parse the "intersection" operator
        insta::assert_debug_snapshot!(parse("foo & bar").unwrap(), @r#"
        Intersection(
            CommitRef(Symbol("foo")),
            CommitRef(Symbol("bar")),
        )
        "#);
        // Parse the "union" operator
        insta::assert_debug_snapshot!(parse("foo | bar").unwrap(), @r#"
        Union(
            CommitRef(Symbol("foo")),
            CommitRef(Symbol("bar")),
        )
        "#);
        // Parse the "difference" operator
        insta::assert_debug_snapshot!(parse("foo ~ bar").unwrap(), @r#"
        Difference(
            CommitRef(Symbol("foo")),
            CommitRef(Symbol("bar")),
        )
        "#);
    }

    #[test]
    fn test_parse_revset_with_modifier() {
        let settings = insta_settings();
        let _guard = settings.bind_to_scope();

        insta::assert_debug_snapshot!(
            parse_with_modifier("all:foo").unwrap(), @r#"
        (
            CommitRef(Symbol("foo")),
            Some(All),
        )
        "#);

        // Top-level string pattern can't be parsed, which is an error anyway
        insta::assert_debug_snapshot!(
            parse_with_modifier(r#"exact:"foo""#).unwrap_err().kind(),
            @r#"NoSuchModifier("exact")"#);
    }

    #[test]
    fn test_parse_string_pattern() {
        let settings = insta_settings();
        let _guard = settings.bind_to_scope();

        insta::assert_debug_snapshot!(
            parse(r#"bookmarks("foo")"#).unwrap(),
            @r#"CommitRef(Bookmarks(Pattern(Substring("foo"))))"#);
        insta::assert_debug_snapshot!(
            parse(r#"bookmarks(exact:"foo")"#).unwrap(),
            @r#"CommitRef(Bookmarks(Pattern(Exact("foo"))))"#);
        insta::assert_debug_snapshot!(
            parse(r#"bookmarks(substring:"foo")"#).unwrap(),
            @r#"CommitRef(Bookmarks(Pattern(Substring("foo"))))"#);
        insta::assert_debug_snapshot!(
            parse(r#"bookmarks(bad:"foo")"#).unwrap_err().kind(),
            @r#"Expression("Invalid string pattern")"#);
        insta::assert_debug_snapshot!(
            parse(r#"bookmarks(exact::"foo")"#).unwrap_err().kind(),
            @r#"Expression("Invalid string expression")"#);
        insta::assert_debug_snapshot!(
            parse(r#"bookmarks(exact:"foo"+)"#).unwrap_err().kind(),
            @r#"Expression("Invalid string expression")"#);

        insta::assert_debug_snapshot!(
            parse(r#"tags("foo")"#).unwrap(),
            @r#"CommitRef(Tags(Pattern(Substring("foo"))))"#);
        insta::assert_debug_snapshot!(
            parse(r#"tags(exact:"foo")"#).unwrap(),
            @r#"CommitRef(Tags(Pattern(Exact("foo"))))"#);
        insta::assert_debug_snapshot!(
            parse(r#"tags(substring:"foo")"#).unwrap(),
            @r#"CommitRef(Tags(Pattern(Substring("foo"))))"#);
        insta::assert_debug_snapshot!(
            parse(r#"tags(bad:"foo")"#).unwrap_err().kind(),
            @r#"Expression("Invalid string pattern")"#);
        insta::assert_debug_snapshot!(
            parse(r#"tags(exact::"foo")"#).unwrap_err().kind(),
            @r#"Expression("Invalid string expression")"#);
        insta::assert_debug_snapshot!(
            parse(r#"tags(exact:"foo"+)"#).unwrap_err().kind(),
            @r#"Expression("Invalid string expression")"#);

        // String pattern isn't allowed at top level.
        assert_matches!(
            parse(r#"(exact:"foo")"#).unwrap_err().kind(),
            RevsetParseErrorKind::NotInfixOperator { .. }
        );
    }

    #[test]
    fn test_parse_compound_string_expression() {
        let settings = insta_settings();
        let _guard = settings.bind_to_scope();

        insta::assert_debug_snapshot!(
            parse(r#"tags(~a)"#).unwrap(),
            @r#"
        CommitRef(
            Tags(NotIn(Pattern(Substring("a")))),
        )
        "#);
        insta::assert_debug_snapshot!(
            parse(r#"tags(a|b&c)"#).unwrap(),
            @r#"
        CommitRef(
            Tags(
                Union(
                    Pattern(Substring("a")),
                    Intersection(
                        Pattern(Substring("b")),
                        Pattern(Substring("c")),
                    ),
                ),
            ),
        )
        "#);
        insta::assert_debug_snapshot!(
            parse(r#"tags(a|b|c)"#).unwrap(),
            @r#"
        CommitRef(
            Tags(
                Union(
                    Pattern(Substring("a")),
                    Union(
                        Pattern(Substring("b")),
                        Pattern(Substring("c")),
                    ),
                ),
            ),
        )
        "#);
        insta::assert_debug_snapshot!(
            parse(r#"tags(a~(b|c))"#).unwrap(),
            @r#"
        CommitRef(
            Tags(
                Intersection(
                    Pattern(Substring("a")),
                    NotIn(
                        Union(
                            Pattern(Substring("b")),
                            Pattern(Substring("c")),
                        ),
                    ),
                ),
            ),
        )
        "#);
    }

    #[test]
    fn test_parse_revset_function() {
        let settings = insta_settings();
        let _guard = settings.bind_to_scope();

        insta::assert_debug_snapshot!(
            parse("parents(foo)").unwrap(), @r#"
        Ancestors {
            heads: CommitRef(Symbol("foo")),
            generation: 1..2,
            parents_range: 0..4294967295,
        }
        "#);
        insta::assert_debug_snapshot!(
            parse("parents(\"foo\")").unwrap(), @r#"
        Ancestors {
            heads: CommitRef(Symbol("foo")),
            generation: 1..2,
            parents_range: 0..4294967295,
        }
        "#);
        insta::assert_debug_snapshot!(
            parse("ancestors(parents(foo))").unwrap(), @r#"
        Ancestors {
            heads: Ancestors {
                heads: CommitRef(Symbol("foo")),
                generation: 1..2,
                parents_range: 0..4294967295,
            },
            generation: 0..18446744073709551615,
            parents_range: 0..4294967295,
        }
        "#);
        insta::assert_debug_snapshot!(
            parse("parents(foo, bar, baz)").unwrap_err().kind(), @r#"
        InvalidFunctionArguments {
            name: "parents",
            message: "Expected 1 to 2 arguments",
        }
        "#);
        insta::assert_debug_snapshot!(
            parse("parents(foo, 2)").unwrap(), @r#"
        Ancestors {
            heads: CommitRef(Symbol("foo")),
            generation: 2..3,
            parents_range: 0..4294967295,
        }
        "#);
        insta::assert_debug_snapshot!(
            parse("root()").unwrap(),
            @"Root");
        assert!(parse("root(a)").is_err());
        insta::assert_debug_snapshot!(
            parse(r#"description("")"#).unwrap(),
            @r#"Filter(Description(Pattern(Substring(""))))"#);
        insta::assert_debug_snapshot!(
            parse("description(foo)").unwrap(),
            @r#"Filter(Description(Pattern(Substring("foo"))))"#);
        insta::assert_debug_snapshot!(
            parse("description(visible_heads())").unwrap_err().kind(),
            @r#"Expression("Invalid string expression")"#);
        insta::assert_debug_snapshot!(
            parse("description(\"(foo)\")").unwrap(),
            @r#"Filter(Description(Pattern(Substring("(foo)"))))"#);
        assert!(parse("mine(foo)").is_err());
        insta::assert_debug_snapshot!(
            parse_with_workspace("empty()", WorkspaceName::DEFAULT).unwrap(),
            @"NotIn(Filter(File(All)))");
        assert!(parse_with_workspace("empty(foo)", WorkspaceName::DEFAULT).is_err());
        assert!(parse_with_workspace("file()", WorkspaceName::DEFAULT).is_err());
        insta::assert_debug_snapshot!(
            parse_with_workspace("files(foo)", WorkspaceName::DEFAULT).unwrap(),
            @r#"Filter(File(Pattern(PrefixPath("foo"))))"#);
        insta::assert_debug_snapshot!(
            parse_with_workspace("files(all())", WorkspaceName::DEFAULT).unwrap(),
            @"Filter(File(All))");
        insta::assert_debug_snapshot!(
            parse_with_workspace(r#"files(file:"foo")"#, WorkspaceName::DEFAULT).unwrap(),
            @r#"Filter(File(Pattern(FilePath("foo"))))"#);
        insta::assert_debug_snapshot!(
            parse_with_workspace("files(foo|bar&baz)", WorkspaceName::DEFAULT).unwrap(), @r#"
        Filter(
            File(
                UnionAll(
                    [
                        Pattern(PrefixPath("foo")),
                        Intersection(
                            Pattern(PrefixPath("bar")),
                            Pattern(PrefixPath("baz")),
                        ),
                    ],
                ),
            ),
        )
        "#);
        insta::assert_debug_snapshot!(
            parse_with_workspace(r#"files(~(foo))"#, WorkspaceName::DEFAULT).unwrap(),
            @r#"
        Filter(
            File(
                Difference(
                    All,
                    Pattern(PrefixPath("foo")),
                ),
            ),
        )
        "#);
        insta::assert_debug_snapshot!(parse("signed()").unwrap(), @"Filter(Signed)");
    }

    #[test]
    fn test_parse_revset_change_commit_id_functions() {
        let settings = insta_settings();
        let _guard = settings.bind_to_scope();

        insta::assert_debug_snapshot!(
            parse("change_id(z)").unwrap(),
            @r#"CommitRef(ChangeId(HexPrefix("0")))"#);
        insta::assert_debug_snapshot!(
            parse("change_id('zk')").unwrap(),
            @r#"CommitRef(ChangeId(HexPrefix("0f")))"#);
        insta::assert_debug_snapshot!(
            parse("change_id(01234)").unwrap_err().kind(),
            @r#"Expression("Invalid change ID prefix")"#);

        insta::assert_debug_snapshot!(
            parse("commit_id(0)").unwrap(),
            @r#"CommitRef(CommitId(HexPrefix("0")))"#);
        insta::assert_debug_snapshot!(
            parse("commit_id('0f')").unwrap(),
            @r#"CommitRef(CommitId(HexPrefix("0f")))"#);
        insta::assert_debug_snapshot!(
            parse("commit_id(xyzzy)").unwrap_err().kind(),
            @r#"Expression("Invalid commit ID prefix")"#);
    }

    #[test]
    fn test_parse_revset_author_committer_functions() {
        let settings = insta_settings();
        let _guard = settings.bind_to_scope();

        insta::assert_debug_snapshot!(
            parse("author(foo)").unwrap(), @r#"
        Union(
            Filter(AuthorName(Pattern(Substring("foo")))),
            Filter(AuthorEmail(Pattern(Substring("foo")))),
        )
        "#);
        insta::assert_debug_snapshot!(
            parse("author_name(foo)").unwrap(),
            @r#"Filter(AuthorName(Pattern(Substring("foo"))))"#);
        insta::assert_debug_snapshot!(
            parse("author_email(foo)").unwrap(),
            @r#"Filter(AuthorEmail(Pattern(Substring("foo"))))"#);

        insta::assert_debug_snapshot!(
            parse("committer(foo)").unwrap(), @r#"
        Union(
            Filter(CommitterName(Pattern(Substring("foo")))),
            Filter(CommitterEmail(Pattern(Substring("foo")))),
        )
        "#);
        insta::assert_debug_snapshot!(
            parse("committer_name(foo)").unwrap(),
            @r#"Filter(CommitterName(Pattern(Substring("foo"))))"#);
        insta::assert_debug_snapshot!(
            parse("committer_email(foo)").unwrap(),
            @r#"Filter(CommitterEmail(Pattern(Substring("foo"))))"#);

        insta::assert_debug_snapshot!(
            parse("mine()").unwrap(),
            @r#"Filter(AuthorEmail(Pattern(ExactI("test.user@example.com"))))"#);
    }

    #[test]
    fn test_parse_revset_keyword_arguments() {
        let settings = insta_settings();
        let _guard = settings.bind_to_scope();

        insta::assert_debug_snapshot!(
            parse("remote_bookmarks(remote=foo)").unwrap(), @r#"
        CommitRef(
            RemoteBookmarks {
                bookmark: Pattern(Substring("")),
                remote: Pattern(Substring("foo")),
                remote_ref_state: None,
            },
        )
        "#);
        insta::assert_debug_snapshot!(
            parse("remote_bookmarks(foo, remote=bar)").unwrap(), @r#"
        CommitRef(
            RemoteBookmarks {
                bookmark: Pattern(Substring("foo")),
                remote: Pattern(Substring("bar")),
                remote_ref_state: None,
            },
        )
        "#);
        insta::assert_debug_snapshot!(
            parse("tracked_remote_bookmarks(foo, remote=bar)").unwrap(), @r#"
        CommitRef(
            RemoteBookmarks {
                bookmark: Pattern(Substring("foo")),
                remote: Pattern(Substring("bar")),
                remote_ref_state: Some(Tracked),
            },
        )
        "#);
        insta::assert_debug_snapshot!(
            parse("untracked_remote_bookmarks(foo, remote=bar)").unwrap(), @r#"
        CommitRef(
            RemoteBookmarks {
                bookmark: Pattern(Substring("foo")),
                remote: Pattern(Substring("bar")),
                remote_ref_state: Some(New),
            },
        )
        "#);
        insta::assert_debug_snapshot!(
            parse(r#"remote_bookmarks(remote=foo, bar)"#).unwrap_err().kind(),
            @r#"
        InvalidFunctionArguments {
            name: "remote_bookmarks",
            message: "Positional argument follows keyword argument",
        }
        "#);
        insta::assert_debug_snapshot!(
            parse(r#"remote_bookmarks("", foo, remote=bar)"#).unwrap_err().kind(),
            @r#"
        InvalidFunctionArguments {
            name: "remote_bookmarks",
            message: "Got multiple values for keyword \"remote\"",
        }
        "#);
        insta::assert_debug_snapshot!(
            parse(r#"remote_bookmarks(remote=bar, remote=bar)"#).unwrap_err().kind(),
            @r#"
        InvalidFunctionArguments {
            name: "remote_bookmarks",
            message: "Got multiple values for keyword \"remote\"",
        }
        "#);
        insta::assert_debug_snapshot!(
            parse(r#"remote_bookmarks(unknown=bar)"#).unwrap_err().kind(),
            @r#"
        InvalidFunctionArguments {
            name: "remote_bookmarks",
            message: "Unexpected keyword argument \"unknown\"",
        }
        "#);
    }

    #[test]
    fn test_expand_symbol_alias() {
        let settings = insta_settings();
        let _guard = settings.bind_to_scope();

        insta::assert_debug_snapshot!(
            parse_with_aliases("AB|c", [("AB", "a|b")]).unwrap(), @r#"
        Union(
            Union(
                CommitRef(Symbol("a")),
                CommitRef(Symbol("b")),
            ),
            CommitRef(Symbol("c")),
        )
        "#);

        // Alias can be substituted to string literal.
        insta::assert_debug_snapshot!(
            parse_with_aliases_and_workspace("files(A)", [("A", "a")], WorkspaceName::DEFAULT)
                .unwrap(),
            @r#"Filter(File(Pattern(PrefixPath("a"))))"#);

        // Alias can be substituted to string pattern.
        insta::assert_debug_snapshot!(
            parse_with_aliases("author_name(A)", [("A", "a")]).unwrap(),
            @r#"Filter(AuthorName(Pattern(Substring("a"))))"#);
        // However, parentheses are required because top-level x:y is parsed as
        // program modifier.
        insta::assert_debug_snapshot!(
            parse_with_aliases("author_name(A)", [("A", "(exact:a)")]).unwrap(),
            @r#"Filter(AuthorName(Pattern(Exact("a"))))"#);

        // Sub-expression alias cannot be substituted to modifier expression.
        insta::assert_debug_snapshot!(
            parse_with_aliases_and_modifier("A-", [("A", "all:a")]).unwrap_err().kind(),
            @r#"InAliasExpansion("A")"#);
    }

    #[test]
    fn test_expand_function_alias() {
        let settings = insta_settings();
        let _guard = settings.bind_to_scope();

        // Pass string literal as parameter.
        insta::assert_debug_snapshot!(
            parse_with_aliases("F(a)", [("F(x)", "author_name(x)|committer_name(x)")]).unwrap(),
            @r#"
        Union(
            Filter(AuthorName(Pattern(Substring("a")))),
            Filter(CommitterName(Pattern(Substring("a")))),
        )
        "#);
    }

    #[test]
    fn test_transform_expression() {
        let settings = insta_settings();
        let _guard = settings.bind_to_scope();

        // Break without pre transformation
        insta::assert_debug_snapshot!(
            transform_expression(
                &ResolvedRevsetExpression::root(),
                |_| ControlFlow::Break(None),
                |_| Some(RevsetExpression::none()),
            ), @"None");

        // Break with pre transformation
        insta::assert_debug_snapshot!(
            transform_expression(
                &ResolvedRevsetExpression::root(),
                |_| ControlFlow::Break(Some(RevsetExpression::all())),
                |_| Some(RevsetExpression::none()),
            ), @"Some(All)");

        // Continue without pre transformation, do transform child
        insta::assert_debug_snapshot!(
            transform_expression(
                &ResolvedRevsetExpression::root().heads(),
                |_| ControlFlow::Continue(()),
                |x| match x.as_ref() {
                    RevsetExpression::Root => Some(RevsetExpression::none()),
                    _ => None,
                },
            ), @"Some(Heads(None))");

        // Continue without pre transformation, do transform self
        insta::assert_debug_snapshot!(
            transform_expression(
                &ResolvedRevsetExpression::root().heads(),
                |_| ControlFlow::Continue(()),
                |x| match x.as_ref() {
                    RevsetExpression::Heads(y) => Some(y.clone()),
                    _ => None,
                },
            ), @"Some(Root)");
    }

    #[test]
    fn test_resolve_referenced_commits() {
        let settings = insta_settings();
        let _guard = settings.bind_to_scope();

        let visibility1 = Arc::new(ResolvedRevsetExpression::WithinVisibility {
            candidates: RevsetExpression::commit(CommitId::from_hex("100001")),
            visible_heads: vec![CommitId::from_hex("100000")],
        });
        let visibility2 = Arc::new(ResolvedRevsetExpression::WithinVisibility {
            candidates: RevsetExpression::filter(RevsetFilterPredicate::HasConflict),
            visible_heads: vec![CommitId::from_hex("200000")],
        });
        let commit3 = RevsetExpression::commit(CommitId::from_hex("000003"));

        // Inner commits should be scoped. Both inner commits and visible heads
        // should be added to the outer scope.
        insta::assert_debug_snapshot!(
            resolve_referenced_commits(&visibility1.intersection(&RevsetExpression::all())), @r#"
        Some(
            WithinReference {
                candidates: Intersection(
                    WithinVisibility {
                        candidates: WithinReference {
                            candidates: Commits(
                                [
                                    CommitId("100001"),
                                ],
                            ),
                            commits: [
                                CommitId("100001"),
                            ],
                        },
                        visible_heads: [
                            CommitId("100000"),
                        ],
                    },
                    All,
                ),
                commits: [
                    CommitId("100000"),
                    CommitId("100001"),
                ],
            },
        )
        "#);

        // Inner scope has no references, so WithinReference should be omitted.
        insta::assert_debug_snapshot!(
            resolve_referenced_commits(
                &visibility2
                    .intersection(&RevsetExpression::all())
                    .union(&commit3),
            ), @r#"
        Some(
            WithinReference {
                candidates: Union(
                    Intersection(
                        WithinVisibility {
                            candidates: Filter(HasConflict),
                            visible_heads: [
                                CommitId("200000"),
                            ],
                        },
                        All,
                    ),
                    Commits(
                        [
                            CommitId("000003"),
                        ],
                    ),
                ),
                commits: [
                    CommitId("000003"),
                    CommitId("200000"),
                ],
            },
        )
        "#);

        // Sibling scopes should track referenced commits individually.
        insta::assert_debug_snapshot!(
            resolve_referenced_commits(
                &visibility1
                    .union(&visibility2)
                    .union(&commit3)
                    .intersection(&RevsetExpression::all())
            ), @r#"
        Some(
            WithinReference {
                candidates: Intersection(
                    Union(
                        Union(
                            WithinVisibility {
                                candidates: WithinReference {
                                    candidates: Commits(
                                        [
                                            CommitId("100001"),
                                        ],
                                    ),
                                    commits: [
                                        CommitId("100001"),
                                    ],
                                },
                                visible_heads: [
                                    CommitId("100000"),
                                ],
                            },
                            WithinVisibility {
                                candidates: Filter(HasConflict),
                                visible_heads: [
                                    CommitId("200000"),
                                ],
                            },
                        ),
                        Commits(
                            [
                                CommitId("000003"),
                            ],
                        ),
                    ),
                    All,
                ),
                commits: [
                    CommitId("000003"),
                    CommitId("100000"),
                    CommitId("100001"),
                    CommitId("200000"),
                ],
            },
        )
        "#);

        // Referenced commits should be propagated from the innermost scope.
        insta::assert_debug_snapshot!(
            resolve_referenced_commits(&Arc::new(ResolvedRevsetExpression::WithinVisibility {
                candidates: visibility1.clone(),
                visible_heads: vec![CommitId::from_hex("400000")],
            })), @r#"
        Some(
            WithinReference {
                candidates: WithinVisibility {
                    candidates: WithinReference {
                        candidates: WithinVisibility {
                            candidates: WithinReference {
                                candidates: Commits(
                                    [
                                        CommitId("100001"),
                                    ],
                                ),
                                commits: [
                                    CommitId("100001"),
                                ],
                            },
                            visible_heads: [
                                CommitId("100000"),
                            ],
                        },
                        commits: [
                            CommitId("100000"),
                            CommitId("100001"),
                        ],
                    },
                    visible_heads: [
                        CommitId("400000"),
                    ],
                },
                commits: [
                    CommitId("400000"),
                    CommitId("100000"),
                    CommitId("100001"),
                ],
            },
        )
        "#);

        // Resolved expression should be reused.
        let resolved = Arc::new(ResolvedRevsetExpression::WithinReference {
            // No referenced commits within the scope to test whether the
            // precomputed value is reused.
            candidates: RevsetExpression::none(),
            commits: vec![CommitId::from_hex("100000")],
        });
        insta::assert_debug_snapshot!(
            resolve_referenced_commits(&resolved), @"None");
        insta::assert_debug_snapshot!(
            resolve_referenced_commits(&resolved.intersection(&RevsetExpression::all())), @r#"
        Some(
            WithinReference {
                candidates: Intersection(
                    WithinReference {
                        candidates: None,
                        commits: [
                            CommitId("100000"),
                        ],
                    },
                    All,
                ),
                commits: [
                    CommitId("100000"),
                ],
            },
        )
        "#);
        insta::assert_debug_snapshot!(
            resolve_referenced_commits(&Arc::new(ResolvedRevsetExpression::WithinVisibility {
                candidates: resolved.clone(),
                visible_heads: vec![CommitId::from_hex("400000")],
            })), @r#"
        Some(
            WithinReference {
                candidates: WithinVisibility {
                    candidates: WithinReference {
                        candidates: None,
                        commits: [
                            CommitId("100000"),
                        ],
                    },
                    visible_heads: [
                        CommitId("400000"),
                    ],
                },
                commits: [
                    CommitId("400000"),
                    CommitId("100000"),
                ],
            },
        )
        "#);
    }

    #[test]
    fn test_optimize_subtree() {
        let settings = insta_settings();
        let _guard = settings.bind_to_scope();

        // Check that transform_expression_bottom_up() never rewrites enum variant
        // (e.g. Range -> DagRange) nor reorders arguments unintentionally.

        insta::assert_debug_snapshot!(
            optimize(parse("parents(bookmarks() & all())").unwrap()), @r#"
        Ancestors {
            heads: CommitRef(Bookmarks(Pattern(Substring("")))),
            generation: 1..2,
            parents_range: 0..4294967295,
        }
        "#);
        insta::assert_debug_snapshot!(
            optimize(parse("children(bookmarks() & all())").unwrap()), @r#"
        Descendants {
            roots: CommitRef(Bookmarks(Pattern(Substring("")))),
            generation: 1..2,
        }
        "#);
        insta::assert_debug_snapshot!(
            optimize(parse("ancestors(bookmarks() & all())").unwrap()), @r#"
        Ancestors {
            heads: CommitRef(Bookmarks(Pattern(Substring("")))),
            generation: 0..18446744073709551615,
            parents_range: 0..4294967295,
        }
        "#);
        insta::assert_debug_snapshot!(
            optimize(parse("descendants(bookmarks() & all())").unwrap()), @r#"
        Descendants {
            roots: CommitRef(Bookmarks(Pattern(Substring("")))),
            generation: 0..18446744073709551615,
        }
        "#);

        insta::assert_debug_snapshot!(
            optimize(parse("(bookmarks() & all())..(all() & tags())").unwrap()), @r#"
        Range {
            roots: CommitRef(Bookmarks(Pattern(Substring("")))),
            heads: CommitRef(Tags(Pattern(Substring("")))),
            generation: 0..18446744073709551615,
            parents_range: 0..4294967295,
        }
        "#);
        insta::assert_debug_snapshot!(
            optimize(parse("(bookmarks() & all())::(all() & tags())").unwrap()), @r#"
        DagRange {
            roots: CommitRef(Bookmarks(Pattern(Substring("")))),
            heads: CommitRef(Tags(Pattern(Substring("")))),
        }
        "#);

        insta::assert_debug_snapshot!(
            optimize(parse("heads(bookmarks() & all())").unwrap()),
            @r#"
        Heads(
            CommitRef(Bookmarks(Pattern(Substring("")))),
        )
        "#);
        insta::assert_debug_snapshot!(
            optimize(parse("roots(bookmarks() & all())").unwrap()),
            @r#"
        Roots(
            CommitRef(Bookmarks(Pattern(Substring("")))),
        )
        "#);

        insta::assert_debug_snapshot!(
            optimize(parse("latest(bookmarks() & all(), 2)").unwrap()), @r#"
        Latest {
            candidates: CommitRef(Bookmarks(Pattern(Substring("")))),
            count: 2,
        }
        "#);

        insta::assert_debug_snapshot!(
            optimize(parse("present(foo ~ bar)").unwrap()), @r#"
        Present(
            Difference(
                CommitRef(Symbol("foo")),
                CommitRef(Symbol("bar")),
            ),
        )
        "#);
        insta::assert_debug_snapshot!(
            optimize(parse("present(bookmarks() & all())").unwrap()),
            @r#"
        Present(
            CommitRef(Bookmarks(Pattern(Substring("")))),
        )
        "#);

        insta::assert_debug_snapshot!(
            optimize(parse("at_operation(@-, bookmarks() & all())").unwrap()), @r#"
        AtOperation {
            operation: "@-",
            candidates: CommitRef(Bookmarks(Pattern(Substring("")))),
        }
        "#);
        insta::assert_debug_snapshot!(
            optimize(Arc::new(RevsetExpression::WithinReference {
                candidates: parse("bookmarks() & all()").unwrap(),
                commits: vec![CommitId::from_hex("012345")],
            })), @r#"
        WithinReference {
            candidates: CommitRef(Bookmarks(Pattern(Substring("")))),
            commits: [
                CommitId("012345"),
            ],
        }
        "#);
        insta::assert_debug_snapshot!(
            optimize(Arc::new(RevsetExpression::WithinVisibility {
                candidates: parse("bookmarks() & all()").unwrap(),
                visible_heads: vec![CommitId::from_hex("012345")],
            })), @r#"
        WithinReference {
            candidates: WithinVisibility {
                candidates: CommitRef(Bookmarks(Pattern(Substring("")))),
                visible_heads: [
                    CommitId("012345"),
                ],
            },
            commits: [
                CommitId("012345"),
            ],
        }
        "#);

        insta::assert_debug_snapshot!(
            optimize(parse("~bookmarks() & all()").unwrap()),
            @r#"
        NotIn(
            CommitRef(Bookmarks(Pattern(Substring("")))),
        )
        "#);
        insta::assert_debug_snapshot!(
            optimize(parse("(bookmarks() & all()) | (all() & tags())").unwrap()), @r#"
        Union(
            CommitRef(Bookmarks(Pattern(Substring("")))),
            CommitRef(Tags(Pattern(Substring("")))),
        )
        "#);
        insta::assert_debug_snapshot!(
            optimize(parse("(bookmarks() & all()) & (all() & tags())").unwrap()), @r#"
        Intersection(
            CommitRef(Bookmarks(Pattern(Substring("")))),
            CommitRef(Tags(Pattern(Substring("")))),
        )
        "#);
        insta::assert_debug_snapshot!(
            optimize(parse("(bookmarks() & all()) ~ (all() & tags())").unwrap()), @r#"
        Difference(
            CommitRef(Bookmarks(Pattern(Substring("")))),
            CommitRef(Tags(Pattern(Substring("")))),
        )
        "#);
    }

    #[test]
    fn test_optimize_unchanged_subtree() {
        fn unwrap_union(
            expression: &UserRevsetExpression,
        ) -> (&Arc<UserRevsetExpression>, &Arc<UserRevsetExpression>) {
            match expression {
                RevsetExpression::Union(left, right) => (left, right),
                _ => panic!("unexpected expression: {expression:?}"),
            }
        }

        // transform_expression_bottom_up() should not recreate tree unnecessarily.
        let parsed = parse("foo-").unwrap();
        let optimized = optimize(parsed.clone());
        assert!(Arc::ptr_eq(&parsed, &optimized));

        let parsed = parse("bookmarks() | tags()").unwrap();
        let optimized = optimize(parsed.clone());
        assert!(Arc::ptr_eq(&parsed, &optimized));

        let parsed = parse("bookmarks() & tags()").unwrap();
        let optimized = optimize(parsed.clone());
        assert!(Arc::ptr_eq(&parsed, &optimized));

        // Only left subtree should be rewritten.
        let parsed = parse("(bookmarks() & all()) | tags()").unwrap();
        let optimized = optimize(parsed.clone());
        assert_matches!(
            unwrap_union(&optimized).0.as_ref(),
            RevsetExpression::CommitRef(RevsetCommitRef::Bookmarks(_))
        );
        assert!(Arc::ptr_eq(
            unwrap_union(&parsed).1,
            unwrap_union(&optimized).1
        ));

        // Only right subtree should be rewritten.
        let parsed = parse("bookmarks() | (all() & tags())").unwrap();
        let optimized = optimize(parsed.clone());
        assert!(Arc::ptr_eq(
            unwrap_union(&parsed).0,
            unwrap_union(&optimized).0
        ));
        assert_matches!(
            unwrap_union(&optimized).1.as_ref(),
            RevsetExpression::CommitRef(RevsetCommitRef::Tags(_))
        );
    }

    #[test]
    fn test_optimize_basic() {
        let settings = insta_settings();
        let _guard = settings.bind_to_scope();

        insta::assert_debug_snapshot!(optimize(parse("all() | none()").unwrap()), @"All");
        insta::assert_debug_snapshot!(optimize(parse("all() & none()").unwrap()), @"None");
        insta::assert_debug_snapshot!(optimize(parse("root() | all()").unwrap()), @"All");
        insta::assert_debug_snapshot!(optimize(parse("root() & all()").unwrap()), @"Root");
        insta::assert_debug_snapshot!(optimize(parse("none() | root()").unwrap()), @"Root");
        insta::assert_debug_snapshot!(optimize(parse("none() & root()").unwrap()), @"None");
        insta::assert_debug_snapshot!(optimize(parse("~none()").unwrap()), @"All");
        insta::assert_debug_snapshot!(optimize(parse("~~none()").unwrap()), @"None");
        insta::assert_debug_snapshot!(optimize(parse("~all()").unwrap()), @"None");
        insta::assert_debug_snapshot!(optimize(parse("~~all()").unwrap()), @"All");
        insta::assert_debug_snapshot!(optimize(parse("~~foo").unwrap()), @r#"CommitRef(Symbol("foo"))"#);
        insta::assert_debug_snapshot!(
            optimize(parse("(root() | none()) & (visible_heads() | ~~all())").unwrap()), @"Root");
        insta::assert_debug_snapshot!(
            optimize(UserRevsetExpression::commits(vec![])), @"None");
        insta::assert_debug_snapshot!(
            optimize(UserRevsetExpression::commits(vec![]).negated()), @"All");
    }

    #[test]
    fn test_optimize_difference() {
        let settings = insta_settings();
        let _guard = settings.bind_to_scope();

        insta::assert_debug_snapshot!(optimize(parse("foo & ~bar").unwrap()), @r#"
        Difference(
            CommitRef(Symbol("foo")),
            CommitRef(Symbol("bar")),
        )
        "#);
        insta::assert_debug_snapshot!(optimize(parse("~foo & bar").unwrap()), @r#"
        Difference(
            CommitRef(Symbol("bar")),
            CommitRef(Symbol("foo")),
        )
        "#);
        insta::assert_debug_snapshot!(optimize(parse("~foo & bar & ~baz").unwrap()), @r#"
        Difference(
            Difference(
                CommitRef(Symbol("bar")),
                CommitRef(Symbol("foo")),
            ),
            CommitRef(Symbol("baz")),
        )
        "#);
        insta::assert_debug_snapshot!(optimize(parse("(all() & ~foo) & bar").unwrap()), @r#"
        Difference(
            CommitRef(Symbol("bar")),
            CommitRef(Symbol("foo")),
        )
        "#);

        // Binary difference operation should go through the same optimization passes.
        insta::assert_debug_snapshot!(
            optimize(parse("all() ~ foo").unwrap()),
            @r#"NotIn(CommitRef(Symbol("foo")))"#);
        insta::assert_debug_snapshot!(optimize(parse("foo ~ bar").unwrap()), @r#"
        Difference(
            CommitRef(Symbol("foo")),
            CommitRef(Symbol("bar")),
        )
        "#);
        insta::assert_debug_snapshot!(optimize(parse("(all() ~ foo) & bar").unwrap()), @r#"
        Difference(
            CommitRef(Symbol("bar")),
            CommitRef(Symbol("foo")),
        )
        "#);

        // Range expression.
        insta::assert_debug_snapshot!(optimize(parse("::foo & ~::bar").unwrap()), @r#"
        Range {
            roots: CommitRef(Symbol("bar")),
            heads: CommitRef(Symbol("foo")),
            generation: 0..18446744073709551615,
            parents_range: 0..4294967295,
        }
        "#);
        insta::assert_debug_snapshot!(optimize(parse("~::foo & ::bar").unwrap()), @r#"
        Range {
            roots: CommitRef(Symbol("foo")),
            heads: CommitRef(Symbol("bar")),
            generation: 0..18446744073709551615,
            parents_range: 0..4294967295,
        }
        "#);
        insta::assert_debug_snapshot!(optimize(parse("foo..").unwrap()), @r#"
        Range {
            roots: CommitRef(Symbol("foo")),
            heads: VisibleHeadsOrReferenced,
            generation: 0..18446744073709551615,
            parents_range: 0..4294967295,
        }
        "#);
        insta::assert_debug_snapshot!(optimize(parse("foo..bar").unwrap()), @r#"
        Range {
            roots: CommitRef(Symbol("foo")),
            heads: CommitRef(Symbol("bar")),
            generation: 0..18446744073709551615,
            parents_range: 0..4294967295,
        }
        "#);
        insta::assert_debug_snapshot!(optimize(parse("foo.. & ::bar").unwrap()), @r#"
        Range {
            roots: CommitRef(Symbol("foo")),
            heads: CommitRef(Symbol("bar")),
            generation: 0..18446744073709551615,
            parents_range: 0..4294967295,
        }
        "#);
        insta::assert_debug_snapshot!(optimize(parse("foo.. & first_ancestors(bar)").unwrap()), @r#"
        Range {
            roots: CommitRef(Symbol("foo")),
            heads: CommitRef(Symbol("bar")),
            generation: 0..18446744073709551615,
            parents_range: 0..1,
        }
        "#);

        // Double/triple negates.
        insta::assert_debug_snapshot!(optimize(parse("foo & ~~bar").unwrap()), @r#"
        Intersection(
            CommitRef(Symbol("foo")),
            CommitRef(Symbol("bar")),
        )
        "#);
        insta::assert_debug_snapshot!(optimize(parse("foo & ~~~bar").unwrap()), @r#"
        Difference(
            CommitRef(Symbol("foo")),
            CommitRef(Symbol("bar")),
        )
        "#);
        insta::assert_debug_snapshot!(optimize(parse("~(all() & ~foo) & bar").unwrap()), @r#"
        Intersection(
            CommitRef(Symbol("foo")),
            CommitRef(Symbol("bar")),
        )
        "#);

        // Should be better than '(all() & ~foo) & (all() & ~bar)'.
        insta::assert_debug_snapshot!(optimize(parse("~foo & ~bar").unwrap()), @r#"
        Difference(
            NotIn(CommitRef(Symbol("foo"))),
            CommitRef(Symbol("bar")),
        )
        "#);

        // The roots of multiple ranges can be folded after being unfolded.
        insta::assert_debug_snapshot!(optimize(parse("a..b & c..d").unwrap()), @r#"
        Intersection(
            Range {
                roots: Union(
                    CommitRef(Symbol("a")),
                    CommitRef(Symbol("c")),
                ),
                heads: CommitRef(Symbol("b")),
                generation: 0..18446744073709551615,
                parents_range: 0..4294967295,
            },
            Ancestors {
                heads: CommitRef(Symbol("d")),
                generation: 0..18446744073709551615,
                parents_range: 0..4294967295,
            },
        )
        "#);

        // Negated `first_ancestors()` doesn't prevent re-folding.
        insta::assert_debug_snapshot!(optimize(parse("foo..bar ~ first_ancestors(baz)").unwrap()), @r#"
        Difference(
            Range {
                roots: CommitRef(Symbol("foo")),
                heads: CommitRef(Symbol("bar")),
                generation: 0..18446744073709551615,
                parents_range: 0..4294967295,
            },
            Ancestors {
                heads: CommitRef(Symbol("baz")),
                generation: 0..18446744073709551615,
                parents_range: 0..1,
            },
        )
        "#);

        // Negated ancestors can be combined into a range regardless of intersection
        // grouping order and intervening expressions.
        insta::assert_debug_snapshot!(optimize(parse("foo ~ ::a & (::b & bar & ::c) & (baz ~ ::d)").unwrap()), @r#"
        Intersection(
            Intersection(
                Intersection(
                    Intersection(
                        Range {
                            roots: Union(
                                CommitRef(Symbol("a")),
                                CommitRef(Symbol("d")),
                            ),
                            heads: CommitRef(Symbol("b")),
                            generation: 0..18446744073709551615,
                            parents_range: 0..4294967295,
                        },
                        Ancestors {
                            heads: CommitRef(Symbol("c")),
                            generation: 0..18446744073709551615,
                            parents_range: 0..4294967295,
                        },
                    ),
                    CommitRef(Symbol("foo")),
                ),
                CommitRef(Symbol("bar")),
            ),
            CommitRef(Symbol("baz")),
        )
        "#);
    }

    #[test]
    fn test_optimize_not_in_ancestors() {
        let settings = insta_settings();
        let _guard = settings.bind_to_scope();

        // '~(::foo)' is equivalent to 'foo..'.
        insta::assert_debug_snapshot!(optimize(parse("~(::foo)").unwrap()), @r#"
        Range {
            roots: CommitRef(Symbol("foo")),
            heads: VisibleHeadsOrReferenced,
            generation: 0..18446744073709551615,
            parents_range: 0..4294967295,
        }
        "#);

        // '~(::foo-)' is equivalent to 'foo-..'.
        insta::assert_debug_snapshot!(optimize(parse("~(::foo-)").unwrap()), @r#"
        Range {
            roots: Ancestors {
                heads: CommitRef(Symbol("foo")),
                generation: 1..2,
                parents_range: 0..4294967295,
            },
            heads: VisibleHeadsOrReferenced,
            generation: 0..18446744073709551615,
            parents_range: 0..4294967295,
        }
        "#);
        insta::assert_debug_snapshot!(optimize(parse("~(::foo--)").unwrap()), @r#"
        Range {
            roots: Ancestors {
                heads: CommitRef(Symbol("foo")),
                generation: 2..3,
                parents_range: 0..4294967295,
            },
            heads: VisibleHeadsOrReferenced,
            generation: 0..18446744073709551615,
            parents_range: 0..4294967295,
        }
        "#);

        // Bounded ancestors shouldn't be substituted.
        insta::assert_debug_snapshot!(optimize(parse("~ancestors(foo, 1)").unwrap()), @r#"
        NotIn(
            Ancestors {
                heads: CommitRef(Symbol("foo")),
                generation: 0..1,
                parents_range: 0..4294967295,
            },
        )
        "#);
        insta::assert_debug_snapshot!(optimize(parse("~ancestors(foo-, 1)").unwrap()), @r#"
        NotIn(
            Ancestors {
                heads: CommitRef(Symbol("foo")),
                generation: 1..2,
                parents_range: 0..4294967295,
            },
        )
        "#);
    }

    #[test]
    fn test_optimize_filter_difference() {
        let settings = insta_settings();
        let _guard = settings.bind_to_scope();

        // '~empty()' -> '~~file(*)' -> 'file(*)'
        insta::assert_debug_snapshot!(optimize(parse("~empty()").unwrap()), @"Filter(File(All))");

        // '& baz' can be moved into the filter node, and form a difference node.
        insta::assert_debug_snapshot!(
            optimize(parse("(author_name(foo) & ~bar) & baz").unwrap()), @r#"
        Intersection(
            Difference(
                CommitRef(Symbol("baz")),
                CommitRef(Symbol("bar")),
            ),
            Filter(AuthorName(Pattern(Substring("foo")))),
        )
        "#);

        // '~set & filter()' shouldn't be substituted.
        insta::assert_debug_snapshot!(
            optimize(parse("~foo & author_name(bar)").unwrap()), @r#"
        Intersection(
            NotIn(CommitRef(Symbol("foo"))),
            Filter(AuthorName(Pattern(Substring("bar")))),
        )
        "#);
        insta::assert_debug_snapshot!(
            optimize(parse("~foo & (author_name(bar) | baz)").unwrap()), @r#"
        Intersection(
            NotIn(CommitRef(Symbol("foo"))),
            AsFilter(
                Union(
                    Filter(AuthorName(Pattern(Substring("bar")))),
                    CommitRef(Symbol("baz")),
                ),
            ),
        )
        "#);

        // Filter should be moved right of the intersection.
        insta::assert_debug_snapshot!(
            optimize(parse("author_name(foo) ~ bar").unwrap()), @r#"
        Intersection(
            NotIn(CommitRef(Symbol("bar"))),
            Filter(AuthorName(Pattern(Substring("foo")))),
        )
        "#);
    }

    #[test]
    fn test_optimize_filter_intersection() {
        let settings = insta_settings();
        let _guard = settings.bind_to_scope();

        insta::assert_debug_snapshot!(
            optimize(parse("author_name(foo)").unwrap()),
            @r#"Filter(AuthorName(Pattern(Substring("foo"))))"#);

        insta::assert_debug_snapshot!(optimize(parse("foo & description(bar)").unwrap()), @r#"
        Intersection(
            CommitRef(Symbol("foo")),
            Filter(Description(Pattern(Substring("bar")))),
        )
        "#);
        insta::assert_debug_snapshot!(optimize(parse("author_name(foo) & bar").unwrap()), @r#"
        Intersection(
            CommitRef(Symbol("bar")),
            Filter(AuthorName(Pattern(Substring("foo")))),
        )
        "#);
        insta::assert_debug_snapshot!(
            optimize(parse("author_name(foo) & committer_name(bar)").unwrap()), @r#"
        AsFilter(
            Intersection(
                Filter(AuthorName(Pattern(Substring("foo")))),
                Filter(CommitterName(Pattern(Substring("bar")))),
            ),
        )
        "#);

        insta::assert_debug_snapshot!(
            optimize(parse("foo & description(bar) & author_name(baz)").unwrap()), @r#"
        Intersection(
            CommitRef(Symbol("foo")),
            AsFilter(
                Intersection(
                    Filter(Description(Pattern(Substring("bar")))),
                    Filter(AuthorName(Pattern(Substring("baz")))),
                ),
            ),
        )
        "#);
        insta::assert_debug_snapshot!(
            optimize(parse("committer_name(foo) & bar & author_name(baz)").unwrap()), @r#"
        Intersection(
            CommitRef(Symbol("bar")),
            AsFilter(
                Intersection(
                    Filter(CommitterName(Pattern(Substring("foo")))),
                    Filter(AuthorName(Pattern(Substring("baz")))),
                ),
            ),
        )
        "#);
        insta::assert_debug_snapshot!(
            optimize(parse_with_workspace(
                "committer_name(foo) & files(bar) & baz",
                WorkspaceName::DEFAULT).unwrap(),
            ), @r#"
        Intersection(
            CommitRef(Symbol("baz")),
            AsFilter(
                Intersection(
                    Filter(CommitterName(Pattern(Substring("foo")))),
                    Filter(File(Pattern(PrefixPath("bar")))),
                ),
            ),
        )
        "#);
        insta::assert_debug_snapshot!(
            optimize(parse_with_workspace(
                "committer_name(foo) & files(bar) & author_name(baz)",
                WorkspaceName::DEFAULT).unwrap(),
            ), @r#"
        AsFilter(
            Intersection(
                Intersection(
                    Filter(CommitterName(Pattern(Substring("foo")))),
                    Filter(File(Pattern(PrefixPath("bar")))),
                ),
                Filter(AuthorName(Pattern(Substring("baz")))),
            ),
        )
        "#);
        insta::assert_debug_snapshot!(
            optimize(parse_with_workspace(
                "foo & files(bar) & baz",
                WorkspaceName::DEFAULT).unwrap(),
            ), @r#"
        Intersection(
            Intersection(
                CommitRef(Symbol("foo")),
                CommitRef(Symbol("baz")),
            ),
            Filter(File(Pattern(PrefixPath("bar")))),
        )
        "#);

        insta::assert_debug_snapshot!(
            optimize(parse("foo & description(bar) & author_name(baz) & qux").unwrap()), @r#"
        Intersection(
            Intersection(
                CommitRef(Symbol("foo")),
                CommitRef(Symbol("qux")),
            ),
            AsFilter(
                Intersection(
                    Filter(Description(Pattern(Substring("bar")))),
                    Filter(AuthorName(Pattern(Substring("baz")))),
                ),
            ),
        )
        "#);
        insta::assert_debug_snapshot!(
            optimize(parse("foo & description(bar) & parents(author_name(baz)) & qux").unwrap()),
            @r#"
        Intersection(
            Intersection(
                Intersection(
                    CommitRef(Symbol("foo")),
                    Ancestors {
                        heads: Filter(AuthorName(Pattern(Substring("baz")))),
                        generation: 1..2,
                        parents_range: 0..4294967295,
                    },
                ),
                CommitRef(Symbol("qux")),
            ),
            Filter(Description(Pattern(Substring("bar")))),
        )
        "#);
        insta::assert_debug_snapshot!(
            optimize(parse("foo & description(bar) & parents(author_name(baz) & qux)").unwrap()),
            @r#"
        Intersection(
            Intersection(
                CommitRef(Symbol("foo")),
                Ancestors {
                    heads: Intersection(
                        CommitRef(Symbol("qux")),
                        Filter(AuthorName(Pattern(Substring("baz")))),
                    ),
                    generation: 1..2,
                    parents_range: 0..4294967295,
                },
            ),
            Filter(Description(Pattern(Substring("bar")))),
        )
        "#);

        // Symbols have to be pushed down to the innermost filter node.
        insta::assert_debug_snapshot!(
            optimize(parse("(a & author_name(A)) & (b & author_name(B)) & (c & author_name(C))").unwrap()),
            @r#"
        Intersection(
            Intersection(
                Intersection(
                    CommitRef(Symbol("a")),
                    CommitRef(Symbol("b")),
                ),
                CommitRef(Symbol("c")),
            ),
            AsFilter(
                Intersection(
                    Intersection(
                        Filter(AuthorName(Pattern(Substring("A")))),
                        Filter(AuthorName(Pattern(Substring("B")))),
                    ),
                    Filter(AuthorName(Pattern(Substring("C")))),
                ),
            ),
        )
        "#);
        insta::assert_debug_snapshot!(
            optimize(parse("(a & author_name(A)) & ((b & author_name(B)) & (c & author_name(C))) & d").unwrap()),
            @r#"
        Intersection(
            Intersection(
                Intersection(
                    Intersection(
                        CommitRef(Symbol("a")),
                        CommitRef(Symbol("b")),
                    ),
                    CommitRef(Symbol("c")),
                ),
                CommitRef(Symbol("d")),
            ),
            AsFilter(
                Intersection(
                    Intersection(
                        Filter(AuthorName(Pattern(Substring("A")))),
                        Filter(AuthorName(Pattern(Substring("B")))),
                    ),
                    Filter(AuthorName(Pattern(Substring("C")))),
                ),
            ),
        )
        "#);

        // 'all()' moves in to 'filter()' first, so 'A & filter()' can be found.
        insta::assert_debug_snapshot!(
            optimize(parse("foo & (all() & description(bar)) & (author_name(baz) & all())").unwrap()),
            @r#"
        Intersection(
            CommitRef(Symbol("foo")),
            AsFilter(
                Intersection(
                    Filter(Description(Pattern(Substring("bar")))),
                    Filter(AuthorName(Pattern(Substring("baz")))),
                ),
            ),
        )
        "#);

        // Filter node shouldn't move across at_operation() boundary.
        insta::assert_debug_snapshot!(
            optimize(parse("author_name(foo) & bar & at_operation(@-, committer_name(baz))").unwrap()),
            @r#"
        Intersection(
            Intersection(
                CommitRef(Symbol("bar")),
                AtOperation {
                    operation: "@-",
                    candidates: Filter(CommitterName(Pattern(Substring("baz")))),
                },
            ),
            Filter(AuthorName(Pattern(Substring("foo")))),
        )
        "#);
    }

    #[test]
    fn test_optimize_filter_subtree() {
        let settings = insta_settings();
        let _guard = settings.bind_to_scope();

        insta::assert_debug_snapshot!(
            optimize(parse("(author_name(foo) | bar) & baz").unwrap()), @r#"
        Intersection(
            CommitRef(Symbol("baz")),
            AsFilter(
                Union(
                    Filter(AuthorName(Pattern(Substring("foo")))),
                    CommitRef(Symbol("bar")),
                ),
            ),
        )
        "#);

        // 'merges() & foo' can be evaluated independently
        insta::assert_debug_snapshot!(
            optimize(parse("merges() & foo | bar").unwrap()), @r#"
        Union(
            Intersection(
                CommitRef(Symbol("foo")),
                Filter(ParentCount(2..4294967295)),
            ),
            CommitRef(Symbol("bar")),
        )
        "#);

        // 'merges() & foo' can be evaluated independently, but 'conflicts()'
        // can't. We'll need implicit 'all() & _' anyway.
        insta::assert_debug_snapshot!(
            optimize(parse("merges() & foo | conflicts()").unwrap()), @r#"
        AsFilter(
            Union(
                Intersection(
                    CommitRef(Symbol("foo")),
                    Filter(ParentCount(2..4294967295)),
                ),
                Filter(HasConflict),
            ),
        )
        "#);

        // Nested filter intersection with union
        insta::assert_debug_snapshot!(
            optimize(parse("foo | conflicts() & merges() & signed()").unwrap()), @r#"
        AsFilter(
            Union(
                CommitRef(Symbol("foo")),
                Intersection(
                    Intersection(
                        Filter(HasConflict),
                        Filter(ParentCount(2..4294967295)),
                    ),
                    Filter(Signed),
                ),
            ),
        )
        "#);

        insta::assert_debug_snapshot!(
            optimize(parse("(foo | committer_name(bar)) & description(baz) & qux").unwrap()), @r#"
        Intersection(
            CommitRef(Symbol("qux")),
            AsFilter(
                Intersection(
                    Union(
                        CommitRef(Symbol("foo")),
                        Filter(CommitterName(Pattern(Substring("bar")))),
                    ),
                    Filter(Description(Pattern(Substring("baz")))),
                ),
            ),
        )
        "#);

        insta::assert_debug_snapshot!(
            optimize(parse(
                "(~present(author_name(foo) & description(bar)) | baz) & qux").unwrap()), @r#"
        Intersection(
            CommitRef(Symbol("qux")),
            AsFilter(
                Union(
                    NotIn(
                        Present(
                            Intersection(
                                Filter(AuthorName(Pattern(Substring("foo")))),
                                Filter(Description(Pattern(Substring("bar")))),
                            ),
                        ),
                    ),
                    CommitRef(Symbol("baz")),
                ),
            ),
        )
        "#);

        // Symbols have to be pushed down to the innermost filter node.
        insta::assert_debug_snapshot!(
            optimize(parse(
                "(a & (author_name(A) | 0)) & (b & (author_name(B) | 1)) & (c & (author_name(C) | 2))").unwrap()),
            @r#"
        Intersection(
            Intersection(
                Intersection(
                    CommitRef(Symbol("a")),
                    CommitRef(Symbol("b")),
                ),
                CommitRef(Symbol("c")),
            ),
            AsFilter(
                Intersection(
                    Intersection(
                        Union(
                            Filter(AuthorName(Pattern(Substring("A")))),
                            CommitRef(Symbol("0")),
                        ),
                        Union(
                            Filter(AuthorName(Pattern(Substring("B")))),
                            CommitRef(Symbol("1")),
                        ),
                    ),
                    Union(
                        Filter(AuthorName(Pattern(Substring("C")))),
                        CommitRef(Symbol("2")),
                    ),
                ),
            ),
        )
        "#);

        // Filters can be merged after ancestor unions are folded.
        insta::assert_debug_snapshot!(optimize(parse("::foo | ::author_name(bar)").unwrap()), @r#"
        Ancestors {
            heads: HeadsRange {
                roots: None,
                heads: VisibleHeadsOrReferenced,
                parents_range: 0..4294967295,
                filter: AsFilter(
                    Union(
                        CommitRef(Symbol("foo")),
                        Filter(AuthorName(Pattern(Substring("bar")))),
                    ),
                ),
            },
            generation: 0..18446744073709551615,
            parents_range: 0..4294967295,
        }
        "#);
    }

    #[test]
    fn test_optimize_ancestors() {
        let settings = insta_settings();
        let _guard = settings.bind_to_scope();

        // Typical scenario: fold nested parents()
        insta::assert_debug_snapshot!(optimize(parse("foo--").unwrap()), @r#"
        Ancestors {
            heads: CommitRef(Symbol("foo")),
            generation: 2..3,
            parents_range: 0..4294967295,
        }
        "#);
        insta::assert_debug_snapshot!(optimize(parse("::(foo---)").unwrap()), @r#"
        Ancestors {
            heads: CommitRef(Symbol("foo")),
            generation: 3..18446744073709551615,
            parents_range: 0..4294967295,
        }
        "#);
        insta::assert_debug_snapshot!(optimize(parse("(::foo)---").unwrap()), @r#"
        Ancestors {
            heads: CommitRef(Symbol("foo")),
            generation: 3..18446744073709551615,
            parents_range: 0..4294967295,
        }
        "#);

        // 'foo-+' is not 'foo'.
        insta::assert_debug_snapshot!(optimize(parse("foo---+").unwrap()), @r#"
        Descendants {
            roots: Ancestors {
                heads: CommitRef(Symbol("foo")),
                generation: 3..4,
                parents_range: 0..4294967295,
            },
            generation: 1..2,
        }
        "#);

        // For 'roots..heads', heads can be folded.
        insta::assert_debug_snapshot!(optimize(parse("foo..(bar--)").unwrap()), @r#"
        Range {
            roots: CommitRef(Symbol("foo")),
            heads: CommitRef(Symbol("bar")),
            generation: 2..18446744073709551615,
            parents_range: 0..4294967295,
        }
        "#);
        // roots can also be folded, and the range expression is reconstructed.
        insta::assert_debug_snapshot!(optimize(parse("(foo--)..(bar---)").unwrap()), @r#"
        Range {
            roots: Ancestors {
                heads: CommitRef(Symbol("foo")),
                generation: 2..3,
                parents_range: 0..4294967295,
            },
            heads: CommitRef(Symbol("bar")),
            generation: 3..18446744073709551615,
            parents_range: 0..4294967295,
        }
        "#);
        // Bounded ancestors shouldn't be substituted to range.
        insta::assert_debug_snapshot!(
            optimize(parse("~ancestors(foo, 2) & ::bar").unwrap()), @r#"
        Difference(
            Ancestors {
                heads: CommitRef(Symbol("bar")),
                generation: 0..18446744073709551615,
                parents_range: 0..4294967295,
            },
            Ancestors {
                heads: CommitRef(Symbol("foo")),
                generation: 0..2,
                parents_range: 0..4294967295,
            },
        )
        "#);

        // If inner range is bounded by roots, it cannot be merged.
        // e.g. '..(foo..foo)' is equivalent to '..none()', not to '..foo'
        insta::assert_debug_snapshot!(optimize(parse("(foo..bar)--").unwrap()), @r#"
        Ancestors {
            heads: Range {
                roots: CommitRef(Symbol("foo")),
                heads: CommitRef(Symbol("bar")),
                generation: 0..18446744073709551615,
                parents_range: 0..4294967295,
            },
            generation: 2..3,
            parents_range: 0..4294967295,
        }
        "#);
        insta::assert_debug_snapshot!(optimize(parse("foo..(bar..baz)").unwrap()), @r#"
        Range {
            roots: CommitRef(Symbol("foo")),
            heads: HeadsRange {
                roots: CommitRef(Symbol("bar")),
                heads: CommitRef(Symbol("baz")),
                parents_range: 0..4294967295,
                filter: All,
            },
            generation: 0..18446744073709551615,
            parents_range: 0..4294967295,
        }
        "#);

        // Ancestors of empty generation range should be empty.
        insta::assert_debug_snapshot!(
            optimize(parse("ancestors(ancestors(foo), 0)").unwrap()), @r#"
        Ancestors {
            heads: CommitRef(Symbol("foo")),
            generation: 0..0,
            parents_range: 0..4294967295,
        }
        "#
        );
        insta::assert_debug_snapshot!(
            optimize(parse("ancestors(ancestors(foo, 0))").unwrap()), @r#"
        Ancestors {
            heads: CommitRef(Symbol("foo")),
            generation: 0..0,
            parents_range: 0..4294967295,
        }
        "#
        );

        // Ancestors can only be folded if parent ranges match.
        insta::assert_debug_snapshot!(
            optimize(parse("first_ancestors(first_ancestors(foo, 5), 5)").unwrap()), @r#"
        Ancestors {
            heads: CommitRef(Symbol("foo")),
            generation: 0..9,
            parents_range: 0..1,
        }
        "#
        );
        insta::assert_debug_snapshot!(
            optimize(parse("first_ancestors(first_parent(foo), 5)").unwrap()), @r#"
        Ancestors {
            heads: CommitRef(Symbol("foo")),
            generation: 1..6,
            parents_range: 0..1,
        }
        "#
        );
        insta::assert_debug_snapshot!(
            optimize(parse("first_ancestors(ancestors(foo, 5), 5)").unwrap()), @r#"
        Ancestors {
            heads: Ancestors {
                heads: CommitRef(Symbol("foo")),
                generation: 0..5,
                parents_range: 0..4294967295,
            },
            generation: 0..5,
            parents_range: 0..1,
        }
        "#
        );
        insta::assert_debug_snapshot!(
            optimize(parse("ancestors(first_ancestors(foo, 5), 5)").unwrap()), @r#"
        Ancestors {
            heads: Ancestors {
                heads: CommitRef(Symbol("foo")),
                generation: 0..5,
                parents_range: 0..1,
            },
            generation: 0..5,
            parents_range: 0..4294967295,
        }
        "#
        );
    }

    #[test]
    fn test_optimize_descendants() {
        let settings = insta_settings();
        let _guard = settings.bind_to_scope();

        // Typical scenario: fold nested children()
        insta::assert_debug_snapshot!(optimize(parse("foo++").unwrap()), @r#"
        Descendants {
            roots: CommitRef(Symbol("foo")),
            generation: 2..3,
        }
        "#);
        insta::assert_debug_snapshot!(optimize(parse("(foo+++)::").unwrap()), @r#"
        Descendants {
            roots: CommitRef(Symbol("foo")),
            generation: 3..18446744073709551615,
        }
        "#);
        insta::assert_debug_snapshot!(optimize(parse("(foo::)+++").unwrap()), @r#"
        Descendants {
            roots: CommitRef(Symbol("foo")),
            generation: 3..18446744073709551615,
        }
        "#);

        // 'foo+-' is not 'foo'.
        insta::assert_debug_snapshot!(optimize(parse("foo+++-").unwrap()), @r#"
        Ancestors {
            heads: Descendants {
                roots: CommitRef(Symbol("foo")),
                generation: 3..4,
            },
            generation: 1..2,
            parents_range: 0..4294967295,
        }
        "#);

        // TODO: Inner Descendants can be folded into DagRange. Perhaps, we can rewrite
        // 'x::y' to 'x:: & ::y' first, so the common substitution rule can handle both
        // 'x+::y' and 'x+ & ::y'.
        insta::assert_debug_snapshot!(optimize(parse("(foo++)::bar").unwrap()), @r#"
        DagRange {
            roots: Descendants {
                roots: CommitRef(Symbol("foo")),
                generation: 2..3,
            },
            heads: CommitRef(Symbol("bar")),
        }
        "#);
    }

    #[test]
    fn test_optimize_flatten_intersection() {
        let settings = insta_settings();
        let _guard = settings.bind_to_scope();

        // Nested intersections should be flattened.
        insta::assert_debug_snapshot!(optimize(parse("a & ((b & c) & (d & e))").unwrap()), @r#"
        Intersection(
            Intersection(
                Intersection(
                    Intersection(
                        CommitRef(Symbol("a")),
                        CommitRef(Symbol("b")),
                    ),
                    CommitRef(Symbol("c")),
                ),
                CommitRef(Symbol("d")),
            ),
            CommitRef(Symbol("e")),
        )
        "#);
    }

    #[test]
    fn test_optimize_ancestors_union() {
        let settings = insta_settings();
        let _guard = settings.bind_to_scope();

        // Ancestors should be folded in unions.
        insta::assert_debug_snapshot!(optimize(parse("::a | ::b | ::c | ::d").unwrap()), @r#"
        Ancestors {
            heads: Union(
                Union(
                    CommitRef(Symbol("a")),
                    CommitRef(Symbol("b")),
                ),
                Union(
                    CommitRef(Symbol("c")),
                    CommitRef(Symbol("d")),
                ),
            ),
            generation: 0..18446744073709551615,
            parents_range: 0..4294967295,
        }
        "#);
        insta::assert_debug_snapshot!(optimize(parse("ancestors(a-) | ancestors(b)").unwrap()), @r#"
        Ancestors {
            heads: Union(
                Ancestors {
                    heads: CommitRef(Symbol("a")),
                    generation: 1..2,
                    parents_range: 0..4294967295,
                },
                CommitRef(Symbol("b")),
            ),
            generation: 0..18446744073709551615,
            parents_range: 0..4294967295,
        }
        "#);

        // Negated ancestors should be folded.
        insta::assert_debug_snapshot!(optimize(parse("~::a- & ~::b & ~::c & ::d").unwrap()), @r#"
        Range {
            roots: Union(
                Union(
                    Ancestors {
                        heads: CommitRef(Symbol("a")),
                        generation: 1..2,
                        parents_range: 0..4294967295,
                    },
                    CommitRef(Symbol("b")),
                ),
                CommitRef(Symbol("c")),
            ),
            heads: CommitRef(Symbol("d")),
            generation: 0..18446744073709551615,
            parents_range: 0..4294967295,
        }
        "#);
        insta::assert_debug_snapshot!(optimize(parse("a..b ~ ::c- ~ ::d").unwrap()), @r#"
        Range {
            roots: Union(
                Union(
                    CommitRef(Symbol("a")),
                    Ancestors {
                        heads: CommitRef(Symbol("c")),
                        generation: 1..2,
                        parents_range: 0..4294967295,
                    },
                ),
                CommitRef(Symbol("d")),
            ),
            heads: CommitRef(Symbol("b")),
            generation: 0..18446744073709551615,
            parents_range: 0..4294967295,
        }
        "#);

        // Ancestors with a bounded generation range should not be merged.
        insta::assert_debug_snapshot!(optimize(parse("ancestors(a, 2) | ancestors(b)").unwrap()), @r#"
        Union(
            Ancestors {
                heads: CommitRef(Symbol("a")),
                generation: 0..2,
                parents_range: 0..4294967295,
            },
            Ancestors {
                heads: CommitRef(Symbol("b")),
                generation: 0..18446744073709551615,
                parents_range: 0..4294967295,
            },
        )
        "#);
    }

    #[test]
    fn test_optimize_sort_negations_and_ancestors() {
        let settings = insta_settings();
        let _guard = settings.bind_to_scope();

        // Negated ancestors and ancestors should be moved to the left, and other
        // negations should be moved to the right.
        insta::assert_debug_snapshot!(optimize(parse("~a & ::b & ~::c & d ~ e & f & ::g & ~::h").unwrap()), @r#"
        Difference(
            Difference(
                Intersection(
                    Intersection(
                        Intersection(
                            Range {
                                roots: Union(
                                    CommitRef(Symbol("c")),
                                    CommitRef(Symbol("h")),
                                ),
                                heads: CommitRef(Symbol("b")),
                                generation: 0..18446744073709551615,
                                parents_range: 0..4294967295,
                            },
                            Ancestors {
                                heads: CommitRef(Symbol("g")),
                                generation: 0..18446744073709551615,
                                parents_range: 0..4294967295,
                            },
                        ),
                        CommitRef(Symbol("d")),
                    ),
                    CommitRef(Symbol("f")),
                ),
                CommitRef(Symbol("a")),
            ),
            CommitRef(Symbol("e")),
        )
        "#);
    }

    #[test]
    fn test_optimize_heads_range() {
        let settings = insta_settings();
        let _guard = settings.bind_to_scope();

        // Heads of basic range operators can be folded.
        insta::assert_debug_snapshot!(optimize(parse("heads(::)").unwrap()), @r"
        HeadsRange {
            roots: None,
            heads: VisibleHeadsOrReferenced,
            parents_range: 0..4294967295,
            filter: All,
        }
        ");
        insta::assert_debug_snapshot!(optimize(parse("heads(::foo)").unwrap()), @r#"
        HeadsRange {
            roots: None,
            heads: CommitRef(Symbol("foo")),
            parents_range: 0..4294967295,
            filter: All,
        }
        "#);
        // It might be better to use `roots: Root`, but it would require adding a
        // special case for `~root()`, and this should be similar in performance.
        insta::assert_debug_snapshot!(optimize(parse("heads(..)").unwrap()), @r"
        HeadsRange {
            roots: None,
            heads: VisibleHeadsOrReferenced,
            parents_range: 0..4294967295,
            filter: NotIn(Root),
        }
        ");
        insta::assert_debug_snapshot!(optimize(parse("heads(foo..)").unwrap()), @r#"
        HeadsRange {
            roots: CommitRef(Symbol("foo")),
            heads: VisibleHeadsOrReferenced,
            parents_range: 0..4294967295,
            filter: All,
        }
        "#);
        insta::assert_debug_snapshot!(optimize(parse("heads(..bar)").unwrap()), @r#"
        HeadsRange {
            roots: Root,
            heads: CommitRef(Symbol("bar")),
            parents_range: 0..4294967295,
            filter: All,
        }
        "#);
        insta::assert_debug_snapshot!(optimize(parse("heads(foo..bar)").unwrap()), @r#"
        HeadsRange {
            roots: CommitRef(Symbol("foo")),
            heads: CommitRef(Symbol("bar")),
            parents_range: 0..4294967295,
            filter: All,
        }
        "#);
        insta::assert_debug_snapshot!(optimize(parse("heads(~::foo & ::bar)").unwrap()), @r#"
        HeadsRange {
            roots: CommitRef(Symbol("foo")),
            heads: CommitRef(Symbol("bar")),
            parents_range: 0..4294967295,
            filter: All,
        }
        "#);
        insta::assert_debug_snapshot!(optimize(parse("heads(~::foo)").unwrap()), @r#"
        HeadsRange {
            roots: CommitRef(Symbol("foo")),
            heads: VisibleHeadsOrReferenced,
            parents_range: 0..4294967295,
            filter: All,
        }
        "#);
        insta::assert_debug_snapshot!(optimize(parse("heads(a..b & c..d)").unwrap()), @r#"
        HeadsRange {
            roots: Union(
                CommitRef(Symbol("a")),
                CommitRef(Symbol("c")),
            ),
            heads: CommitRef(Symbol("b")),
            parents_range: 0..4294967295,
            filter: Ancestors {
                heads: CommitRef(Symbol("d")),
                generation: 0..18446744073709551615,
                parents_range: 0..4294967295,
            },
        }
        "#);

        // Heads of first-parent ancestors can also be folded.
        insta::assert_debug_snapshot!(optimize(parse("heads(first_ancestors(foo))").unwrap()), @r#"
        HeadsRange {
            roots: None,
            heads: CommitRef(Symbol("foo")),
            parents_range: 0..1,
            filter: All,
        }
        "#);
        insta::assert_debug_snapshot!(optimize(parse("heads(first_ancestors(foo) & bar..)").unwrap()), @r#"
        HeadsRange {
            roots: CommitRef(Symbol("bar")),
            heads: CommitRef(Symbol("foo")),
            parents_range: 0..1,
            filter: All,
        }
        "#);
        insta::assert_debug_snapshot!(optimize(parse("heads(foo.. & first_ancestors(bar) & ::baz)").unwrap()), @r#"
        HeadsRange {
            roots: CommitRef(Symbol("foo")),
            heads: CommitRef(Symbol("bar")),
            parents_range: 0..1,
            filter: Ancestors {
                heads: CommitRef(Symbol("baz")),
                generation: 0..18446744073709551615,
                parents_range: 0..4294967295,
            },
        }
        "#);

        // Ancestors with a limited depth should not be optimized.
        insta::assert_debug_snapshot!(optimize(parse("heads(ancestors(foo, 2))").unwrap()), @r#"
        Heads(
            Ancestors {
                heads: CommitRef(Symbol("foo")),
                generation: 0..2,
                parents_range: 0..4294967295,
            },
        )
        "#);
        insta::assert_debug_snapshot!(optimize(parse("heads(first_ancestors(foo, 2))").unwrap()), @r#"
        Heads(
            Ancestors {
                heads: CommitRef(Symbol("foo")),
                generation: 0..2,
                parents_range: 0..1,
            },
        )
        "#);

        // Generation folding should not prevent optimizing heads.
        insta::assert_debug_snapshot!(optimize(parse("heads(ancestors(foo--))").unwrap()), @r#"
        HeadsRange {
            roots: None,
            heads: Ancestors {
                heads: CommitRef(Symbol("foo")),
                generation: 2..3,
                parents_range: 0..4294967295,
            },
            parents_range: 0..4294967295,
            filter: All,
        }
        "#);
        insta::assert_debug_snapshot!(optimize(parse("heads(first_ancestors(first_parent(foo, 2)))").unwrap()), @r#"
        HeadsRange {
            roots: None,
            heads: Ancestors {
                heads: CommitRef(Symbol("foo")),
                generation: 2..3,
                parents_range: 0..1,
            },
            parents_range: 0..1,
            filter: All,
        }
        "#);

        // Heads of filters and negations can be folded.
        insta::assert_debug_snapshot!(optimize(parse("heads(author_name(A) | author_name(B))").unwrap()), @r#"
        HeadsRange {
            roots: None,
            heads: VisibleHeadsOrReferenced,
            parents_range: 0..4294967295,
            filter: AsFilter(
                Union(
                    Filter(AuthorName(Pattern(Substring("A")))),
                    Filter(AuthorName(Pattern(Substring("B")))),
                ),
            ),
        }
        "#);
        insta::assert_debug_snapshot!(optimize(parse("heads(~author_name(A))").unwrap()), @r#"
        HeadsRange {
            roots: None,
            heads: VisibleHeadsOrReferenced,
            parents_range: 0..4294967295,
            filter: AsFilter(
                NotIn(
                    Filter(AuthorName(Pattern(Substring("A")))),
                ),
            ),
        }
        "#);
        insta::assert_debug_snapshot!(optimize(parse("heads(~foo)").unwrap()), @r#"
        HeadsRange {
            roots: None,
            heads: VisibleHeadsOrReferenced,
            parents_range: 0..4294967295,
            filter: NotIn(CommitRef(Symbol("foo"))),
        }
        "#);

        // Heads of intersections with filters can be folded.
        insta::assert_debug_snapshot!(optimize(parse("heads(author_name(A) & ::foo ~ author_name(B))").unwrap()), @r#"
        HeadsRange {
            roots: None,
            heads: CommitRef(Symbol("foo")),
            parents_range: 0..4294967295,
            filter: AsFilter(
                Difference(
                    Filter(AuthorName(Pattern(Substring("A")))),
                    Filter(AuthorName(Pattern(Substring("B")))),
                ),
            ),
        }
        "#);

        // Heads of intersections with negations can be folded.
        insta::assert_debug_snapshot!(optimize(parse("heads(~foo & ~roots(bar) & ::baz)").unwrap()), @r#"
        HeadsRange {
            roots: None,
            heads: CommitRef(Symbol("baz")),
            parents_range: 0..4294967295,
            filter: Difference(
                NotIn(CommitRef(Symbol("foo"))),
                Roots(CommitRef(Symbol("bar"))),
            ),
        }
        "#);
    }

    #[test]
    fn test_optimize_ancestors_heads_range() {
        // Can use heads range to optimize ancestors of filter.
        insta::assert_debug_snapshot!(optimize(parse("::description(bar)").unwrap()), @r#"
        Ancestors {
            heads: HeadsRange {
                roots: None,
                heads: VisibleHeadsOrReferenced,
                parents_range: 0..4294967295,
                filter: Filter(
                    Description(
                        Pattern(
                            Substring(
                                "bar",
                            ),
                        ),
                    ),
                ),
            },
            generation: 0..18446744073709551615,
            parents_range: 0..4294967295,
        }
        "#);
        insta::assert_debug_snapshot!(optimize(parse("author_name(foo)..").unwrap()), @r#"
        Range {
            roots: HeadsRange {
                roots: None,
                heads: VisibleHeadsOrReferenced,
                parents_range: 0..4294967295,
                filter: Filter(
                    AuthorName(
                        Pattern(
                            Substring(
                                "foo",
                            ),
                        ),
                    ),
                ),
            },
            heads: VisibleHeadsOrReferenced,
            generation: 0..18446744073709551615,
            parents_range: 0..4294967295,
        }
        "#);

        // Can use heads range to optimize ancestors of range.
        insta::assert_debug_snapshot!(optimize(parse("::(foo..bar)").unwrap()), @r#"
        Ancestors {
            heads: HeadsRange {
                roots: CommitRef(
                    Symbol(
                        "foo",
                    ),
                ),
                heads: CommitRef(
                    Symbol(
                        "bar",
                    ),
                ),
                parents_range: 0..4294967295,
                filter: All,
            },
            generation: 0..18446744073709551615,
            parents_range: 0..4294967295,
        }
        "#);

        // Can't optimize if not using full generation and parents ranges.
        insta::assert_debug_snapshot!(optimize(parse("ancestors(author_name(foo), 5)").unwrap()), @r#"
        Ancestors {
            heads: Filter(
                AuthorName(
                    Pattern(
                        Substring(
                            "foo",
                        ),
                    ),
                ),
            ),
            generation: 0..5,
            parents_range: 0..4294967295,
        }
        "#);
        insta::assert_debug_snapshot!(optimize(parse("first_ancestors(author_name(foo))").unwrap()), @r#"
        Ancestors {
            heads: Filter(
                AuthorName(
                    Pattern(
                        Substring(
                            "foo",
                        ),
                    ),
                ),
            ),
            generation: 0..18446744073709551615,
            parents_range: 0..1,
        }
        "#);
    }

    #[test]
    fn test_escape_string_literal() {
        // Valid identifiers don't need quoting
        assert_eq!(format_symbol("foo"), "foo");
        assert_eq!(format_symbol("foo.bar"), "foo.bar");

        // Invalid identifiers need quoting
        assert_eq!(format_symbol("foo@bar"), r#""foo@bar""#);
        assert_eq!(format_symbol("foo bar"), r#""foo bar""#);
        assert_eq!(format_symbol(" foo "), r#"" foo ""#);
        assert_eq!(format_symbol("(foo)"), r#""(foo)""#);
        assert_eq!(format_symbol("all:foo"), r#""all:foo""#);

        // Some characters also need escaping
        assert_eq!(format_symbol("foo\"bar"), r#""foo\"bar""#);
        assert_eq!(format_symbol("foo\\bar"), r#""foo\\bar""#);
        assert_eq!(format_symbol("foo\\\"bar"), r#""foo\\\"bar""#);
        assert_eq!(format_symbol("foo\nbar"), r#""foo\nbar""#);

        // Some characters don't technically need escaping, but we escape them for
        // clarity
        assert_eq!(format_symbol("foo\"bar"), r#""foo\"bar""#);
        assert_eq!(format_symbol("foo\\bar"), r#""foo\\bar""#);
        assert_eq!(format_symbol("foo\\\"bar"), r#""foo\\\"bar""#);
        assert_eq!(format_symbol("foo \x01 bar"), r#""foo \x01 bar""#);
    }

    #[test]
    fn test_escape_remote_symbol() {
        assert_eq!(format_remote_symbol("foo", "bar"), "foo@bar");
        assert_eq!(
            format_remote_symbol(" foo ", "bar:baz"),
            r#"" foo "@"bar:baz""#
        );
    }
}
