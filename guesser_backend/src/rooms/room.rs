use crate::ids::{PlayerId, RoomId};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct Room {
    id: RoomId,
    players: Arc<Vec<PlayerId>>,
}

impl Room {
    pub fn new() -> Self {
        Self {
            id: RoomId::new(),
            players: Arc::new(Vec::new()),
        }
    }

    pub fn id(&self) -> RoomId {
        self.id
    }
}
