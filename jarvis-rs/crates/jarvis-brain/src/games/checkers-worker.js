import { chooseMove } from "./checkers-engine.js";

self.onmessage = ({ data: { revision, kind, board, turn, level } }) => {
  self.postMessage({ revision, kind, move: chooseMove(board, turn, level) });
};
