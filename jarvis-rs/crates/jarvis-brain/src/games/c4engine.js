// Connect Four: rules and a negamax / alpha-beta player. The board is 6 rows x 7 columns, row 0 on top,
// cells 0 (empty), 1 or 2. Five levels, from random-ish beginner to a deep search.

export const ROWS = 6;
export const COLS = 7;

export const LEVELS = {
  1: { name: "beginner", depth: 1, blunder: 0.4 },
  2: { name: "easy", depth: 2, blunder: 0.15 },
  3: { name: "medium", depth: 4, blunder: 0 },
  4: { name: "hard", depth: 6, blunder: 0 },
  5: { name: "expert", depth: 9, blunder: 0 },
};

export function emptyBoard() {
  return Array.from({ length: ROWS }, () => Array(COLS).fill(0));
}

export function legalColumns(board) {
  return [3, 2, 4, 1, 5, 0, 6].filter(c => board[0][c] === 0); // centre first: better pruning
}

/** Drop a piece; returns the row it landed in, or -1 if the column is full. */
export function drop(board, col, player) {
  for (let r = ROWS - 1; r >= 0; r--) {
    if (board[r][col] === 0) {
      board[r][col] = player;
      return r;
    }
  }
  return -1;
}

export function undrop(board, col) {
  for (let r = 0; r < ROWS; r++) {
    if (board[r][col] !== 0) {
      board[r][col] = 0;
      return;
    }
  }
}

/** The winner (1 or 2) with the four winning cells, or null. */
export function winner(board) {
  const dirs = [[0, 1], [1, 0], [1, 1], [1, -1]];
  for (let r = 0; r < ROWS; r++) {
    for (let c = 0; c < COLS; c++) {
      const p = board[r][c];
      if (!p) continue;
      for (const [dr, dc] of dirs) {
        const cells = [[r, c]];
        for (let k = 1; k < 4; k++) {
          const rr = r + dr * k;
          const cc = c + dc * k;
          if (rr < 0 || rr >= ROWS || cc < 0 || cc >= COLS || board[rr][cc] !== p) break;
          cells.push([rr, cc]);
        }
        if (cells.length === 4) return { player: p, cells };
      }
    }
  }
  return null;
}

export function isFull(board) {
  return board[0].every(v => v !== 0);
}

function scoreWindow(window, me) {
  const other = 3 - me;
  const mine = window.filter(v => v === me).length;
  const theirs = window.filter(v => v === other).length;
  const empty = window.filter(v => v === 0).length;
  if (mine === 4) return 100000;
  if (mine === 3 && empty === 1) return 50;
  if (mine === 2 && empty === 2) return 10;
  if (theirs === 3 && empty === 1) return -80;
  return 0;
}

/** Heuristic value for `me`: centre column, and open threes/twos. */
export function evaluate(board, me) {
  let score = 0;
  for (let r = 0; r < ROWS; r++) if (board[r][3] === me) score += 6;
  for (let r = 0; r < ROWS; r++) {
    for (let c = 0; c < COLS; c++) {
      if (c + 3 < COLS) score += scoreWindow([board[r][c], board[r][c + 1], board[r][c + 2], board[r][c + 3]], me);
      if (r + 3 < ROWS) score += scoreWindow([board[r][c], board[r + 1][c], board[r + 2][c], board[r + 3][c]], me);
      if (r + 3 < ROWS && c + 3 < COLS) score += scoreWindow([board[r][c], board[r + 1][c + 1], board[r + 2][c + 2], board[r + 3][c + 3]], me);
      if (r + 3 < ROWS && c - 3 >= 0) score += scoreWindow([board[r][c], board[r + 1][c - 1], board[r + 2][c - 2], board[r + 3][c - 3]], me);
    }
  }
  return score;
}

/** Negamax value of the position for the player to move (`player`). */
function negamax(board, depth, alpha, beta, player, me) {
  const win = winner(board);
  if (win) return win.player === player ? 1_000_000 + depth : -(1_000_000 + depth);
  const columns = legalColumns(board);
  if (columns.length === 0) return 0;
  if (depth === 0) return player === me ? evaluate(board, me) - evaluate(board, 3 - me) : -(evaluate(board, me) - evaluate(board, 3 - me));
  let best = -Infinity;
  for (const col of columns) {
    drop(board, col, player);
    const score = -negamax(board, depth - 1, -beta, -alpha, 3 - player, me);
    undrop(board, col);
    if (score > best) best = score;
    alpha = Math.max(alpha, score);
    if (alpha >= beta) break;
  }
  return best;
}

/** Pick a column for `player`. */
export function chooseColumn(board, player, level = 3) {
  const config = LEVELS[Math.min(5, Math.max(1, Math.round(level)))];
  const columns = legalColumns(board);
  if (columns.length === 0) return null;
  // Everyone takes a win in one, and (above beginner) blocks the opponent's.
  for (const col of columns) {
    drop(board, col, player);
    const wins = winner(board)?.player === player;
    undrop(board, col);
    if (wins) return col;
  }
  if (level >= 2) {
    for (const col of columns) {
      drop(board, col, 3 - player);
      const blocks = winner(board)?.player === 3 - player;
      undrop(board, col);
      if (blocks) return col;
    }
  }
  if (Math.random() < config.blunder) return columns[Math.floor(Math.random() * columns.length)];
  let best = columns[0];
  let bestScore = -Infinity;
  for (const col of columns) {
    drop(board, col, player);
    const score = -negamax(board, config.depth - 1, -Infinity, Infinity, 3 - player, player);
    undrop(board, col);
    if (score > bestScore) {
      bestScore = score;
      best = col;
    }
  }
  return best;
}
