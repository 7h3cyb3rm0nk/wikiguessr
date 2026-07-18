# WikiGuessr Multiplayer Backend Design Specification (v2)

## 1. Overview

Authoritative multiplayer backend for WikiGuessr on Wikimedia Toolforge. Clients are untrusted: they render UI and send actions; they never compute game state.

Goals: multiplayer support, deterministic gameplay, low Wikimedia API usage, fault isolation per room, testability.

Changes from v1: replaced shared-map-plus-lock room model with actor-per-room (removes lock-across-await hazard), added explicit WS room routing, fixed location pool concurrency primitive, specified scoring formula, added guess idempotency, added timer cancellation on room teardown, added cache TTL jitter, made auth posture explicit instead of silently absent.

---

## 2. High-Level Architecture

```
                        Browser
                 (Leaflet + Frontend)
                         │
              REST + WebSocket (per-connection)
                         │
                 ┌────────────────┐
                 │  Network Layer │   (routing, (de)serialization, ws framing)
                 └────────────────┘
                         │
                 ┌────────────────┐
                 │  RoomRegistry  │   (RoomId -> mpsc::Sender<RoomCommand>)
                 └────────────────┘
                         │
              spawns / addresses
                         │
              ┌──────────────────────┐
              │   Room Actor (task)   │  x N, one per active room
              │   owns: state, timers │
              └──────────────────────┘
                         │
                 ┌────────────────┐
                 │ ResourceManager│   (single shared instance, Arc)
                 └────────────────┘
                    │           │
              LocationPool   ImageCache
                    │           │
              WikidataClient CommonsClient
```

Key structural change: there is no `GameEngine` object that reaches into a shared `DashMap<RoomId, Room>` under lock. Each room is an independent Tokio task (an actor) that owns its own state exclusively. Concurrency between rooms is achieved by having _N_ independent tasks, not by locking one shared structure. This eliminates the lock-held-across-await class of bugs and gives natural fault isolation: a panic or hang in one room's actor cannot block another room.

---

## 3. Design Principles

**Rule 1 — Clients never decide game state.** Server decides round progression, timers, scoring, winners.

**Rule 2 — Only `ResourceManager` talks to Wikimedia.** No other component performs HTTP requests.

**Rule 3 — Room actors own their state exclusively.** No shared mutable room state exists outside the actor's task. Communication is by message-passing (`mpsc` channel), not by shared lock.

**Rule 4 — Each room actor owns its rules for that room's lifecycle.** There is no separate "GameEngine" god-object; game logic lives in `room::actor`, parameterized by a shared, stateless `scoring` and `rules` module so logic isn't duplicated per-instance.

**Rule 5 — Network layer performs zero business logic.** It only does:

```
JSON → Rust command type → RoomRegistry::route(room_id, command)
                                         ↓
                              room actor mailbox (mpsc)
                                         ↓
                              actor processes, emits event
                                         ↓
JSON ← Rust event type ← broadcast to room's connected sockets
```

---

## 4. Project Structure

```
src/
  main.rs
  config.rs
  state.rs                 // AppState: RoomRegistry + ResourceManager (both Arc)

  api/
    http.rs
    websocket.rs            // handshake, room-binding, frame loop
    request.rs
    response.rs

  room/
    registry.rs             // RoomId -> mpsc::Sender<RoomCommand>, create/lookup/destroy
    actor.rs                // per-room task: state machine + timers
    state.rs                // Room, Player, RoundState (owned only by actor)
    command.rs               // RoomCommand enum (input to actor)
    event.rs                  // RoomEvent enum (output, broadcast to clients)
    scoring.rs                // pure fn: (guess, actual) -> Score
    rules.rs                  // pure fns: round transitions, win conditions

  resources/
    manager.rs
    wikidata.rs
    commons.rs
    cache.rs                  // ImageCache with jittered TTL
    pool.rs                   // LocationPool, Mutex<VecDeque<_>> + refill worker

  models/
    location.rs
    image.rs

  util/
```

---

## 5. Network Layer

Responsibilities: HTTP routing, WebSocket upgrade + room binding, serialization, per-room broadcast fan-out. No game logic — it only converts JSON to commands and events to JSON.

### HTTP Endpoints

```
POST   /api/rooms                 -> create room, returns {room_id, host_token}
GET    /api/rooms/{id}            -> room metadata (player count, status), no secrets
POST   /api/rooms/{id}/join       -> validates join_code, returns {player_token}
GET    /api/health
```

### WebSocket

```
GET /api/ws?room_id={id}&token={player_token}
```

The room binding happens **at handshake**, not per-message. `token` is validated against the room actor before the upgrade completes (issued by `POST /join`, single-use, scoped to one room). This closes the v1 gap where any `room_id` could join or guess with no credential — see §14 for full auth posture.

Once upgraded, the connection is registered as a `Sink<RoomEvent>` with exactly one room actor. Because the room is bound at handshake, individual client messages (`Guess`, `Ready`, etc.) do **not** need to carry `room_id`; the network layer already knows which room's mailbox to forward to. This fixes the v1 protocol gap where `Guess` had no room-routing information.

---

## 6. WebSocket Protocol

### Client → Server (`RoomCommand`)

```json
{ "type": "ready" }
{ "type": "guess", "seq": 3, "latitude": 12.5, "longitude": 75.4 }
{ "type": "leave" }
{ "type": "ping" }
```

`seq` is a per-player monotonically increasing counter set by the client and echoed back. It is **not** trusted for ordering (the actor's mailbox already gives FIFO order per-connection) — it exists purely for guess idempotency (§8).

### Server → Client (`RoomEvent`)

```
RoomCreated
PlayerJoined { player_id, name }
PlayerLeft   { player_id }
RoundStarted { round, image_ids, deadline_unix_ms }
PlayerReady  { player_id }
GuessAck     { player_id, seq }        // NEW: confirms receipt, decoupled from result
RoundEnded   { scores: [...], actual_location }
Leaderboard  { standings: [...] }
GameFinished { final_standings: [...] }
Error        { code, message }
```

`GuessAck` is new versus v1: guesses are scored only at round end (server-authoritative, prevents leaking distance info mid-round via timing side channels), so the client needs an immediate ack distinct from the eventual `RoundEnded` scoring.

---

## 7. Room Actor

Replaces the v1 `GameEngine` + `RoomManager` pair. One Tokio task per room, spawned by `RoomRegistry::create_room()`, torn down on game end or inactivity timeout.

```rust
async fn run(mut self, mut mailbox: mpsc::Receiver<RoomCommand>) {
    loop {
        tokio::select! {
            Some(cmd) = mailbox.recv() => self.handle_command(cmd).await,
            _ = self.round_timer.tick(), if self.round_timer.is_armed() => self.end_round().await,
            _ = self.inactivity_timer.tick() => { self.teardown().await; return; }
        }
    }
}
```

Because state is owned by the task and only ever touched from within `run`, there is no lock, no `Arc<Mutex<Room>>`, and therefore no possibility of holding a lock across an `.await` on a Wikimedia call. When the actor needs a new location, it calls `resource_manager.next_location().await` — this is a normal async call inside the actor's own task; it blocks nothing but this one room's progress, and other rooms are unaffected since they're separate tasks.

### Public interface (via `RoomCommand`, not direct method calls)

```
CreateRoom (via registry, not actor)
Join(player_id, sender)
Leave(player_id)
Ready(player_id)
Guess(player_id, seq, lat, lon)
```

`start_game`, `next_round`, `finish_game` are internal actor state transitions, not externally invokable — this matches Rule 1 (clients don't decide progression) more strictly than v1, which exposed them as public `GameEngine` methods callable from... somewhere unspecified.

---

## 8. Guess Idempotency

v1 gap: no defined behavior for a duplicate `submit_guess`. Fixed as follows:

- Each player has one guess slot per round, keyed by round number, stored in the actor's in-memory `RoundState`.
- A `Guess` command with the same `(player_id, round)` as an already-recorded guess is **rejected** with `Error{code: "already_guessed"}` — first guess wins, no overwrite.
- `seq` from the client is echoed in `GuessAck` so the client can deduplicate its own retries (e.g. distinguish "my retry got a fresh ack" from "server never saw the first attempt") without the server needing to track client-side retry state.

---

## 9. Room Registry

Purpose: create/destroy room actors, route commands to the right mailbox. Holds **no game state** — it's a directory, not a store.

```rust
struct RoomRegistry {
    rooms: DashMap<RoomId, mpsc::Sender<RoomCommand>>,
}
```

`DashMap` is appropriate _here_ specifically because the value is a cheap `Sender` handle, not the room state itself — no lock is ever held across an await; `clone()` of a sender is O(1) and non-blocking, so the critical section is trivially short.

```
create(room_id) -> spawns actor, inserts sender
route(room_id, cmd) -> looks up sender, cmd.send().await   // await happens *after* map access, on the owned clone
remove(room_id)   -> called by actor on self-teardown, removes entry
```

---

## 10. Resource Manager

Single shared `Arc<ResourceManager>`, only component permitted to perform Wikimedia HTTP requests.

```
next_location()          -> pop from LocationPool, non-blocking under normal load
images_for_location(id)  -> ImageCache lookup, fetch from Commons on miss
refill_pool()             -> background worker only
refresh_cache()            -> background worker only
```

---

## 11. Global Location Pool

Single pool, shared across all rooms — not per-room.

```
Background worker refills:  1000 target / 300 refill threshold / 200 batch size
```

**Fix vs v1:** the pool was specified as `RwLock<VecDeque<Location>>`. `next_location()` pops from the front — a mutation — so every consumer takes the _write_ lock; there is no concurrent-read path that would justify `RwLock` over a plain `Mutex`. `RwLock` write acquisition is typically more expensive than `Mutex` lock in the uncontended case on common implementations, so this was strictly worse than a plain mutex for zero benefit.

```rust
struct LocationPool {
    inner: Mutex<VecDeque<Location>>,
}
```

Alternative (preferred if pool throughput becomes a bottleneck): bounded MPMC channel (`crossbeam::channel` or `tokio::sync::mpsc` with a single consumer per pop, refill worker as producer) — avoids lock semantics entirely and makes backpressure (pool empty → room await) explicit rather than a spin/retry loop.

---

## 12. Image Cache

```rust
DashMap<ItemId, ImageSet>
```

**Fix vs v1:** fixed 24h TTL for every entry causes correlated expiry (thundering herd on Commons when a burst of entries populated around the same time all expire together). Add jitter:

```
ttl = base_ttl(24h) + uniform_random(-2h, +2h)
```

Flow unchanged: cache hit returns immediately; miss fetches from Commons, stores with jittered TTL, returns.

---

## 13. Wikimedia Clients

Unchanged from v1 — two independent clients, single responsibility each:

- **Wikidata client**: fetches locations only, never images.
- **Commons client**: fetches images for a given location only, never locations.

---

## 14. Authentication & Room Access

v1 left this as "future work" with no stated interim posture — meaning the _actual_ interim behavior (anyone with a `room_id` can join and guess) was an unstated default, not a decision. Made explicit:

- `POST /api/rooms/{id}/join` requires a `join_code` (short, room-scoped, shown only to players the host invites out-of-band). Returns a single-use `player_token`.
- `player_token` is required at WS handshake (§5) and is scoped to exactly one `room_id`; it is not valid for any other room and expires when the room is destroyed.
- The host additionally receives a `host_token` at room creation, required for host-only actions (force-start, kick).
- This is _not_ user authentication (no account system) — it is room-access control, sufficient to prevent unauthenticated strangers from joining or submitting guesses to a room they weren't invited to. Full account-based auth remains future work per §18.

---

## 15. Timers and Lifecycle

Each room actor owns its timers as plain values inside its own task (`tokio::time::Interval` / `Sleep`), not as separately spawned tasks reachable from outside.

**Fix vs v1:** v1 specified "Tokio tasks drive timers" with no mention of cancellation, which leaks a task per room if `RoomManager.remove()` doesn't explicitly `.abort()` the handle. Since timers here live inside the actor's own `select!` loop (§7) rather than as detached spawned tasks, there is nothing to separately cancel — when the actor's `run()` returns (game end or inactivity timeout), the timer state is dropped along with everything else in the task. No separate cleanup step, no leak path.

```
Lobby timer     -> room destroyed if no Ready within N seconds of creation
Round timer     -> triggers end_round() on expiry
Inactivity timer -> resets on any command; destroys room if idle too long
```

---

## 16. Scoring

```
distance = haversine(guess, actual)   // meters
score    = round(5000 * exp(-distance / 2000))   // exponential decay, tunable constant
score    = clamp(score, 0, 5000)
```

Explicit formula (v1 left this as "distance → Haversine → score" with no function specified). The decay constant (2000 m half-scale) and max score (5000) are configuration, not hardcoded — exposed via `config.rs` so game balance can be tuned without a code change. Computed only inside the room actor at round end; never sent to or trusted from the client.

---

## 17. Concurrency Summary

```
Room state         -> owned exclusively by its actor task, no lock
Room registry       -> DashMap<RoomId, Sender>  (cheap-value map, safe)
Location pool        -> Mutex<VecDeque<Location>>  (or MPMC channel)
Image cache            -> DashMap<ItemId, ImageSet>, jittered TTL
ResourceManager         -> Arc, shared, internally synchronized per above
```

No component holds a lock across an `.await` boundary on an external HTTP call. The only cross-task synchronization primitives are: (a) `mpsc` channels into room actors, (b) the registry's `DashMap` of cheap sender handles, (c) `Mutex` on the location pool for a bounded, non-blocking pop/push, (d) `DashMap` on the image cache for independent per-key hits.

---

## 18. Room Lifecycle

```
POST /rooms          -> registry spawns actor, actor state = Lobby
join_code distributed -> players call POST /join, get player_token
WS connect w/ token   -> actor state = Lobby, player registered
Ready x N             -> actor state = InRound(1)
Guess (x N, idempotent per §8)
Round timer expiry OR all guessed -> scoring, RoundEnded, Leaderboard
... repeat for N rounds (default 5, configurable) ...
Final round ends       -> GameFinished, actor enters Draining
Draining               -> broadcast flushed, sockets closed, actor returns, registry entry removed
```

---

## 19. Frontend Responsibilities

Unchanged from v1: the frontend renders the map and images, sends `Ready`/`Guess`/`Leave`, displays timers and leaderboard client-side for UX only. It never fetches Wikimedia directly, computes score, advances rounds, or decides winners — all of that is server-authoritative per Rule 1.

---

## 20. Error Handling

External requests (Wikidata, Commons) classify as:

```
Success
Retryable Failure   -> retry with exponential backoff, max 3 attempts, 10s timeout per attempt
Permanent Failure    -> fall back to cached location/image if available, else surface Error event to affected room only
```

A permanent failure in `ResourceManager` degrades only the room(s) currently awaiting that resource — it cannot cascade into other rooms since each room actor awaits independently.

---

## 21. Logging

Structured logging via `tracing`, with `request_id`, `room_id`, `player_id` as span fields wherever available. Since each room is its own task, a `tracing::Span` can be attached to the actor's `run()` future once at spawn time, and every event emitted inside that task automatically inherits `room_id` — no manual threading of the id through every log call.

Key events: room created, player joined/left, round started, guess accepted/rejected (incl. `already_guessed`), round ended, game finished, Wikimedia request failure (with classification from §20).

---

## 22. Future Extensions

Full account-based authentication (beyond the room-scoped tokens in §14), ranked matchmaking, daily challenges, persistent leaderboards, spectator mode (read-only actor subscriber, no `RoomCommand` access), AI opponents, tournament mode, alternative location providers, distributed room registry (for horizontal scaling — would require moving `RoomRegistry` from in-process `DashMap` to a distributed directory, e.g. backed by Redis, with room actors pinned to a specific worker node), mobile clients.

The actor-per-room model in particular makes horizontal scaling more tractable than the v1 shared-map design: since each room is already isolated to a single task with no cross-room shared mutable state (only the stateless `ResourceManager`), sharding rooms across multiple processes/nodes is a routing problem at the registry layer, not a rearchitecture of the game logic.

---

## 23. Dependency Graph

```
                   Browser
                       │
              REST / WebSocket (room-bound at handshake)
                       │
               Network Layer
                       │
               Room Registry  ──routes to──>  Room Actor (x N, isolated tasks)
                                                     │
                                            Resource Manager (Arc, shared)
                                                     │
                                       Wikidata Client + Commons Client
                                                     │
                                          Wikidata + Commons APIs
```

Strictly one-directional: Network Layer → Registry → Actor → ResourceManager → Wikimedia clients. No lower layer calls upward. The key structural difference from v1 is that "Room Actor" replaces "Game Engine + Room Manager" as two peers sharing a lock — actors are independent, addressable-by-message units, which is what actually gives the fault isolation and lock-free concurrency the v1 diagram claimed but didn't structurally guarantee.
