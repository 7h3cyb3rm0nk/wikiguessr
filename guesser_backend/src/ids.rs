use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

/// Alphabet for room codes: A-Z (minus I, O) + 2-9 = 32 characters.
/// Avoids ambiguous characters (0/O, 1/I).
const ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

// ---------------------------------------------------------------------------
// RoomId
// ---------------------------------------------------------------------------

/// Unique room identifier. Generated via random `u64`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RoomId(pub u64);

impl RoomId {
    pub fn new() -> Self {
        Self(rand::random::<u64>())
    }

    /// Derive a 6-character alphanumeric room code from this ID.
    /// Uses the lower 30 bits (32^6 ≈ 1 billion combinations).
    pub fn to_room_code(&self) -> RoomCode {
        let mut chars = [0u8; 6];
        let mut n = self.0;
        for c in &mut chars {
            *c = ALPHABET[(n & 0x1F) as usize];
            n >>= 5;
        }
        // SAFETY: ALPHABET contains only ASCII bytes.
        RoomCode(String::from_utf8(chars.to_vec()).unwrap())
    }
}

impl Default for RoomId {
    fn default() -> Self {
        Self::new()
    }
}
impl fmt::Display for RoomId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ---------------------------------------------------------------------------
// RoomCode
// ---------------------------------------------------------------------------

/// Human-shareable room code (e.g. "X7K2M3").
/// Derived from a `RoomId` — multiple `RoomId`s can collide to the same code,
/// so the `RoomRegistry` maintains a `code → RoomId` lookup.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RoomCode(pub String);

impl RoomCode {
    /// Decode a room code back to its lower 30 bits.
    /// Returns `None` if the code contains invalid characters or wrong length.
    pub fn decode(&self) -> Option<u64> {
        if self.0.len() != 6 {
            return None;
        }
        let mut result: u64 = 0;
        for (i, ch) in self.0.bytes().enumerate() {
            let idx = ALPHABET.iter().position(|&c| c == ch)? as u64;
            result |= idx << (5 * i);
        }
        Some(result)
    }
}

impl fmt::Display for RoomCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ---------------------------------------------------------------------------
// PlayerId
// ---------------------------------------------------------------------------

/// Unique player identifier. Monotonically increasing via `AtomicU64`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PlayerId(pub u64);

static NEXT_PLAYER_ID: AtomicU64 = AtomicU64::new(1);

impl PlayerId {
    pub fn new() -> Self {
        Self(NEXT_PLAYER_ID.fetch_add(1, Ordering::Relaxed))
    }
}

impl Default for PlayerId {
    fn default() -> Self {
        Self::new()
    }
}
impl fmt::Display for PlayerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn room_code_is_six_chars_from_alphabet() {
        let id = RoomId::new();
        let code = id.to_room_code();
        assert_eq!(code.0.len(), 6);
        for ch in code.0.bytes() {
            assert!(ALPHABET.contains(&ch), "unexpected char: {}", ch as char);
        }
    }

    #[test]
    fn room_code_decode_matches_lower_bits() {
        let id = RoomId(0xDEAD_BEEF);
        let code = id.to_room_code();
        let decoded = code.decode().unwrap();
        assert_eq!(decoded, 0xDEAD_BEEF & 0x3FFF_FFFF);
    }

    #[test]
    fn room_code_deterministic_for_same_id() {
        let id = RoomId(42);
        assert_eq!(id.to_room_code(), id.to_room_code());
    }

    #[test]
    fn room_code_invalid_chars_rejected() {
        // 'O' is not in the alphabet
        let bad = RoomCode("OOOOOO".to_string());
        assert!(bad.decode().is_none());
    }

    #[test]
    fn room_code_wrong_length_rejected() {
        assert!(RoomCode("ABC".to_string()).decode().is_none());
        assert!(RoomCode("ABCDEFGH".to_string()).decode().is_none());
    }

    #[test]
    fn player_ids_are_monotonic() {
        let a = PlayerId::new();
        let b = PlayerId::new();
        assert!(b.0 > a.0);
    }

    #[test]
    fn player_ids_are_unique() {
        let ids: Vec<PlayerId> = (0..100).map(|_| PlayerId::new()).collect();
        let unique: std::collections::HashSet<_> = ids.iter().collect();
        assert_eq!(unique.len(), ids.len());
    }
}
