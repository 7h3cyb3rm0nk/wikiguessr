use crate::image::Image;
use crate::location::coordinates::Location;
use std::collections::VecDeque;
use std::sync::Mutex;

#[derive(Debug, Clone)]
pub struct PreparedRound {
    pub location: Location,
    pub images: Vec<Image>,
}

/// Thread-safe pool of pre-fetched locations.
///
/// A background worker keeps this filled; room actors pop from the front.
/// `Mutex` is correct here (not `RwLock`) because every operation mutates.
pub struct LocationPool {
    inner: Mutex<VecDeque<PreparedRound>>,
}

impl LocationPool {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(VecDeque::new()),
        }
    }

    /// Pop the next location from the front of the pool.
    /// Returns `None` if the pool is empty.
    pub fn pop(&self) -> Option<PreparedRound> {
        self.inner.lock().unwrap().pop_front()
    }

    /// Push a batch of locations to the back of the pool.
    pub fn push_batch(&self, rounds: Vec<PreparedRound>) {
        let mut pool = self.inner.lock().unwrap();
        pool.extend(rounds);
    }

    /// Current number of locations available.
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().len()
    }

    /// Whether the pool is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for LocationPool {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::location::coordinates::{Coordinate, ItemId};

    fn test_round(id: &str) -> PreparedRound {
        PreparedRound {
            location: Location {
                item_id: ItemId(id.to_string()),
                coordinate: Coordinate {
                    latitude: 0.0,
                    longitude: 0.0,
                },
            },
            images: vec![],
        }
    }

    #[test]
    fn pop_from_empty_pool_returns_none() {
        let pool = LocationPool::new();
        assert!(pool.pop().is_none());
        assert!(pool.is_empty());
    }

    #[test]
    fn push_and_pop_fifo_order() {
        let pool = LocationPool::new();
        pool.push_batch(vec![
            test_round("Q1"),
            test_round("Q2"),
            test_round("Q3"),
        ]);
        assert_eq!(pool.len(), 3);

        assert_eq!(pool.pop().unwrap().location.item_id.0, "Q1");
        assert_eq!(pool.pop().unwrap().location.item_id.0, "Q2");
        assert_eq!(pool.pop().unwrap().location.item_id.0, "Q3");
        assert!(pool.pop().is_none());
    }

    #[test]
    fn multiple_batches_append() {
        let pool = LocationPool::new();
        pool.push_batch(vec![test_round("Q1")]);
        pool.push_batch(vec![test_round("Q2")]);
        assert_eq!(pool.len(), 2);
        assert_eq!(pool.pop().unwrap().location.item_id.0, "Q1");
        assert_eq!(pool.pop().unwrap().location.item_id.0, "Q2");
    }
}
