use crate::rooms::{
    commands,
    room::Room,
    types::{PlayerId, RoomId},
};
use dashmap::{
    DashMap,
    mapref::one::{Ref, RefMut},
};

pub struct RoomRegistry {
    rooms: DashMap<RoomId, Room>,
}

impl RoomRegistry {
    pub fn new() -> Self {
        Self {
            rooms: DashMap::new(),
        }
    }

    pub fn insert(&self, room: Room) {
        self.rooms.insert(room.id(), room);
    }

    pub fn get(&self, id: &RoomId) -> Option<Ref<'_, RoomId, Room>> {
        self.rooms.get(id)
    }

    pub fn get_mut(&self, id: &RoomId) -> Option<RefMut<'_, RoomId, Room>> {
        self.rooms.get_mut(id)
    }

    pub fn remove(&self, id: &RoomId) -> Option<(RoomId, Room)> {
        self.rooms.remove(id)
    }
}
