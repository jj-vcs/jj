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

use std::collections::BTreeMap;
use std::fmt::Debug;

use itertools::Either;
use itertools::Itertools as _;
use jj_lib::repo_path::RepoPath;
use jj_lib::repo_path::RepoPathBuf;
use jj_lib::repo_path::RepoPathComponentBuf;
use proptest::collection::btree_map;
use proptest::collection::vec;
use proptest::prelude::*;
use proptest::sample::select;
use proptest_state_machine::ReferenceStateMachine;

#[derive(Debug, Clone)]
pub struct RepoRefState {
    root: Tree,
}

#[derive(Debug, Clone)]
pub enum File {
    RegularFile { contents: String, executable: bool },
}

#[derive(Debug, Clone)]
pub enum Transition {
    /// Create a file with the given contents at `path`.
    ///
    /// Parent directories are created as necessary, existing files or
    /// directories are replaced.
    CreateFile {
        path: RepoPathBuf,
        contents: String,
        executable: bool,
    },

    /// Delete the file at `path`.
    ///
    /// Emptied parent directories are cleaned up (expect repo root).
    DeleteFile { path: RepoPathBuf },
}

#[derive(Clone, Default)]
struct Tree {
    nodes: BTreeMap<RepoPathComponentBuf, Node>,
}

#[derive(Clone)]
enum Node {
    File(File),
    Directory(Tree),
}

impl Debug for Tree {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.nodes
            .iter()
            .fold(&mut f.debug_struct("Directory"), |list, (name, node)| {
                list.field(name.as_internal_str(), node)
            })
            .finish()
    }
}

impl Debug for Node {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Node::File(file) => file.fmt(f),
            Node::Directory(tree) => tree.fmt(f),
        }
    }
}

impl Default for Node {
    fn default() -> Self {
        Node::Directory(Tree::default())
    }
}

impl RepoRefState {
    pub fn files(&self) -> impl IntoIterator<Item = (RepoPathBuf, &File)> + '_ {
        self.root.files(RepoPath::root())
    }
}

impl Tree {
    fn files(&self, prefix: &RepoPath) -> Vec<(RepoPathBuf, &File)> {
        self.nodes
            .iter()
            .filter_map(move |(name, node)| match node {
                Node::File(file) => Some((prefix.join(name), file)),
                Node::Directory(_) => None,
            })
            .chain(
                self.nodes
                    .iter()
                    .filter_map(move |(name, node)| match node {
                        Node::File(_) => None,
                        Node::Directory(tree) => Some((prefix.join(name), tree)),
                    })
                    .flat_map(|(path, tree)| tree.files(&path)),
            )
            .collect()
    }
}

fn arb_tree() -> impl Strategy<Value = Tree> {
    btree_map(arb_path_component(), arb_node(), 1..8).prop_map(|nodes| Tree { nodes })
}

fn arb_path_component() -> impl Strategy<Value = RepoPathComponentBuf> {
    // biased towards naming collisions (alpha-delta) but with the option to
    // generate arbitrary UTF-8
    "(alpha|beta|gamma|delta|[\\PC&&[^/]]+)".prop_map(|s| RepoPathComponentBuf::new(s).unwrap())
}

fn arb_node() -> impl Strategy<Value = Node> {
    let file_leaf = ("[a-z]{0,3}", proptest::bool::ANY).prop_map(|(contents, executable)| {
        Node::File(File::RegularFile {
            contents,
            executable,
        })
    });
    file_leaf.prop_recursive(4, 8, 8, |inner| {
        btree_map(arb_path_component(), inner, 1..8)
            .prop_map(|nodes| Node::Directory(Tree { nodes }))
    })
}

fn arb_extant_dir(root: Tree) -> impl Strategy<Value = RepoPathBuf> {
    fn arb_extant_dir_recursive(path: &RepoPath, tree: Tree) -> impl Strategy<Value = RepoPathBuf> {
        let subdirs: Vec<_> = tree
            .nodes
            .into_iter()
            .filter_map(|(name, node)| match node {
                Node::File(_) => None,
                Node::Directory(tree) => Some((path.join(&name), tree)),
            })
            .collect();

        if subdirs.is_empty() {
            Just(path.to_owned()).boxed()
        } else {
            prop_oneof![
                Just(path.to_owned()),
                select(subdirs)
                    .prop_flat_map(|(subdir, subtree)| arb_extant_dir_recursive(&subdir, subtree)),
            ]
            .boxed()
        }
    }

    arb_extant_dir_recursive(RepoPath::root(), root)
}

fn arb_extant_file(root: Tree) -> impl Strategy<Value = RepoPathBuf> {
    fn arb_extant_file_recursive(
        path: &RepoPath,
        tree: Tree,
    ) -> impl Strategy<Value = RepoPathBuf> {
        let (files, subdirs): (Vec<_>, Vec<_>) =
            tree.nodes
                .into_iter()
                .partition_map(|(name, node)| match node {
                    Node::File(_) => Either::Left(path.join(&name)),
                    Node::Directory(tree) => Either::Right((path.join(&name), tree)),
                });

        match (&files[..], &subdirs[..]) {
            ([], []) => unreachable!("directory must not be empty"),
            ([], _) => select(subdirs)
                .prop_flat_map(|(subdir, subtree)| arb_extant_file_recursive(&subdir, subtree))
                .boxed(),
            (_, []) => select(files).boxed(),
            (_, _) => prop_oneof![
                select(files),
                select(subdirs)
                    .prop_flat_map(|(subdir, subtree)| arb_extant_file_recursive(&subdir, subtree)),
            ]
            .boxed(),
        }
    }

    arb_extant_file_recursive(RepoPath::root(), root)
}

fn arb_transition_create_file(state: &RepoRefState) -> impl Strategy<Value = Transition> {
    arb_extant_dir(state.root.clone())
        .prop_flat_map(|dir| {
            vec(arb_path_component(), 1..4).prop_map(move |new_path_components| {
                let mut file_path = dir.clone();
                file_path.extend(new_path_components);
                file_path
            })
        })
        .prop_flat_map(|path| {
            ("[a-z]{0,3}", proptest::bool::ANY).prop_map(move |(contents, executable)| {
                Transition::CreateFile {
                    path: path.clone(),
                    contents,
                    executable,
                }
            })
        })
}

fn arb_transition_delete_file(state: &RepoRefState) -> impl Strategy<Value = Transition> {
    arb_extant_file(state.root.clone()).prop_map(|path| Transition::DeleteFile { path })
}

impl ReferenceStateMachine for RepoRefState {
    type State = Self;

    type Transition = Transition;

    fn init_state() -> BoxedStrategy<Self::State> {
        prop_oneof![
            1 => Just(Self {
                root: Tree::default()
            }),
            10 => arb_tree().prop_map(|root| Self { root }),
        ]
        .boxed()
    }

    fn transitions(state: &Self::State) -> BoxedStrategy<Self::Transition> {
        if state.root.nodes.is_empty() {
            arb_transition_create_file(state).boxed()
        } else {
            prop_oneof![
                arb_transition_create_file(state),
                arb_transition_delete_file(state),
            ]
            .boxed()
        }
    }

    fn apply(mut state: Self::State, transition: &Self::Transition) -> Self::State {
        match transition {
            Transition::CreateFile {
                path,
                contents,
                executable,
            } => {
                let mut components = path.components();
                let Some(filename) = components.next_back() else {
                    panic!("file path cannot be empty");
                };
                let directory = components.fold(&mut state.root, |tree, pc| {
                    let Node::Directory(tree) = tree
                        .nodes
                        .entry(pc.to_owned())
                        .and_modify(|node| {
                            if let Node::File { .. } = node {
                                // replace any files along the way with directories
                                *node = Node::default();
                            }
                        })
                        .or_default()
                    else {
                        panic!("encountered file, expected directory: {pc:?}");
                    };
                    tree
                });

                directory.nodes.insert(
                    filename.to_owned(),
                    Node::File(File::RegularFile {
                        contents: contents.clone(),
                        executable: *executable,
                    }),
                );
            }
            Transition::DeleteFile { path } => {
                fn delete_recursive(
                    directory: &mut Tree,
                    components: &mut jj_lib::repo_path::RepoPathComponentsIter<'_>,
                ) -> bool {
                    let component = components.next().expect("trying to delete a directory");

                    match directory.nodes.get_mut(component) {
                        Some(Node::File { .. }) => {
                            assert!(
                                components.next().is_none(),
                                "file does not exist: {component:?} is not a directory"
                            );
                            directory.nodes.remove(component);
                            true
                        }
                        Some(Node::Directory(tree)) => {
                            if delete_recursive(tree, components) && tree.nodes.is_empty() {
                                directory.nodes.remove(component);
                                true
                            } else {
                                false
                            }
                        }
                        None => false,
                    }
                }

                delete_recursive(&mut state.root, &mut path.components());
            }
        }

        state
    }
}
