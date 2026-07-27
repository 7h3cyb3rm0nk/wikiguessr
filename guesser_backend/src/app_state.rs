use crate::resources::manager::ResourceManager;
use crate::rooms::registry::RoomRegistry;
use std::sync::{Arc, RwLock};
pub struct AppState {
    pub resources: Arc<RwLock<ResourceManager>>,
    pub rooms: Arc<RwLock<RoomRegistry>>,
}
