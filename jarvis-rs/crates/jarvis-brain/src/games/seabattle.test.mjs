import test from "node:test";
import assert from "node:assert/strict";
import { SIZE, FLEET, randomFleet, emptyView, fire, afloat, heatmap, chooseShot, parseShot, label, rng, hint } from "./seabattle-engine.js";

test("sea battle: a random fleet fits, doesn't overlap or touch, and has every ship", () => {
  for (let seed = 1; seed <= 200; seed++) {
    const ships = randomFleet(rng(seed));
    assert.deepEqual(ships.map(s => s.length), FLEET.map(f => f.length));
    const seen = new Set();
    for (const ship of ships) {
      for (const p of ship.cells) {
        assert.ok(p.r >= 0 && p.r < SIZE && p.c >= 0 && p.c < SIZE);
        assert.ok(!seen.has(p.r * SIZE + p.c), "overlap");
        seen.add(p.r * SIZE + p.c);
      }
    }
    for (const ship of ships) {
      const mine = new Set(ship.cells.map(p => p.r * SIZE + p.c));
      for (const p of ship.cells) for (const [dr, dc] of [[1, 0], [-1, 0], [0, 1], [0, -1]]) {
        const k = (p.r + dr) * SIZE + (p.c + dc);
        if (p.r + dr >= 0 && p.r + dr < SIZE && p.c + dc >= 0 && p.c + dc < SIZE && !mine.has(k)) assert.ok(!seen.has(k), `ships touch at seed ${seed}`);
      }
    }
  }
});

test("sea battle: shots miss, hit, sink, and repeats are refused", () => {
  const ships = [{ name: "Patrol boat", length: 2, cells: [{ r: 0, c: 0 }, { r: 0, c: 1 }], hits: 0 }];
  const view = emptyView();
  assert.equal(fire(ships, view, 5, 5).result, "miss");
  assert.equal(fire(ships, view, 5, 5).result, "repeat");
  assert.deepEqual(fire(ships, view, 0, 0), { result: "hit", ship: "Patrol boat", allSunk: false });
  const last = fire(ships, view, 0, 1);
  assert.equal(last.result, "sunk");
  assert.equal(last.allSunk, true);
  assert.equal(view[0][0], "sunk");
  assert.deepEqual(afloat(ships), []);
  assert.throws(() => fire(ships, view, 10, 0));
});

test("sea battle: the hunter follows up a hit, and lines up a second hit", () => {
  const view = emptyView();
  view[4][4] = "hit";
  for (let i = 0; i < 30; i++) {
    const s = chooseShot(view, [3], 2, Math.random);
    assert.equal(Math.abs(s.r - 4) + Math.abs(s.c - 4), 1, "next to the hit");
  }
  view[4][5] = "hit";
  for (let i = 0; i < 30; i++) {
    const s = chooseShot(view, [3], 3, Math.random);
    assert.equal(s.r, 4, "continues along the line");
    assert.ok(s.c === 3 || s.c === 6);
  }
});

test("sea battle: the probability map ignores misses and sunk ships and favours squares next to hits", () => {
  const view = emptyView();
  view[0][0] = "miss";
  view[9][9] = "sunk";
  view[5][5] = "hit";
  const heat = heatmap(view, [2, 3]);
  assert.equal(heat[0][0], 0);
  assert.equal(heat[9][9], 0);
  assert.ok(heat[5][6] > heat[2][2], "next to a hit is hotter");
  const shot = chooseShot(view, [2, 3], 5, () => 0);
  assert.equal(Math.abs(shot.r - 5) + Math.abs(shot.c - 5), 1);
});

test("sea battle: stronger levels sink a fleet in fewer shots on average", () => {
  const average = level => {
    let total = 0;
    for (let seed = 1; seed <= 25; seed++) {
      const random = rng(seed * 7 + level);
      const ships = randomFleet(rng(seed));
      const view = emptyView();
      let shots = 0, done = false;
      while (!done) {
        const s = chooseShot(view, afloat(ships), level, random);
        done = fire(ships, view, s.r, s.c).allSunk;
        shots++;
      }
      total += shots;
    }
    return total / 25;
  };
  const [beginner, medium, expert] = [average(1), average(3), average(5)];
  assert.ok(beginner > medium && medium > expert, `${beginner} > ${medium} > ${expert}`);
  assert.ok(expert < 65, `expert should be sharp (${expert})`);
});

test("sea battle: spoken coordinates are understood", () => {
  const cases = { "b7": "B7", "fire at b7": "B7", "Fire at B 7.": "B7", "bravo seven": "B7", "j10": "J10", "a 1": "A1", "shoot charlie ten": "C10", "see five": "C5", "e for": "E4", "fire at d nine please": "D9" };
  for (const [said, square] of Object.entries(cases)) {
    const shot = parseShot(said);
    assert.ok(shot, said);
    assert.equal(label(shot), square, said);
  }
  for (const nonsense of ["hello", "k5", "b11", "fire", ""]) assert.equal(parseShot(nonsense), null, nonsense);
});

test("sea battle: a hint points at an unknown square", () => {
  const view = emptyView();
  view[3][3] = "hit";
  const h = hint(view, [3, 2]);
  assert.equal(view[h.r][h.c], "");
  assert.equal(Math.abs(h.r - 3) + Math.abs(h.c - 3), 1);
});
