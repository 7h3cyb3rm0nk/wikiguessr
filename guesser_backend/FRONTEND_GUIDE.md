# WikiGuessr Frontend Integration & Design Guide

This guide details how to build or connect a modern frontend to the **WikiGuessr Backend**. It covers API specifications for both **Single-Player** (REST) and **Multiplayer** (REST + WebSocket), along with UI/UX design recommendations.

---

## 1. System Architecture & Flow

```
                      ┌────────────────────────┐
                      │    Frontend Client     │
                      │ (HTML5/CSS3/Leaflet/JS)│
                      └───────────┬────────────┘
                                  │
                 ┌────────────────┴────────────────┐
                 │                                 │
           REST Requests                     WebSocket Stream
         (Start, Join, Guess)               (Events, Sync, Broadcasts)
                 │                                 │
                 ▼                                 ▼
      ┌─────────────────────┐           ┌─────────────────────┐
      │  Axum REST Router   │           │  WebSocket Handler  │
      └──────────┬──────────┘           └──────────┬──────────┘
                 │                                 │
                 └────────────────┬────────────────┘
                                  │
                                  ▼
                       ┌─────────────────────┐
                       │   RoomActor Task    │
                       │(State Machine Engine)│
                       └─────────────────────┘
```

The backend is **fully server-authoritative**:
- The server determines locations, image selection, round deadlines, and scoring.
- The frontend renders images, interactive maps, timers, and sends user actions (`ready`, `guess`).
- Images are served directly from **Wikimedia Commons CDN** URLs provided by the API.

---

## 2. Single-Player Integration Spec (REST Only)

Single-player mode requires no WebSockets. It uses simple HTTP `POST` requests.

### 2.1 Start Single-Player Game

- **Endpoint**: `POST /api/solo/start`
- **Request Body**: None (empty body)
- **Response**: `200 OK`

```json
{
  "room_code": "X7K2M3",
  "player_id": 1042,
  "player_token": "p_tok_1042",
  "total_rounds": 5,
  "round_duration_secs": 60
}
```

> **Client Action**: Store `room_code` and `player_id` in local session state. Render round 1 immediately.

---

### 2.2 Submit Single-Player Guess

- **Endpoint**: `POST /api/solo/guess`
- **Request Body**:

```json
{
  "room_code": "X7K2M3",
  "player_id": 1042,
  "latitude": 48.8566,
  "longitude": 2.3522
}
```

- **Response**: `200 OK`

```json
{
  "score": 4850,
  "status": "success"
}
```

---

## 3. Multiplayer Integration Spec (REST + WebSocket)

Multiplayer uses REST endpoints to create/join lobbies and WebSocket for real-time round sync.

### 3.1 Create Room (Host)

- **Endpoint**: `POST /api/rooms`
- **Request Body**: None (or `{ "name": "HostName" }`)
- **Response**: `200 OK`

```json
{
  "room_id": 182947192847,
  "room_code": "K9M2P4",
  "join_code": "4812",
  "host_token": "host_tok_91823719"
}
```

---

### 3.2 Join Room (Guest)

- **Endpoint**: `POST /api/rooms/:code/join`
- **URL Path**: `:code` = room code (e.g. `K9M2P4`)
- **Request Body**:

```json
{
  "name": "Alex",
  "join_code": "4812"
}
```

- **Response**: `200 OK`

```json
{
  "player_id": 2045,
  "player_token": "p_tok_2045"
}
```

---

### 3.3 Connect to Room WebSocket

- **WebSocket URL**: `ws://HOST:PORT/api/ws?room_code=K9M2P4&player_id=2045`

#### A. Client → Server Messages (`ClientWsMessage`)

Send JSON stringified text frames:

1. **Mark Ready (Start / Next Round)**:
   ```json
   { "type": "ready" }
   ```

2. **Submit Guess**:
   ```json
   {
     "type": "guess",
     "seq": 1,
     "latitude": 51.5074,
     "longitude": -0.1278
   }
   ```

3. **Ping (Keepalive)**:
   ```json
   { "type": "ping" }
   ```

4. **Leave Room**:
   ```json
   { "type": "leave" }
   ```

---

#### B. Server → Client Broadcast Events (`RoomEvent`)

The server emits tagged JSON messages:

1. **`player_joined`**:
   ```json
   {
     "type": "player_joined",
     "player_id": 2045,
     "name": "Alex"
   }
   ```

2. **`player_ready`**:
   ```json
   {
     "type": "player_ready",
     "player_id": 2045
   }
   ```

3. **`round_started`**:
   ```json
   {
     "type": "round_started",
     "round": 1,
     "total_rounds": 5,
     "images": [
       {
         "url": "https://upload.wikimedia.org/.../image.jpg",
         "thumb_url": "https://upload.wikimedia.org/.../800px-image.jpg",
         "width": 1920,
         "height": 1080
       }
     ],
     "deadline_unix_ms": 1785189200000
   }
   ```

4. **`guess_ack`**:
   ```json
   {
     "type": "guess_ack",
     "player_id": 2045,
     "seq": 1
   }
   ```

5. **`round_ended`**:
   ```json
   {
     "type": "round_ended",
     "round": 1,
     "scores": [
       { "player_id": 2045, "score": 4850, "distance_meters": 1420.5 }
     ],
     "actual_location": { "latitude": 48.8566, "longitude": 2.3522 },
     "item_id": "Q42"
   }
   ```

6. **`leaderboard`**:
   ```json
   {
     "type": "leaderboard",
     "standings": [
       { "player_id": 2045, "name": "Alex", "score": 4850 }
     ]
   }
   ```

7. **`game_finished`**:
   ```json
   {
     "type": "game_finished",
     "final_standings": [
       { "player_id": 2045, "name": "Alex", "score": 21400 }
     ]
   }
   ```

---

## 4. Frontend UI/UX Design System

To match modern web standards, follow these visual design principles:

### 4.1 Color Palette & Theme (Dark Glassmorphism)

| Token | Hex / HSL | Usage |
|---|---|---|
| `--bg-primary` | `#0f172a` (Slate 900) | Application background |
| `--bg-surface` | `rgba(30, 41, 59, 0.7)` | Glass card containers |
| `--accent-emerald` | `#10b981` | Correct location, high score |
| `--accent-amber` | `#f59e0b` | Round timer warning, medium score |
| `--accent-rose` | `#f43f5e` | Low score, missed location line |
| `--accent-cyan` | `#06b6d4` | Interactive buttons, selected pin |

### 4.2 Key Components & Features

1. **Split-Screen Layout**:
   - **Left / Top Half**: Wikimedia Commons photo carousel/gallery with full-screen zoom modal.
   - **Right / Bottom Half**: Interactive **Leaflet.js** or **Mapbox** map with OSM tiles.

2. **Map Markers & Result Polyline**:
   - Player click places a custom cyan pin.
   - On round end, draw a dashed red polyline connecting player's guess to the actual location pin (`actual_location`).
   - Automatically zoom map bounds to fit both pins (`map.fitBounds([guess, actual])`).

3. **Round Timer Bar**:
   - Calculate remaining time: `deadline_unix_ms - Date.now()`.
   - Animate a linear CSS progress bar reducing from 100% to 0%.

4. **Gallery vs Slideshow View**:
   - Provide a toggle for players to switch between a multi-image thumbnail grid and an auto-advancing slideshow.

---

## 5. Health & Diagnostic API

- **Endpoint**: `GET /api/health`
- **Response**: `200 OK`

```json
{
  "status": "ok",
  "active_rooms": 12,
  "pool_size": 940
}
```
