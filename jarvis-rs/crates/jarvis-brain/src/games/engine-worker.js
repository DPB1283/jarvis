// Runs the game engines off the page's main thread so the board stays responsive while Jarvis thinks.
import { chooseMove } from "./engine.js";
import { chooseColumn } from "./c4engine.js";

onmessage = event => {
  const m = event.data;
  try {
    if (m.game === "chess") postMessage({ id: m.id, move: chooseMove(m.fen, m.level, { timeMs: m.timeMs }) });
    else postMessage({ id: m.id, col: chooseColumn(m.board, m.player, m.level) });
  } catch (error) {
    postMessage({ id: m.id, error: String(error) });
  }
};
