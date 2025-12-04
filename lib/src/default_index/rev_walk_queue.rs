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
use std::mem;

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(super) struct RevWalkWorkItem<P, T> {
    pub pos: P,
    pub value: T,
}

#[derive(Clone)]
pub(super) struct RevWalkQueue<P, T> {
    items: BinaryHeap<RevWalkWorkItem<P, T>>,
    // Optionally keep the greatest item out of the heap, so pop() + push() of
    // the greatest item won't have to rebalance the heap.
    scratch_item: Option<RevWalkWorkItem<P, T>>,
    min_pos: P,
}

impl<P: Ord, T: Ord> RevWalkQueue<P, T> {
    pub fn with_min_pos(min_pos: P) -> Self {
        Self {
            items: BinaryHeap::new(),
            scratch_item: None,
            min_pos,
        }
    }

    #[cfg_attr(not(test), expect(dead_code))]
    pub fn len(&self) -> usize {
        self.items.len() + usize::from(self.scratch_item.is_some())
    }

    pub fn push(&mut self, pos: P, value: T) {
        if pos < self.min_pos {
            return;
        }
        self.push_item(RevWalkWorkItem { pos, value });
    }

    fn push_item(&mut self, new: RevWalkWorkItem<P, T>) {
        if let Some(next) = self.scratch_item.as_mut() {
            if new < *next {
                self.items.push(new);
            } else {
                let next = mem::replace(next, new);
                self.items.push(next);
            }
        } else if let Some(next) = self.items.peek() {
            if new < *next {
                // items[0] could be moved to scratch_item, but simply leave
                // scratch_item empty.
                self.items.push(new);
            } else {
                self.scratch_item = Some(new);
            }
        } else {
            self.scratch_item = Some(new);
        }
    }

    pub fn extend(&mut self, positions: impl IntoIterator<Item = P>, value: T)
    where
        T: Clone,
    {
        for pos in positions {
            self.push(pos, value.clone());
        }
    }

    pub fn peek(&self) -> Option<&RevWalkWorkItem<P, T>> {
        self.scratch_item.as_ref().or_else(|| self.items.peek())
    }

    pub fn pop(&mut self) -> Option<RevWalkWorkItem<P, T>> {
        let next = self.scratch_item.take().or_else(|| self.items.pop())?;
        Some(next)
    }

    pub fn pop_if(
        &mut self,
        predicate: impl FnOnce(&RevWalkWorkItem<P, T>) -> bool,
    ) -> Option<RevWalkWorkItem<P, T>> {
        let next = self.peek()?;
        predicate(next).then(|| self.pop().unwrap())
    }

    pub fn pop_eq(&mut self, pos: &P) -> Option<RevWalkWorkItem<P, T>> {
        self.pop_if(|next| next.pos == *pos)
    }

    pub fn skip_while_eq(&mut self, pos: &P) {
        while self.pop_eq(pos).is_some() {
            continue;
        }
    }
}

#[cfg(test)]
mod tests {
    use assert_matches::assert_matches;

    use super::*;

    #[test]
    fn test_push_pop_in_forward_order() {
        let mut queue: RevWalkQueue<u32, ()> = RevWalkQueue::with_min_pos(0);

        queue.push(0, ());
        assert!(queue.scratch_item.is_some());
        assert_eq!(queue.items.len(), 0);

        queue.push(1, ());
        assert!(queue.scratch_item.is_some());
        assert_eq!(queue.items.len(), 1);

        assert_matches!(queue.pop(), Some(RevWalkWorkItem { pos: 1, .. }));
        assert!(queue.scratch_item.is_none());
        assert_eq!(queue.items.len(), 1);

        queue.push(2, ());
        assert!(queue.scratch_item.is_some());
        assert_eq!(queue.items.len(), 1);

        assert_matches!(queue.pop(), Some(RevWalkWorkItem { pos: 2, .. }));
        assert!(queue.scratch_item.is_none());
        assert_eq!(queue.items.len(), 1);

        assert_matches!(queue.pop(), Some(RevWalkWorkItem { pos: 0, .. }));
        assert!(queue.scratch_item.is_none());
        assert_eq!(queue.items.len(), 0);

        assert_matches!(queue.pop(), None);
    }

    #[test]
    fn test_push_pop_in_reverse_order() {
        let mut queue: RevWalkQueue<u32, ()> = RevWalkQueue::with_min_pos(0);

        queue.push(2, ());
        assert!(queue.scratch_item.is_some());
        assert_eq!(queue.items.len(), 0);

        queue.push(1, ());
        assert!(queue.scratch_item.is_some());
        assert_eq!(queue.items.len(), 1);

        assert_matches!(queue.pop(), Some(RevWalkWorkItem { pos: 2, .. }));
        assert!(queue.scratch_item.is_none());
        assert_eq!(queue.items.len(), 1);

        queue.push(0, ());
        assert!(queue.scratch_item.is_none());
        assert_eq!(queue.items.len(), 2);

        assert_matches!(queue.pop(), Some(RevWalkWorkItem { pos: 1, .. }));
        assert!(queue.scratch_item.is_none());
        assert_eq!(queue.items.len(), 1);

        assert_matches!(queue.pop(), Some(RevWalkWorkItem { pos: 0, .. }));
        assert!(queue.scratch_item.is_none());
        assert_eq!(queue.items.len(), 0);

        assert_matches!(queue.pop(), None);
    }
}
