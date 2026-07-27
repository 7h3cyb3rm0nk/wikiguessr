use serde::{Deserialize, Serialize};
 
use super::location::ItemId;
 
/// A single Commons image associated with a Location.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Image {
    pub url: String,
    pub width: u32,
    pub height: u32,
}
 
/// All images fetched for one location, keyed by the location's
/// ItemId in ImageCache. Empty sets are valid (some locations have
/// no nearby Commons images) — callers must handle that, not treat
/// it as a cache miss.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageSet {
    pub item_id: ItemId,
    pub images: Vec<Image>,
}
 

