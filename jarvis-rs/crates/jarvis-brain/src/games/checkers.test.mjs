import test from "node:test";
import assert from "node:assert/strict";
import { YOU, JARVIS, initialBoard, parsePath, squareName, legalMoves, playMove, winner, chooseMove } from "./checkers-engine.js";

function position(pieces) {
  const board = Array(64).fill(0);
  for (const [name, piece] of Object.entries(pieces)) board[parsePath(name + " a1")[0]] = piece;
  return board;
}

test("initial position has seven legal moves and standard voice coordinates", () => {
  const board = initialBoard();
  assert.equal(board.filter(piece => piece === YOU).length, 12);
  assert.equal(board.filter(piece => piece === JARVIS).length, 12);
  assert.equal(legalMoves(board, YOU).length, 7);
  for (const text of ["c3 d4", "C3 to D4", "c3-d4", "c3 - d4"]) {
    assert.deepEqual(parsePath(text).map(squareName), ["c3", "d4"]);
  }
  assert.equal(parsePath("go to i4"), null);
});

test("a forced capture excludes other pieces and must continue through every jump", () => {
  const board = position({ a3: YOU, c3: YOU, d4: JARVIS, f6: JARVIS });
  const original = board.slice();
  const moves = legalMoves(board, YOU);
  assert.deepEqual(moves.map(move => move.path.map(squareName)), [["c3", "e5", "g7"]]);
  assert.deepEqual(moves[0].captured.map(squareName), ["d4", "f6"]);
  assert.deepEqual(playMove(board, moves[0]), position({ a3: YOU, g7: YOU }));
  assert.deepEqual(board, original);
});

test("branching king captures restore pieces while generating alternate paths", () => {
  const board = position({ c3: 2, d4: -2, d6: JARVIS, f6: JARVIS });
  const moves = legalMoves(board, YOU);
  assert.deepEqual(moves.map(move => move.path.map(squareName)).sort(), [["c3", "e5", "c7"], ["c3", "e5", "g7"]]);
  assert.equal(board[parsePath("d4 a1")[0]], -2);
});

test("promotion ends a jump turn and the crowned piece can move backwards", () => {
  const board = position({ a6: YOU, b7: JARVIS, d7: JARVIS });
  const [capture] = legalMoves(board, YOU);
  assert.deepEqual(capture.path.map(squareName), ["a6", "c8"]);
  const next = playMove(board, capture);
  assert.deepEqual(next, position({ c8: 2, d7: JARVIS }));
  assert.deepEqual(legalMoves(next, YOU)[0].path.map(squareName), ["c8", "e6"]);
});

test("an immobilized side loses and the search plays a legal move", () => {
  assert.equal(winner(position({ a1: -2, h8: YOU }), YOU), JARVIS);
  const board = initialBoard();
  const move = chooseMove(board, YOU, 3, () => 0.5);
  assert.ok(legalMoves(board, YOU).some(option => JSON.stringify(option) === JSON.stringify(move)));
});
