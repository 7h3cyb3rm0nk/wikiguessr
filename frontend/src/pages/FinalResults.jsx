import "../styles/FinalResults.css";
import PartyPoppers from "../components/PartyPoppers";

function FinalResults() {
  return (
    <div className="results-page">

          <PartyPoppers />
          
      <div className="results-card">

        <h1 className="result-title">MATCH COMPLETE</h1>

        {/* Total Score Card */}
        <div className="score-card">
          <span className="score-label">TOTAL SCORE</span>
          <h2 className="score-value">18,720</h2>
        </div>

        {/* Stats Cards */}
        <div className="stats-grid">

          <div className="stat-card">
            <span>ROUNDS</span>
            <h3>5</h3>
          </div>

          <div className="stat-card">
            <span>AVERAGE</span>
            <h3>3744</h3>
          </div>

          <div className="stat-card">
            <span>BEST ROUND</span>
            <h3>4980</h3>
          </div>

        </div>

        {/* Buttons */}
        <div className="buttons">
          <button className="primary-btn">
            Play Again
          </button>

          <button className="secondary-btn">
            Return Home
          </button>
        </div>

      </div>
    </div>
  );
}

export default FinalResults;