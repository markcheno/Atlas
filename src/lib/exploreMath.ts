/**
 * The Explore map's geometry (Slice 4.6.1): fitting the map to the canvas,
 * zooming about a point, and finding the book under the pointer through a
 * grid, so a hover stays quick at 30,000 books.
 */

export interface View {
  /** Pixels per map unit, and where the map's origin lands. */
  scale: number;
  x: number;
  y: number;
}

export interface Bounds {
  minX: number;
  maxX: number;
  minY: number;
  maxY: number;
}

export interface Placed {
  x: number;
  y: number;
}

export function bounds(points: readonly Placed[]): Bounds {
  let minX = Infinity;
  let maxX = -Infinity;
  let minY = Infinity;
  let maxY = -Infinity;
  for (const p of points) {
    minX = Math.min(minX, p.x);
    maxX = Math.max(maxX, p.x);
    minY = Math.min(minY, p.y);
    maxY = Math.max(maxY, p.y);
  }
  if (!Number.isFinite(minX)) return { minX: 0, maxX: 1, minY: 0, maxY: 1 };
  return { minX, maxX, minY, maxY };
}

/** The whole map in `width` × `height`, with `pad` pixels around it. */
export function fit(b: Bounds, width: number, height: number, pad = 28): View {
  const spanX = Math.max(b.maxX - b.minX, 1e-6);
  const spanY = Math.max(b.maxY - b.minY, 1e-6);
  const scale = Math.max(Math.min((width - 2 * pad) / spanX, (height - 2 * pad) / spanY), 1e-6);
  return {
    scale,
    x: (width - spanX * scale) / 2 - b.minX * scale,
    y: (height - spanY * scale) / 2 - b.minY * scale,
  };
}

export function toScreen(v: View, p: Placed): [number, number] {
  return [p.x * v.scale + v.x, p.y * v.scale + v.y];
}

/** Zooms by `factor`, keeping the map point under (cx, cy) where it is. */
export function zoomAt(v: View, factor: number, cx: number, cy: number): View {
  const scale = Math.max(v.scale * factor, 1e-6);
  return {
    scale,
    x: cx - ((cx - v.x) * scale) / v.scale,
    y: cy - ((cy - v.y) * scale) / v.scale,
  };
}

/** Points by cell, in map units, for finding the nearest quickly. */
export class Grid {
  #cells = new Map<string, number[]>();
  #size: number;
  #points: readonly Placed[];

  constructor(points: readonly Placed[]) {
    this.#points = points;
    const b = bounds(points);
    const span = Math.max(b.maxX - b.minX, b.maxY - b.minY, 1e-6);
    const side = Math.max(1, Math.round(Math.sqrt(points.length / 4)));
    this.#size = span / side;
    points.forEach((p, i) => {
      const key = this.#key(this.#cell(p.x), this.#cell(p.y));
      const cell = this.#cells.get(key);
      if (cell) cell.push(i);
      else this.#cells.set(key, [i]);
    });
  }

  #cell(v: number): number {
    return Math.floor(v / this.#size);
  }

  #key(cx: number, cy: number): string {
    return `${cx},${cy}`;
  }

  /** The index of the point nearest (x, y) within `reach` map units, or -1. */
  nearest(x: number, y: number, reach: number): number {
    const r = Math.ceil(reach / this.#size);
    const cx = this.#cell(x);
    const cy = this.#cell(y);
    let best = -1;
    let bestD = reach * reach;
    for (let gx = cx - r; gx <= cx + r; gx++) {
      for (let gy = cy - r; gy <= cy + r; gy++) {
        for (const i of this.#cells.get(this.#key(gx, gy)) ?? []) {
          const p = this.#points[i];
          const d = (p.x - x) ** 2 + (p.y - y) ** 2;
          if (d < bestD || (d === bestD && i < best)) {
            bestD = d;
            best = i;
          }
        }
      }
    }
    return best;
  }
}
