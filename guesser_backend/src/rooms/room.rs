use crate::rooms::types::PlayerId;
use std::sync::Arc;
use uuid::Uuid;
pub type RoomId = Uuid;

#[derive(Debug, Clone)]
pub struct Room {
    id: RoomId,
    players: Arc<Vec<PlayerId>>,
}

impl Room {
    fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            players: Arc::new(Vec::new()),
        }
    }

    pub fn id(&self) -> RoomId {
        self.id
    }
}
