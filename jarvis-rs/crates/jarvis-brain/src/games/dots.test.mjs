import test from "node:test";
import assert from "node:assert/strict";
import { YOU, JARVIS, layout, createGame, edgeName, parseEdge, playEdge, score, gameStatus, chooseEdge } from "./dots-engine.js";

test("voice coordinates accept reversed neighbors and reject gaps or out-of-range dots", () => {
  const { horizontal, vertical } = layout(5);
  for (const text of ["a1 b1", "A1 to B1", "b1-a1", "a1 – b1"]) assert.equal(parseEdge(text, 5), horizontal(0, 0));
  assert.equal(parseEdge("a2 a1", 5), vertical(0, 0));
  assert.equal(parseEdge("a1 b2", 5), null);
  assert.equal(parseEdge("a6 a7", 5), null);
  for (const edge of layout(7).edges) assert.equal(parseEdge(edgeName(7, edge.id), 7), edge.id);
});

test("closing a box scores one point and gives the same player another turn", () => {
  let game = createGame(3);
  for (const text of ["a1 b1", "a2 b2", "a1 a2"]) game = playEdge(game, parseEdge(text, 3));
  const last = parseEdge("b1 b2", 3);
  assert.equal(game.turn, JARVIS);
  assert.equal(chooseEdge(game, 1, () => 0), last);
  const closed = playEdge(game, last);
  assert.equal(closed.turn, JARVIS);
  assert.deepEqual(score(closed), { you: 0, jarvis: 1 });
  assert.equal(playEdge(closed, last), null);
  assert.equal(game.owners[0], 0);
});

test("one shared edge can close two boxes at once", () => {
  const game = createGame(3);
  const { boxes, vertical } = layout(3);
  const shared = vertical(0, 1);
  for (const edge of [...boxes[0], ...boxes[1]]) if (edge !== shared) game.lines[edge] = 1;
  const closed = playEdge(game, shared);
  assert.equal(closed.turn, YOU);
  assert.deepEqual(score(closed), { you: 2, jarvis: 0 });
});

test("the engine avoids giving away a third side when a safe line exists", () => {
  const game = createGame(3);
  const { boxes } = layout(3);
  for (const edge of boxes[0].slice(0, 2)) game.lines[edge] = 1;
  for (const level of [3, 4, 5]) assert.ok(!boxes[0].includes(chooseEdge(game, level, () => 0.3)));
});

test("a full game has no legal moves and reports the result from the user's side", () => {
  let game = createGame(3);
  for (const edge of layout(3).edges) game = playEdge(game, edge.id);
  const points = score(game);
  assert.equal(points.you + points.jarvis, 9);
  assert.equal(gameStatus(game), points.you > points.jarvis ? "won" : "lost");
  assert.equal(chooseEdge(game, 5), null);
});
