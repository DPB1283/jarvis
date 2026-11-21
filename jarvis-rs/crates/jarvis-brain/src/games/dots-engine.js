export const YOU = 1;
export const JARVIS = 2;

const layouts = new Map();

export function layout(size) {
  if (layouts.has(size)) return layouts.get(size);
  const offset = size * (size + 1);
  const horizontal = (row, col) => row * size + col;
  const vertical = (row, col) => offset + row * (size + 1) + col;
  const edges = Array.from({ length: offset * 2 }, (_, id) => {
    const across = id < offset;
    const index = across ? id : id - offset;
    const width = across ? size : size + 1;
    const row = Math.floor(index / width);
    const col = index % width;
    return { id, across, row, col, boxes: [] };
  });
  const boxes = Array.from({ length: size * size }, (_, id) => {
    const row = Math.floor(id / size);
    const col = id % size;
    const sides = [horizontal(row, col), horizontal(row + 1, col), vertical(row, col), vertical(row, col + 1)];
    for (const edge of sides) edges[edge].boxes.push(id);
    return sides;
  });
  const result = { edges, boxes, horizontal, vertical };
  layouts.set(size, result);
  return result;
}

export function createGame(size = 5) {
  const geometry = layout(size);
  return { size, lines: geometry.edges.map(() => 0), owners: geometry.boxes.map(() => 0), turn: YOU };
}

function dotName(row, col) {
  return String.fromCharCode(97 + col) + (row + 1);
}

export function edgeName(size, edge) {
  const { row, col, across } = layout(size).edges[edge];
  return dotName(row, col) + "–" + dotName(row + Number(!across), col + Number(across));
}

export function parseEdge(text, size) {
  const names = text.trim().toLowerCase().split(/\s*[-–]\s*|\s+(?:to\s+)?/);
  if (names.length !== 2 || names.some(name => !/^[a-h][1-8]$/.test(name))) return null;
  const [[r1, c1], [r2, c2]] = names.map(name => [Number(name[1]) - 1, name.charCodeAt(0) - 97]);
  if ([r1, c1, r2, c2].some(index => index > size) || Math.abs(r1 - r2) + Math.abs(c1 - c2) !== 1) return null;
  const { horizontal, vertical } = layout(size);
  return r1 === r2 ? horizontal(r1, Math.min(c1, c2)) : vertical(Math.min(r1, r2), c1);
}

export function playEdge(game, edge) {
  if (game.lines[edge] !== 0) return null;
  const { edges, boxes } = layout(game.size);
  const { turn } = game;
  const lines = game.lines.slice();
  const owners = game.owners.slice();
  lines[edge] = turn;
  let completed = 0;
  for (const box of edges[edge].boxes) {
    if (boxes[box].every(side => lines[side])) {
      owners[box] = turn;
      completed++;
    }
  }
  return { ...game, lines, owners, turn: completed ? turn : 3 - turn };
}

export function score(game) {
  const total = { you: 0, jarvis: 0 };
  for (const owner of game.owners) {
    if (owner === YOU) total.you++;
    if (owner === JARVIS) total.jarvis++;
  }
  return total;
}

export function gameStatus(game) {
  if (game.owners.includes(0)) return "playing";
  const { you, jarvis } = score(game);
  return you === jarvis ? "draw" : you > jarvis ? "won" : "lost";
}

function moveOptions(edges, occupied, counts) {
  const options = [];
  for (const edge of edges) {
    if (occupied[edge.id]) continue;
    let gain = 0, risk = 0;
    for (const box of edge.boxes) {
      if (counts[box] === 3) gain++;
      if (counts[box] === 2) risk++;
    }
    options.push({ edge, gain, risk });
  }
  return options;
}

function preferred(options) {
  const maxGain = Math.max(...options.map(move => move.gain));
  const taking = options.filter(move => move.gain === maxGain);
  const minRisk = Math.min(...taking.map(move => move.risk));
  return taking.filter(move => move.risk === minRisk);
}

export function chooseEdge(game, level = 3, random = Math.random) {
  const { edges, boxes } = layout(game.size);
  const occupied = Uint8Array.from(game.lines, Boolean);
  const counts = boxes.map(sides => sides.reduce((sum, edge) => sum + occupied[edge], 0));
  const options = moveOptions(edges, occupied, counts);
  if (!options.length) return null;
  const pick = moves => moves[Math.floor(random() * moves.length)];
  const easyMoves = preferred(options);
  if (level <= 3) {
    const mistake = [0.27, 0.1, 0][level - 1] ?? 0;
    return pick(easyMoves[0].gain || random() >= mistake ? easyMoves : options).edge.id;
  }

  function mark(edge, value) {
    occupied[edge.id] = value;
    for (const box of edge.boxes) counts[box] += value ? 1 : -1;
  }

  // Exact play in small endgames accounts for giving away two boxes to keep
  // control of the remaining chains, even when a box can be taken immediately.
  if (options.length <= (level === 5 ? 15 : 10)) {
    const positions = new Map();
    const bits = new Map(options.map(({ edge }, i) => [edge.id, 1 << i]));
    function value(mask) {
      if (!mask) return 0;
      if (positions.has(mask)) return positions.get(mask);
      let best = -Infinity;
      for (const { edge, gain } of moveOptions(edges, occupied, counts)) {
        mark(edge, 1);
        const rest = value(mask ^ bits.get(edge.id));
        mark(edge, 0);
        best = Math.max(best, gain ? gain + rest : -rest);
      }
      positions.set(mask, best);
      return best;
    }
    const mask = (1 << options.length) - 1;
    let best = null, bestValue = -Infinity;
    for (const move of options) {
      mark(move.edge, 1);
      const rest = value(mask ^ bits.get(move.edge.id));
      mark(move.edge, 0);
      const result = move.gain ? move.gain + rest : -rest;
      if (result > bestValue) {
        best = move.edge.id;
        bestValue = result;
      }
    }
    return best;
  }

  // Compare full rollouts so unsafe openings are ranked by the length of the
  // chains they give away, rather than only by their adjacent boxes.
  const candidates = level === 5 && easyMoves[0].gain && options.length <= 28
    ? options
    : easyMoves[0].risk && !easyMoves[0].gain ? options : easyMoves;
  let best = null, bestValue = -Infinity;
  for (const move of candidates) {
    let total = 0;
    const trials = level === 5 ? 8 : 4;
    for (let i = 0; i < trials; i++) {
      const played = [move.edge];
      mark(move.edge, 1);
      let sign = move.gain ? 1 : -1;
      let net = move.gain;
      while (played.length < options.length) {
        const next = pick(preferred(moveOptions(edges, occupied, counts)));
        net += next.gain * sign;
        if (!next.gain) sign = -sign;
        played.push(next.edge);
        mark(next.edge, 1);
      }
      for (const edge of played) mark(edge, 0);
      total += net;
    }
    if (total > bestValue) {
      best = move.edge.id;
      bestValue = total;
    }
  }
  return best;
}
