use crate::config::GameConfig;
use crate::ids::{RoomCode, RoomId};
use crate::resources::manager::ResourceManager;
use crate::rooms::actor::RoomActor;
use crate::rooms::commands::RoomCommand;
use dashmap::DashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, info};

pub struct RoomRegistry {
    rooms: DashMap<RoomId, mpsc::Sender<RoomCommand>>,
    codes: DashMap<RoomCode, RoomId>,
}

impl RoomRegistry {
    pub fn new() -> Self {
        Self {
            rooms: DashMap::new(),
            codes: DashMap::new(),
        }
    }

    /// Create a new room actor, spawn its task, and store its mailbox sender.
    ///
    /// A cleanup task removes the room from the registry once the actor
    /// terminates (inactivity timeout, last player leave, or mailbox closed),
    /// preventing stale entries from leaking.
    ///
    /// Returns `(room_id, room_code, join_code, host_token, sender)`.
    pub fn create_room(
        self: Arc<Self>,
        config: GameConfig,
        resources: Arc<ResourceManager>,
    ) -> (RoomId, RoomCode, String, String, mpsc::Sender<RoomCommand>) {
        let id = RoomId::new();
        let code = id.to_room_code();
        let join_code = format!("{:04}", rand::random::<u16>() % 10000);
        let host_token = format!("host_tok_{}", rand::random::<u64>());

        let (tx, rx) = mpsc::channel(100);

        let actor = RoomActor::new(
            id,
            join_code.clone(),
            host_token.clone(),
            config,
            resources,
            rx,
        );

        // Register before spawning so the cleanup task always finds the entries.
        self.rooms.insert(id, tx.clone());
        self.codes.insert(code.clone(), id);

        let handle = tokio::spawn(actor.run());
        let registry = self.clone();
        tokio::spawn(async move {
            let _ = handle.await;
            registry.remove(id);
        });

        info!(%id, %code, "registered and spawned new room actor");
        (id, code, join_code, host_token, tx)
    }

    /// Look up a room ID by its shareable 6-character room code.
    pub fn get_id_by_code(&self, code: &RoomCode) -> Option<RoomId> {
        self.codes.get(code).map(|r| *r)
    }

    /// Send a command to a room actor's mailbox by `RoomId`.
    pub async fn route(&self, id: RoomId, cmd: RoomCommand) -> Result<(), String> {
        let sender = match self.rooms.get(&id) {
            Some(s) => s.clone(),
            None => return Err("room_not_found".to_string()),
        };

        sender
            .send(cmd)
            .await
            .map_err(|_| "room_actor_closed".to_string())
    }

    /// Send a command to a room actor's mailbox by `RoomCode`.
    pub async fn route_by_code(&self, code: &RoomCode, cmd: RoomCommand) -> Result<(), String> {
        let id = match self.get_id_by_code(code) {
            Some(id) => id,
            None => return Err("room_not_found".to_string()),
        };

        self.route(id, cmd).await
    }

    /// Remove a room entry from the registry (called on room teardown).
    pub fn remove(&self, id: RoomId) {
        if let Some((_, sender)) = self.rooms.remove(&id) {
            drop(sender);
        }
        self.codes.retain(|_, v| *v != id);
        debug!(%id, "removed room from registry");
    }

    /// Current count of active rooms.
    pub fn len(&self) -> usize {
        self.rooms.len()
    }
    #[allow(unused)]
    pub fn is_empty(&self) -> bool {
        self.rooms.is_empty()
    }
}
impl Default for RoomRegistry {
    fn default() -> Self {
        Self::new()
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn create_and_route_to_room() {
        let registry = Arc::new(RoomRegistry::new());
        let config = GameConfig::default();
        let resources = Arc::new(ResourceManager::new(&config));

        let (id, code, _join_code, _host_token, _tx) =
            registry.clone().create_room(config, resources);
        assert_eq!(registry.len(), 1);

        assert_eq!(registry.get_id_by_code(&code), Some(id));

        let ping_res = registry
            .route(
                id,
                RoomCommand::Ping {
                    player_id: crate::ids::PlayerId(1),
                },
            )
            .await;
        assert!(ping_res.is_ok());
    }

    #[tokio::test]
    async fn room_is_removed_when_actor_terminates() {
        let registry = Arc::new(RoomRegistry::new());
        let config = GameConfig::default();
        let resources = Arc::new(ResourceManager::new(&config));

        let (id, code, _join_code, _host_token, _tx) =
            registry.clone().create_room(config, resources);
        assert_eq!(registry.len(), 1);

        // Terminate the room by leaving the last player.
        let player_id = crate::ids::PlayerId(999);
        let _ = registry
            .route_by_code(
                &code,
                RoomCommand::Join {
                    player_id,
                    name: "Solo".to_string(),
                    join_code: None,
                    response_tx: None,
                },
            )
            .await;
        let _ = registry.route(id, RoomCommand::Leave { player_id }).await;

        // Give the cleanup task a moment to observe actor termination.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        assert_eq!(
            registry.len(),
            0,
            "terminated room should be removed from registry"
        );
        assert_eq!(registry.get_id_by_code(&code), None);
    }
}
