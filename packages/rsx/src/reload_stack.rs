/// A vec that's optimized for finding and removing elements that match a predicate.
///
/// Currently will do a linear search for the first element that matches the predicate.
/// Uses a next_free pointer to optimize the search such that future searches start from left-most
/// non-None item, making it O(1) on average for sorted input.
///
/// The motivating factor here is that hashes are expensive and actually quite hard to maintain for
/// callbody. Hashing would imply a number of nested invariants that are hard to maintain.
///
/// Deriving hash will start to slurp up private fields which is not what we want, so the comparison
/// function is moved here to the reloadstack interface.
pub struct PopVec<T> {
    stack: Vec<Option<T>>,
    next_free: usize,
}

impl<T> PopVec<T> {
    pub fn new(f: impl Iterator<Item = T>) -> Self {
        let stack = f.map(Some).collect();
        Self {
            stack,
            next_free: 0,
        }
    }

    pub fn remove(&mut self, idx: usize) -> Option<T> {
        let item = self.stack.get_mut(idx).unwrap().take();

        // move the next_free pointer to the right-most non-none element
        for i in self.next_free..=idx {
            if self.stack[i].is_some() {
                break;
            }
            self.next_free = i + 1;
        }

        item
    }

    pub fn pop_where(&mut self, f: impl Fn(&T) -> bool) -> Option<T> {
        let idx = self
            .stack
            .iter()
            .position(|x| if let Some(x) = x { f(x) } else { false })?;

        self.remove(idx)
    }

    /// Returns the index and score of the highest scored element
    ///
    /// shortcircuits if the score is usize::MAX
    /// returns None if the score was 0
    pub fn highest_score(&self, score: impl Fn(&T) -> usize) -> Option<(usize, usize)> {
        let mut highest_score = 0;
        let mut best = None;

        // todo: this reload stack is actually meant to allow quick jumps from the start to the nearest
        // non-none item for searching, making it, on average, O(1) when the input is sorted.
        for (idx, x) in self.stack.iter().enumerate().skip(self.next_free) {
            if let Some(x) = x {
                let scored = score(x);
                if scored > highest_score {
                    best = Some(idx);
                    highest_score = scored;
                }

                if highest_score == usize::MAX {
                    break;
                }
            }
        }

        if highest_score == 0 {
            return None;
        }

        best.map(|idx| (idx, highest_score))
    }

    pub fn is_empty(&self) -> bool {
        self.stack.is_empty()
    }

    pub fn raw_len(&self) -> usize {
        self.stack.len()
    }
}

#[test]
fn searches_and_works() {
    let mut stack = PopVec::new(vec![1, 2, 3, 4, 5].into_iter());

    assert_eq!(stack.pop_where(|x| *x == 3), Some(3));
    assert_eq!(stack.pop_where(|x| *x == 1), Some(1));
    assert_eq!(stack.pop_where(|x| *x == 5), Some(5));
    assert_eq!(stack.pop_where(|x| *x == 2), Some(2));
    assert_eq!(stack.pop_where(|x| *x == 4), Some(4));
    assert_eq!(stack.pop_where(|x| *x == 4), None);

    assert!(stack.is_empty());
}

#[test]
fn free_optimization_works() {
    let mut stack = PopVec::new(vec![0, 1, 2, 3, 4, 5].into_iter());

    _ = stack.remove(0);
    assert_eq!(stack.next_free, 1);

    _ = stack.remove(1);
    assert_eq!(stack.next_free, 2);

    _ = stack.remove(4);
    assert_eq!(stack.next_free, 2);

    _ = stack.remove(2);
    assert_eq!(stack.next_free, 3);
}
