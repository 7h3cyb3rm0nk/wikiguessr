use crate::config::Config;
use crate::resources::manager::ResourceManager;
use crate::rooms::registry::RoomRegistry;
use std::sync::Arc;

/// Shared application state passed to Axum route handlers.
#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub resources: Arc<ResourceManager>,
    pub rooms: Arc<RoomRegistry>,
}

impl AppState {
    pub fn new(config: Config) -> Self {
        let resources = Arc::new(ResourceManager::new(&config.game));
        let rooms = Arc::new(RoomRegistry::new());
        Self {
            config,
            resources,
            rooms,
        }
    }
}
