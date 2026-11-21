// Sea battle: two hidden 10×10 fleets, one shot per turn. Pure functions so they can be tested with node --test.
// Squares are { r, c } with r = 0..9 (row 1..10, top to bottom) and c = 0..9 (column A..J).

export const SIZE = 10;
export const FLEET = [
  { name: "Carrier", length: 5 },
  { name: "Cruiser", length: 4 },
  { name: "Frigate", length: 3 },
  { name: "Submarine", length: 3 },
  { name: "Patrol boat", length: 2 },
];
export const COLUMNS = "ABCDEFGHIJ";

export const label = ({ r, c }) => COLUMNS[c] + (r + 1);
const inside = (r, c) => r >= 0 && r < SIZE && c >= 0 && c < SIZE;
const key = (r, c) => r * SIZE + c;

/** Small seedable random generator, so tests (and replays) are repeatable. */
export function rng(seed = Date.now()) {
  let s = seed >>> 0 || 1;
  return () => {
    s ^= s << 13; s ^= s >>> 17; s ^= s << 5;
    return (s >>> 0) / 4294967296;
  };
}

function cells(r, c, length, across) {
  return Array.from({ length }, (_, i) => (across ? { r, c: c + i } : { r: r + i, c }));
}

/** Place the whole fleet at random: no overlaps, and (for a fairer game) no two ships touching side by side. */
export function randomFleet(random = Math.random) {
  for (let attempt = 0; attempt < 500; attempt++) {
    const taken = new Set();
    const ships = [];
    let ok = true;
    for (const { name, length } of FLEET) {
      let placed = false;
      for (let tries = 0; tries < 200 && !placed; tries++) {
        const across = random() < 0.5;
        const r = Math.floor(random() * (across ? SIZE : SIZE - length + 1));
        const c = Math.floor(random() * (across ? SIZE - length + 1 : SIZE));
        const body = cells(r, c, length, across);
        const clear = body.every(p => [[0, 0], [1, 0], [-1, 0], [0, 1], [0, -1]].every(([dr, dc]) => !taken.has(key(p.r + dr, p.c + dc))));
        if (!clear) continue;
        for (const p of body) taken.add(key(p.r, p.c));
        ships.push({ name, length, cells: body, hits: 0 });
        placed = true;
      }
      if (!placed) { ok = false; break; }
    }
    if (ok) return ships;
  }
  throw new Error("Could not place the fleet");
}

/** A fresh view of a sea: what each square is known to be. "" unknown, "miss", "hit", "sunk". */
export const emptyView = () => Array.from({ length: SIZE }, () => Array(SIZE).fill(""));

/**
 * Fire at a fleet. Mutates the ship's hit count and the view. Returns { result: "miss" | "hit" | "sunk" | "repeat",
 * ship?: name, allSunk: bool }.
 */
export function fire(ships, view, r, c) {
  if (!inside(r, c)) throw new Error("Off the board");
  if (view[r][c]) return { result: "repeat", allSunk: ships.every(s => s.hits >= s.length) };
  const ship = ships.find(s => s.cells.some(p => p.r === r && p.c === c));
  if (!ship) {
    view[r][c] = "miss";
    return { result: "miss", allSunk: false };
  }
  ship.hits += 1;
  view[r][c] = "hit";
  const sunk = ship.hits >= ship.length;
  if (sunk) for (const p of ship.cells) view[p.r][p.c] = "sunk";
  return { result: sunk ? "sunk" : "hit", ship: ship.name, allSunk: ships.every(s => s.hits >= s.length) };
}

/** Lengths of the ships in a fleet that are still afloat. */
export const afloat = ships => ships.filter(s => s.hits < s.length).map(s => s.length);

/**
 * How likely each unknown square is to hold a ship, given what is known: count the ways every remaining ship could
 * lie there. Placements through a known hit count extra, since an unsunk hit means a ship is right there.
 */
export function heatmap(view, lengths) {
  const heat = Array.from({ length: SIZE }, () => Array(SIZE).fill(0));
  for (const length of lengths) {
    for (const across of [true, false]) {
      for (let r = 0; r < (across ? SIZE : SIZE - length + 1); r++) {
        for (let c = 0; c < (across ? SIZE - length + 1 : SIZE); c++) {
          const body = cells(r, c, length, across);
          if (body.some(p => view[p.r][p.c] === "miss" || view[p.r][p.c] === "sunk")) continue;
          const hits = body.filter(p => view[p.r][p.c] === "hit").length;
          const weight = 1 + hits * 30;
          for (const p of body) if (!view[p.r][p.c]) heat[p.r][p.c] += weight;
        }
      }
    }
  }
  return heat;
}

function unknownSquares(view) {
  const out = [];
  for (let r = 0; r < SIZE; r++) for (let c = 0; c < SIZE; c++) if (!view[r][c]) out.push({ r, c });
  return out;
}

/** Squares next to an unsunk hit: where a hunter looks next. Lined-up hits extend along their line first. */
function targets(view) {
  const hits = [];
  for (let r = 0; r < SIZE; r++) for (let c = 0; c < SIZE; c++) if (view[r][c] === "hit") hits.push({ r, c });
  const lined = [], around = [];
  for (const h of hits) {
    for (const [dr, dc] of [[0, 1], [0, -1], [1, 0], [-1, 0]]) {
      const r = h.r + dr, c = h.c + dc;
      if (!inside(r, c) || view[r][c]) continue;
      const behind = { r: h.r - dr, c: h.c - dc };
      (inside(behind.r, behind.c) && view[behind.r][behind.c] === "hit" ? lined : around).push({ r, c });
    }
  }
  return lined.length ? lined : around;
}

/**
 * Jarvis's shot. level 1: random. 2: random until a hit, then hunts around it. 3: as 2 but searches on a
 * checkerboard (every ship covers one). 4–5: probability map (5 adds no randomness and weighs hunting harder).
 */
export function chooseShot(view, lengths, level = 3, random = Math.random) {
  const open = unknownSquares(view);
  if (!open.length) throw new Error("No squares left");
  const pick = list => list[Math.floor(random() * list.length)];
  if (level <= 1) return pick(open);
  if (level <= 3) {
    const hunt = targets(view);
    if (hunt.length) return pick(hunt);
    if (level === 3) {
      const smallest = Math.min(...lengths, 2);
      const parity = open.filter(p => (p.r + p.c) % smallest === 0);
      if (parity.length) return pick(parity);
    }
    return pick(open);
  }
  const heat = heatmap(view, lengths);
  let best = -1, choices = [];
  for (const p of open) {
    const h = heat[p.r][p.c] + (level === 4 ? random() * 2 : 0);
    if (h > best + 1e-9) { best = h; choices = [p]; } else if (Math.abs(h - best) <= 1e-9) choices.push(p);
  }
  return pick(choices);
}

const WORDS = { alpha: "a", alfa: "a", bravo: "b", charlie: "c", delta: "d", echo: "e", foxtrot: "f", golf: "g", hotel: "h", india: "i", juliet: "j", juliett: "j" };
const NUMBERS = { one: 1, two: 2, three: 3, four: 4, five: 5, six: 6, seven: 7, eight: 8, nine: 9, ten: 10, to: 2, too: 2, for: 4, won: 1, ate: 8 };
const LETTER_SOUNDS = { bee: "b", be: "b", see: "c", sea: "c", dee: "d", gee: "g", jay: "j", eye: "i" };

/** Turn what the user said ("fire at B7", "bravo seven", "b 10") into a square, or null. */
export function parseShot(text) {
  const words = String(text).toLowerCase().replace(/[^a-z0-9\s]/g, " ").split(/\s+/).filter(Boolean)
    .filter(w => !["fire", "at", "shoot", "target", "on", "the", "square", "please", "jarvis", "ultron", "missile", "torpedo", "launch", "hit", "try"].includes(w));
  const joined = words.map(w => WORDS[w] ?? LETTER_SOUNDS[w] ?? (NUMBERS[w] !== undefined ? String(NUMBERS[w]) : w)).join(" ");
  const m = /\b([a-j])\s*(10|[1-9])\b/.exec(joined) ?? /\b([a-j])(10|[1-9])\b/.exec(joined.replace(/\s+/g, ""));
  if (!m) return null;
  return { r: Number(m[2]) - 1, c: COLUMNS.indexOf(m[1].toUpperCase()) };
}

/** A suggestion for the user: the hottest square in Jarvis's waters. */
export const hint = (view, lengths) => chooseShot(view, lengths, 5, () => 0);
