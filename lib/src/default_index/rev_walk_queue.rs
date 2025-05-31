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

use std::collections::BinaryHeap;

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd)]
pub(super) struct RevWalkWorkItem<P, T> {
    pub pos: P,
    pub state: RevWalkWorkItemState<T>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum RevWalkWorkItemState<T> {
    // Order matters: Unwanted should appear earlier in the max-heap.
    Wanted(T),
    Unwanted,
}

impl<P, T> RevWalkWorkItem<P, T> {
    pub fn is_wanted(&self) -> bool {
        matches!(self.state, RevWalkWorkItemState::Wanted(_))
    }
}

#[derive(Clone)]
pub(super) struct RevWalkQueue<P, T> {
    items: BinaryHeap<RevWalkWorkItem<P, T>>,
    min_pos: P,
    unwanted_count: usize,
}

impl<P: Ord, T: Ord> RevWalkQueue<P, T> {
    pub fn with_min_pos(min_pos: P) -> Self {
        Self {
            items: BinaryHeap::new(),
            min_pos,
            unwanted_count: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn wanted_count(&self) -> usize {
        self.len() - self.unwanted_count
    }

    pub fn unwanted_count(&self) -> usize {
        self.unwanted_count
    }

    pub fn iter(&self) -> impl Iterator<Item = &RevWalkWorkItem<P, T>> {
        self.items.iter()
    }

    pub fn push_wanted(&mut self, pos: P, t: T) {
        if pos < self.min_pos {
            return;
        }
        let state = RevWalkWorkItemState::Wanted(t);
        self.items.push(RevWalkWorkItem { pos, state });
    }

    pub fn push_unwanted(&mut self, pos: P) {
        if pos < self.min_pos {
            return;
        }
        let state = RevWalkWorkItemState::Unwanted;
        self.items.push(RevWalkWorkItem { pos, state });
        self.unwanted_count += 1;
    }

    pub fn extend_wanted(&mut self, positions: impl IntoIterator<Item = P>, t: T)
    where
        T: Clone,
    {
        // positions typically contains one item, and single BinaryHeap::push()
        // appears to be slightly faster than .extend() as of rustc 1.73.0.
        for pos in positions {
            self.push_wanted(pos, t.clone());
        }
    }

    pub fn extend_unwanted(&mut self, positions: impl IntoIterator<Item = P>) {
        for pos in positions {
            self.push_unwanted(pos);
        }
    }

    pub fn peek(&self) -> Option<&RevWalkWorkItem<P, T>> {
        self.items.peek()
    }

    pub fn pop(&mut self) -> Option<RevWalkWorkItem<P, T>> {
        let next = self.items.pop()?;
        self.unwanted_count -= !next.is_wanted() as usize;
        Some(next)
    }

    pub fn pop_eq(&mut self, pos: &P) -> Option<RevWalkWorkItem<P, T>> {
        let next = self.peek()?;
        (next.pos == *pos).then(|| self.pop().unwrap())
    }

    pub fn skip_while_eq(&mut self, pos: &P) {
        while self.pop_eq(pos).is_some() {
            continue;
        }
    }
}
