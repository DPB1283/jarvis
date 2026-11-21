// A small chess engine on top of chess.js (which supplies the rules): material + piece-square evaluation,
// negamax with alpha-beta pruning, capture-first move ordering, quiescence on the higher levels, and an
// iterative-deepening time limit. Five levels, from "beginner" (sometimes blunders) to "expert".
import { Chess } from "./chess.js";

const VALUE = { p: 100, n: 320, b: 330, r: 500, q: 900, k: 0 };
const MATE = 100000;

// Piece-square tables from White's point of view, listed from rank 8 down to rank 1 (the order of chess.board()).
const PST = {
  p: [0, 0, 0, 0, 0, 0, 0, 0, 50, 50, 50, 50, 50, 50, 50, 50, 10, 10, 20, 30, 30, 20, 10, 10, 5, 5, 10, 25, 25, 10, 5, 5, 0, 0, 0, 20, 20, 0, 0, 0, 5, -5, -10, 0, 0, -10, -5, 5, 5, 10, 10, -20, -20, 10, 10, 5, 0, 0, 0, 0, 0, 0, 0, 0],
  n: [-50, -40, -30, -30, -30, -30, -40, -50, -40, -20, 0, 0, 0, 0, -20, -40, -30, 0, 10, 15, 15, 10, 0, -30, -30, 5, 15, 20, 20, 15, 5, -30, -30, 0, 15, 20, 20, 15, 0, -30, -30, 5, 10, 15, 15, 10, 5, -30, -40, -20, 0, 5, 5, 0, -20, -40, -50, -40, -30, -30, -30, -30, -40, -50],
  b: [-20, -10, -10, -10, -10, -10, -10, -20, -10, 0, 0, 0, 0, 0, 0, -10, -10, 0, 5, 10, 10, 5, 0, -10, -10, 5, 5, 10, 10, 5, 5, -10, -10, 0, 10, 10, 10, 10, 0, -10, -10, 10, 10, 10, 10, 10, 10, -10, -10, 5, 0, 0, 0, 0, 5, -10, -20, -10, -10, -10, -10, -10, -10, -20],
  r: [0, 0, 0, 0, 0, 0, 0, 0, 5, 10, 10, 10, 10, 10, 10, 5, -5, 0, 0, 0, 0, 0, 0, -5, -5, 0, 0, 0, 0, 0, 0, -5, -5, 0, 0, 0, 0, 0, 0, -5, -5, 0, 0, 0, 0, 0, 0, -5, -5, 0, 0, 0, 0, 0, 0, -5, 0, 0, 0, 5, 5, 0, 0, 0],
  q: [-20, -10, -10, -5, -5, -10, -10, -20, -10, 0, 0, 0, 0, 0, 0, -10, -10, 0, 5, 5, 5, 5, 0, -10, -5, 0, 5, 5, 5, 5, 0, -5, 0, 0, 5, 5, 5, 5, 0, -5, -10, 5, 5, 5, 5, 5, 0, -10, -10, 0, 5, 0, 0, 0, 0, -10, -20, -10, -10, -5, -5, -10, -10, -20],
  k: [-30, -40, -40, -50, -50, -40, -40, -30, -30, -40, -40, -50, -50, -40, -40, -30, -30, -40, -40, -50, -50, -40, -40, -30, -30, -40, -40, -50, -50, -40, -40, -30, -20, -30, -30, -40, -40, -30, -30, -20, -10, -20, -20, -20, -20, -20, -20, -10, 20, 20, 0, 0, 0, 0, 20, 20, 20, 30, 10, 0, 0, 10, 30, 20],
};

export const LEVELS = {
  1: { name: "beginner", depth: 1, noise: 240, quiesce: false, timeMs: 400 },
  2: { name: "easy", depth: 2, noise: 90, quiesce: false, timeMs: 800 },
  3: { name: "medium", depth: 3, noise: 20, quiesce: false, timeMs: 1800 },
  4: { name: "hard", depth: 5, noise: 0, quiesce: true, timeMs: 3000 },
  5: { name: "expert", depth: 8, noise: 0, quiesce: true, timeMs: 6000 },
};

/** Static evaluation in centipawns from the point of view of the side to move. */
export function evaluate(chess) {
  let score = 0;
  const board = chess.board();
  for (let r = 0; r < 8; r++) {
    for (let c = 0; c < 8; c++) {
      const piece = board[r][c];
      if (!piece) continue;
      if (piece.color === "w") score += VALUE[piece.type] + PST[piece.type][r * 8 + c];
      else score -= VALUE[piece.type] + PST[piece.type][(7 - r) * 8 + c];
    }
  }
  return chess.turn() === "w" ? score : -score;
}

function orderMoves(moves) {
  return moves
    .map(m => ({ m, key: (m.captured ? 10 * VALUE[m.captured] - VALUE[m.piece] : 0) + (m.promotion ? 800 : 0) + (m.san.includes("+") ? 50 : 0) }))
    .sort((a, b) => b.key - a.key)
    .map(x => x.m);
}

class Timeout extends Error {}

function play(chess, m) {
  chess.move({ from: m.from, to: m.to, promotion: m.promotion });
}

export function chooseMove(fen, level = 3, options = {}) {
  const config = LEVELS[Math.min(5, Math.max(1, Math.round(level)))];
  const chess = new Chess(fen);
  const deadline = performance.now() + (options.timeMs ?? config.timeMs);
  let nodes = 0;
  const tick = () => {
    if ((++nodes & 511) === 0 && performance.now() > deadline) throw new Timeout();
  };

  function quiesce(alpha, beta, depth) {
    tick();
    const stand = evaluate(chess);
    if (depth === 0 || stand >= beta) return stand;
    alpha = Math.max(alpha, stand);
    for (const m of orderMoves(chess.moves({ verbose: true }).filter(x => x.captured))) {
      play(chess, m);
      const score = -quiesce(-beta, -alpha, depth - 1);
      chess.undo();
      if (score >= beta) return score;
      alpha = Math.max(alpha, score);
    }
    return alpha;
  }

  function search(depth, alpha, beta, ply) {
    tick();
    const moves = chess.moves({ verbose: true });
    if (moves.length === 0) return chess.isCheckmate() ? -MATE + ply : 0;
    if (chess.isInsufficientMaterial() || chess.isThreefoldRepetition() || chess.isDrawByFiftyMoves()) return 0;
    if (depth === 0) return config.quiesce ? quiesce(alpha, beta, 4) : evaluate(chess);
    let best = -Infinity;
    for (const m of orderMoves(moves)) {
      play(chess, m);
      const score = -search(depth - 1, -beta, -alpha, ply + 1);
      chess.undo();
      if (score > best) best = score;
      alpha = Math.max(alpha, score);
      if (alpha >= beta) break;
    }
    return best;
  }

  const rootMoves = orderMoves(chess.moves({ verbose: true }));
  if (rootMoves.length === 0) return null;
  let chosen = rootMoves[0];
  let chosenScore = 0;
  let reached = 0;

  // Iterative deepening: keep the best move from the last depth that finished in time.
  for (let depth = 1; depth <= config.depth; depth++) {
    let bestMove = null;
    let bestScore = -Infinity;
    try {
      let alpha = -Infinity;
      for (const m of rootMoves) {
        play(chess, m);
        // Low levels need comparable scores for every move so noise can make them miss things.
        const score = -search(depth - 1, -Infinity, config.noise ? Infinity : -alpha, 1) + (config.noise ? Math.random() * config.noise : 0);
        chess.undo();
        if (score > bestScore) {
          bestScore = score;
          bestMove = m;
        }
        alpha = Math.max(alpha, score);
      }
    } catch (error) {
      if (!(error instanceof Timeout)) throw error;
      // The search was cut off mid-line: start from the position again and keep the previous depth's answer.
      chess.load(fen);
      break;
    }
    chosen = bestMove;
    chosenScore = bestScore;
    reached = depth;
    if (Math.abs(bestScore) > MATE / 2) break; // a forced mate is found
  }
  return { from: chosen.from, to: chosen.to, promotion: chosen.promotion, san: chosen.san, score: Math.round(chosenScore), depth: reached };
}
