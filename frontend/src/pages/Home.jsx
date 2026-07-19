import { useState } from "react";
import homeBg from "../assets/home-bg.png";
import "../styles/Home.css";

function Home() {
  const [showJoin, setShowJoin] = useState(false);

  return (
    <div
      className="home"
      style={{
        backgroundImage: `url(${homeBg})`,
      }}
    >
      <div className="title">
        <span className="globe">🌍</span>
        <h1>WikiGuessr</h1>
      </div>

      <p className="subtitle">
        Guess locations from Wikipedia images.
      </p>

      {!showJoin && (
        <button className="primary-btn">
           ▶ Play
        </button>
      )}
      
      {!showJoin && (
        <button className="primary-btn">
          Create Room
        </button>
      )}

      {!showJoin && (
        <button
          className="secondary-btn"
          onClick={() => setShowJoin(true)}
        >
          Join Room
        </button>
      )}

      {showJoin && (
        <div className="join-section">

          <input
            className="room-input"
            type="text"
            placeholder="Enter Room Code"
          />

          
          <button className="secondary-btn">
            Join
          </button>

          <button
            className="back-btn"
            onClick={() => setShowJoin(false)}
          >
              ← Back
          </button>
        </div>
        
      )}
    </div>
  );
}

export default Home;