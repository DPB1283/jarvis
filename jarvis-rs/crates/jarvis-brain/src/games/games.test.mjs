// Tests for the chess and Connect Four engines. Run:  node --test jarvis-rs/crates/jarvis-brain/src/games/
import test from "node:test";
import assert from "node:assert/strict";
import { Chess } from "./chess.js";
import { chooseMove, evaluate, LEVELS } from "./engine.js";
import * as c4 from "./c4engine.js";

const play = (fen, level, timeMs) => chooseMove(fen, level, { timeMs });

test("chess: every level plays a legal move from the start", () => {
  for (const level of [1, 2, 3, 4, 5]) {
    const move = play(new Chess().fen(), level, 300);
    const chess = new Chess();
    assert.doesNotThrow(() => chess.move({ from: move.from, to: move.to, promotion: move.promotion }), `level ${level}`);
  }
});

test("chess: finds a mate in one (back-rank) at medium and above", () => {
  const fen = "6k1/5ppp/8/8/8/8/5PPP/R5K1 w - - 0 1";
  for (const level of [3, 4, 5]) {
    const move = play(fen, level, 1500);
    assert.equal(move.san, "Ra8#", `level ${level} played ${move.san}`);
  }
});

test("chess: takes a free queen at medium and above", () => {
  // White's knight can capture the undefended black queen on h4.
  const fen = "rnb1kbnr/pppp1ppp/8/4p3/4P2q/5N2/PPPP1PPP/RNBQKB1R w KQkq - 2 3";
  for (const level of [3, 4, 5]) {
    const move = play(fen, level, 1500);
    assert.equal(move.san, "Nxh4", `level ${level} played ${move.san}`);
  }
});

test("chess: does not leave its queen en prise when a safe move exists (hard)", () => {
  // Black to move; the queen on d8 is attacked by nothing now, but Bxf7+/Qxd8 tricks are avoided by a 5-ply search.
  const fen = "rnbqkbnr/pppp1ppp/8/4p3/4P3/8/PPPP1PPP/RNBQKBNR w KQkq e6 0 2";
  const move = play(fen, 4, 2000);
  const chess = new Chess(fen);
  chess.move({ from: move.from, to: move.to, promotion: move.promotion });
  const queenCaptured = chess.moves({ verbose: true }).some(m => m.captured === "q");
  assert.equal(queenCaptured, false, `after ${move.san} the queen can be captured`);
});

test("chess: evaluation is symmetric and sees a material edge", () => {
  assert.equal(evaluate(new Chess()), 0);
  const upAQueen = new Chess("rnb1kbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1");
  assert.ok(evaluate(upAQueen) > 800);
  const sameBlackToMove = new Chess("rnb1kbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR b KQkq - 0 1");
  assert.ok(evaluate(sameBlackToMove) < -800, "scores are from the side to move");
});

test("chess: a checkmated or stalemated side has no move", () => {
  assert.equal(play("7k/5Q2/6K1/8/8/8/8/8 b - - 0 1", 3, 300), null, "stalemate");
  assert.equal(play("R5k1/5ppp/8/8/8/8/5PPP/6K1 b - - 0 1", 3, 300), null, "checkmate");
});

test("chess: the time limit is respected and a full level-4 move is quick enough to play against", () => {
  const started = performance.now();
  const move = play(new Chess().fen(), 4, 1000);
  const elapsed = performance.now() - started;
  assert.ok(move);
  assert.ok(elapsed < 2500, `took ${elapsed} ms`);
});

test("chess: the strongest level beats the weakest over a short game", () => {
  const chess = new Chess();
  const strongWhite = true;
  for (let ply = 0; ply < 60 && !chess.isGameOver(); ply++) {
    const level = (chess.turn() === "w") === strongWhite ? 4 : 1;
    const move = play(chess.fen(), level, level === 4 ? 400 : 100);
    chess.move({ from: move.from, to: move.to, promotion: move.promotion });
  }
  const board = chess.board().flat().filter(Boolean);
  const material = color => board.filter(p => p.color === color).reduce((s, p) => s + { p: 1, n: 3, b: 3, r: 5, q: 9, k: 0 }[p.type], 0);
  assert.ok(chess.isCheckmate() ? chess.turn() === "b" : material("w") > material("b"), `white ${material("w")} vs black ${material("b")}`);
});

test("chess: level names", () => {
  assert.deepEqual(Object.values(LEVELS).map(l => l.name), ["beginner", "easy", "medium", "hard", "expert"]);
});

// ---- Connect Four ----------------------------------------------------------------------------------

function boardFrom(rows) {
  return rows.map(r => [...r].map(ch => (ch === "X" ? 1 : ch === "O" ? 2 : 0)));
}

test("connect four: detects wins in all four directions and a draw", () => {
  const b = c4.emptyBoard();
  for (let c = 0; c < 4; c++) c4.drop(b, c, 1);
  assert.equal(c4.winner(b).player, 1);
  const v = c4.emptyBoard();
  for (let i = 0; i < 4; i++) c4.drop(v, 2, 2);
  assert.equal(c4.winner(v).player, 2);
  const d = boardFrom(["......X", ".....XO", "....XOO", "...XOOO", "......", "......"].map(r => r.padEnd(7, ".")));
  assert.equal(c4.winner(d).player, 1);
  const a = boardFrom(["X......", ".X.....", "..X....", "...X...", ".......", "......."]);
  assert.equal(c4.winner(a).player, 1);
  assert.equal(c4.winner(c4.emptyBoard()), null);
  assert.equal(c4.drop(boardFrom(["XOXOXOX", "OXOXOXO", "XOXOXOX", "OXOXOXO", "XOXOXOX", "OXOXOXO"]), 0, 1), -1, "full column");
});

test("connect four: takes a win in one and blocks a loss in one", () => {
  const win = boardFrom([".......", ".......", ".......", ".......", "OO.....", "XXX...."]);
  assert.equal(c4.chooseColumn(win, 1, 3), 3);
  const block = boardFrom([".......", ".......", ".......", ".......", "XX.....", "OOO...."]);
  assert.equal(c4.chooseColumn(block, 1, 3), 3);
});

test("connect four: sees a two-move trap (an open three on the bottom row) at medium", () => {
  // X has .XX. on the bottom with both ends open: O must stop it now or lose. Playing the centre-left block is required.
  const b = boardFrom([".......", ".......", ".......", ".......", ".......", ".XX...."]);
  // X to move can play column 3 for XXX with both 0 and 4... only 0 and 3 end open: choose any move that keeps the threat.
  const col = c4.chooseColumn(b, 1, 3);
  assert.ok([0, 3].includes(col), `played ${col}`);
});

test("connect four: the hard AI beats a random player nearly always", () => {
  let wins = 0;
  const games = 12;
  for (let g = 0; g < games; g++) {
    const b = c4.emptyBoard();
    let player = g % 2 === 0 ? 1 : 2; // the AI is player 1 in even games
    for (let ply = 0; ply < 42; ply++) {
      const ai = player === 1;
      const col = ai ? c4.chooseColumn(b, 1, 3) : (() => { const cols = c4.legalColumns(b); return cols[Math.floor(Math.random() * cols.length)]; })();
      if (col === null) break;
      c4.drop(b, col, player);
      const w = c4.winner(b);
      if (w) { if (w.player === 1) wins++; break; }
      player = 3 - player;
    }
  }
  assert.ok(wins >= 11, `the AI won ${wins} of ${games}`);
});

test("connect four: levels exist and the strongest takes longer but still answers", () => {
  assert.deepEqual(Object.values(c4.LEVELS).map(l => l.name), ["beginner", "easy", "medium", "hard", "expert"]);
  const b = c4.emptyBoard();
  const started = performance.now();
  assert.ok(c4.legalColumns(b).includes(c4.chooseColumn(b, 1, 5)));
  assert.ok(performance.now() - started < 8000, "expert answers within a few seconds");
});
