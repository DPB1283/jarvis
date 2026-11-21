// Positive pieces move toward row 0. Absolute value 2 denotes a king.
export const YOU = 1;
export const JARVIS = -1;

export function initialBoard() {
  return Array.from({ length: 64 }, (_, square) => {
    const row = square >> 3;
    if ((row + square % 8) % 2 === 0) return 0;
    return row < 3 ? JARVIS : row > 4 ? YOU : 0;
  });
}

export function squareName(square) {
  return String.fromCharCode(97 + square % 8) + (8 - (square >> 3));
}

export function parsePath(text) {
  const separator = /\s*[-x]\s*|\s+(?:to\s+)?/;
  const names = text.trim().toLowerCase().split(separator);
  if (names.length < 2 || names.some(name => !/^[a-h][1-8]$/.test(name))) return null;
  return names.map(name => (8 - Number(name[1])) * 8 + name.charCodeAt(0) - 97);
}

function step(square, dr, dc) {
  const row = (square >> 3) + dr;
  const col = square % 8 + dc;
  return row < 0 || row > 7 || col < 0 || col > 7 ? -1 : row * 8 + col;
}

function crowns(piece, square) {
  return Math.abs(piece) === 1 && (piece > 0 ? square < 8 : square >= 56);
}

function jumps(board, path, captured, piece, result) {
  const from = path.at(-1);
  let canJump = false;
  for (const dr of Math.abs(piece) === 2 ? [-1, 1] : [-Math.sign(piece)]) {
    for (const dc of [-1, 1]) {
      const over = step(from, dr, dc);
      const to = step(from, dr * 2, dc * 2);
      if (over < 0 || to < 0 || piece * board[over] >= 0 || board[to] !== 0) continue;
      canJump = true;
      const nextPath = [...path, to];
      const nextCaptured = [...captured, over];
      // In English draughts, reaching the king row ends the turn.
      if (crowns(piece, to)) {
        result.push({ path: nextPath, captured: nextCaptured });
        continue;
      }
      const enemy = board[over];
      board[from] = board[over] = 0;
      board[to] = piece;
      jumps(board, nextPath, nextCaptured, piece, result);
      board[to] = 0;
      board[from] = piece;
      board[over] = enemy;
    }
  }
  if (!canJump && captured.length) result.push({ path, captured });
}

export function legalMoves(board, player) {
  const captures = [];
  const ordinary = [];
  const scratch = board.slice();
  for (let from = 0; from < 64; from++) {
    const piece = board[from];
    if (piece * player <= 0) continue;
    jumps(scratch, [from], [], piece, captures);
    for (const dr of Math.abs(piece) === 2 ? [-1, 1] : [-player]) {
      for (const dc of [-1, 1]) {
        const to = step(from, dr, dc);
        if (to >= 0 && board[to] === 0) ordinary.push({ path: [from, to], captured: [] });
      }
    }
  }
  return captures.length ? captures : ordinary;
}

export function playMove(board, move) {
  const next = board.slice();
  const piece = board[move.path[0]];
  const to = move.path.at(-1);
  next[move.path[0]] = 0;
  for (const square of move.captured) next[square] = 0;
  next[to] = crowns(piece, to) ? piece * 2 : piece;
  return next;
}

export function winner(board, turn) {
  return legalMoves(board, turn).length ? null : -turn;
}

function evaluate(board, turn) {
  let score = 0;
  for (let i = 0; i < 64; i++) {
    const piece = board[i];
    if (!piece) continue;
    const row = i >> 3;
    const col = i % 8;
    const center = 7 - Math.abs(row - 3.5) - Math.abs(col - 3.5);
    const advance = piece > 0 ? 7 - row : row;
    score += Math.sign(piece) * (Math.abs(piece) === 2 ? 176 + center * 3 : 100 + advance * 4 + center * 2);
  }
  return turn * score;
}

export function chooseMove(board, player, level = 3, random = Math.random) {
  const moves = legalMoves(board, player);
  if (moves.length < 2) return moves[0] ?? null;
  const depth = [1, 2, 4, 6, 9][level - 1] ?? 4;
  const budget = [40, 65, 120, 250, 650][level - 1] ?? 120;
  const noise = [110, 52, 10, 0, 0][level - 1] ?? 10;
  const deadline = performance.now() + budget;
  const timeout = Symbol("search limit");
  let nodes = 0;
  function search(position, turn, remaining, alpha, beta) {
    if ((++nodes & 63) === 0 && performance.now() >= deadline) throw timeout;
    const nextMoves = legalMoves(position, turn);
    if (!nextMoves.length) return -100000 - remaining;
    if (!remaining) return evaluate(position, turn);
    for (const move of nextMoves) {
      const value = -search(playMove(position, move), -turn, remaining - 1, -beta, -alpha);
      alpha = Math.max(alpha, value);
      if (alpha >= beta) break;
    }
    return alpha;
  }
  let ordered = moves.map(move => ({ move, bias: random() * noise, value: 0 }));
  for (let iteration = 1; iteration <= depth; iteration++) {
    try {
      ordered = ordered.map(entry => ({
        ...entry,
        value: entry.bias - search(playMove(board, entry.move), -player, iteration - 1, -Infinity, Infinity),
      })).sort((a, b) => b.value - a.value);
    } catch (error) {
      if (error !== timeout) throw error;
      break;
    }
  }
  return ordered[0].move;
}
