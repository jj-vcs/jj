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

//! Benchmarks for the revset graph iteration pipeline, approximating the cost
//! of `jj log` (excluding template rendering). Each benchmark iteration covers
//! revset evaluation and topo-grouped graph traversal — the two phases `jj log`
//! runs before rendering each commit.

use criterion::BenchmarkId;
use criterion::Criterion;
use criterion::criterion_group;
use criterion::criterion_main;
use itertools::Itertools as _;
use jj_lib::graph::TopoGroupedGraphIterator;
use jj_lib::revset::RevsetExpression;
use pollster::FutureExt as _;
use testutils::TestRepo;
use testutils::write_random_commit;
use testutils::write_random_commit_with_parents;

/// Benchmark over a long linear chain of hidden commits.
///
/// Graph shape (N = chain length):
///
///   tip (visible)
///    |
///   [N hidden commits]
///    |
///   root (visible)
///
/// This exercises `edges_from_external_commit` (two-phase BFS) and
/// `count_hidden_to_visible` (the elided-count BFS) over a deep linear path.
fn bench_linear_chain(c: &mut Criterion) {
    let mut group = c.benchmark_group("revset_graph/linear_chain");
    for &n in &[100usize, 1_000] {
        // Build the repo once outside the measurement loop.
        let test_repo = TestRepo::init();
        let mut tx = test_repo.repo.start_transaction();
        let root_visible = write_random_commit(tx.repo_mut());
        let mut prev = root_visible.clone();
        for _ in 0..n {
            prev = write_random_commit_with_parents(tx.repo_mut(), &[&prev]);
        }
        let tip_visible = write_random_commit_with_parents(tx.repo_mut(), &[&prev]);
        let repo = tx.commit("bench").block_on().unwrap();

        let expression =
            RevsetExpression::commits(vec![root_visible.id().clone(), tip_visible.id().clone()]);

        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
            b.iter(|| {
                let revset = expression.clone().evaluate(repo.as_ref()).unwrap();
                TopoGroupedGraphIterator::new(revset.iter_graph(), |id| id)
                    .try_collect::<_, Vec<_>, _>()
                    .unwrap()
            });
        });
    }
    group.finish();
}

/// Benchmark over a staircase of diamond-shaped hidden subgraphs.
///
/// Graph shape for D=2 layers, W=3 branches (all intermediate commits hidden):
///
///   tip  (visible)
///    |
///  merge_1  (hidden)
///  / | \
/// b0 b1 b2  (hidden)
///  \ | /
///  merge_0  (hidden)
///  / | \
/// b0 b1 b2  (hidden)
///  \ | /
///   root  (visible)
///
/// Parameterized by D (depth = number of diamond layers) and W (width =
/// branches per layer).
fn bench_diamond_staircase(c: &mut Criterion) {
    let mut group = c.benchmark_group("revset_graph/diamond_staircase");
    // (depth, width): depth = number of diamond layers, width = branches per layer
    for &(depth, width) in &[(200usize, 2usize), (100, 4), (40, 10)] {
        let test_repo = TestRepo::init();
        let mut tx = test_repo.repo.start_transaction();
        let root_visible = write_random_commit(tx.repo_mut());
        let mut base = root_visible.clone();
        for _ in 0..depth {
            let branches: Vec<_> = (0..width)
                .map(|_| write_random_commit_with_parents(tx.repo_mut(), &[&base]))
                .collect();
            let branch_refs: Vec<&_> = branches.iter().collect();
            base = write_random_commit_with_parents(tx.repo_mut(), &branch_refs);
        }
        let tip_visible = write_random_commit_with_parents(tx.repo_mut(), &[&base]);
        let repo = tx.commit("bench").block_on().unwrap();

        let expression =
            RevsetExpression::commits(vec![root_visible.id().clone(), tip_visible.id().clone()]);

        let label = format!("d{depth}_w{width}");
        group.bench_with_input(BenchmarkId::from_parameter(&label), &label, |b, _| {
            b.iter(|| {
                let revset = expression.clone().evaluate(repo.as_ref()).unwrap();
                TopoGroupedGraphIterator::new(revset.iter_graph(), |id| id)
                    .try_collect::<_, Vec<_>, _>()
                    .unwrap()
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_linear_chain, bench_diamond_staircase);
criterion_main!(benches);
