use crate::location::coordinates::Location;
use std::collections::VecDeque;
use std::sync::Mutex;

/// Thread-safe pool of pre-fetched locations.
///
/// A background worker keeps this filled; room actors pop from the front.
/// `Mutex` is correct here (not `RwLock`) because every operation mutates.
pub struct LocationPool {
    inner: Mutex<VecDeque<Location>>,
}

impl LocationPool {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(VecDeque::new()),
        }
    }

    /// Pop the next location from the front of the pool.
    /// Returns `None` if the pool is empty.
    pub fn pop(&self) -> Option<Location> {
        self.inner.lock().unwrap().pop_front()
    }

    /// Push a batch of locations to the back of the pool.
    pub fn push_batch(&self, locations: Vec<Location>) {
        let mut pool = self.inner.lock().unwrap();
        pool.extend(locations);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::location::coordinates::{Coordinate, ItemId};

    fn test_location(id: &str) -> Location {
        Location {
            item_id: ItemId(id.to_string()),
            coordinate: Coordinate {
                latitude: 0.0,
                longitude: 0.0,
            },
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
        pool.push_batch(vec![test_location("Q1"), test_location("Q2"), test_location("Q3")]);
        assert_eq!(pool.len(), 3);

        assert_eq!(pool.pop().unwrap().item_id.0, "Q1");
        assert_eq!(pool.pop().unwrap().item_id.0, "Q2");
        assert_eq!(pool.pop().unwrap().item_id.0, "Q3");
        assert!(pool.pop().is_none());
    }

    #[test]
    fn multiple_batches_append() {
        let pool = LocationPool::new();
        pool.push_batch(vec![test_location("Q1")]);
        pool.push_batch(vec![test_location("Q2")]);
        assert_eq!(pool.len(), 2);
        assert_eq!(pool.pop().unwrap().item_id.0, "Q1");
        assert_eq!(pool.pop().unwrap().item_id.0, "Q2");
    }
}
