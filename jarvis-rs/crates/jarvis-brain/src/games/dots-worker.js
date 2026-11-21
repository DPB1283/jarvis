import { chooseEdge } from "./dots-engine.js";

self.onmessage = ({ data: { revision, kind, board, level } }) => {
  self.postMessage({ revision, kind, move: chooseEdge(board, level) });
};
