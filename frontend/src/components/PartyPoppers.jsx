import { useEffect } from "react";
import confetti from "canvas-confetti";

function PartyPoppers() {
  useEffect(() => {
    // Left popper
    confetti({
      particleCount: 120,
      angle: 60,
      spread: 60,
      origin: { x: 0, y: 1 },
    });

    // Right popper
    confetti({
      particleCount: 120,
      angle: 120,
      spread: 60,
      origin: { x: 1, y: 1 },
    });
  }, []);

  return null;
}

export default PartyPoppers;