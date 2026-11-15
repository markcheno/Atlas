import { describe, expect, it } from 'vitest';
import { bounds, fit, Grid, toScreen, zoomAt } from './exploreMath';

describe('the Explore map’s geometry (Slice 4.6.1)', () => {
  it('fits the whole map in the canvas, centred, with a margin', () => {
    const points = [
      { x: -5, y: 0 },
      { x: 5, y: 2 },
    ];
    const v = fit(bounds(points), 400, 200, 20);
    const [x0, y0] = toScreen(v, points[0]);
    const [x1, y1] = toScreen(v, points[1]);
    expect(x0).toBeCloseTo(20);
    expect(x1).toBeCloseTo(380);
    expect((y0 + y1) / 2).toBeCloseTo(100);
  });

  it('zooms about a point, keeping it where it is', () => {
    const v = { scale: 10, x: 3, y: -7 };
    const z = zoomAt(v, 2.5, 120, 80);
    const before = [(120 - v.x) / v.scale, (80 - v.y) / v.scale];
    const after = [(120 - z.x) / z.scale, (80 - z.y) / z.scale];
    expect(after[0]).toBeCloseTo(before[0]);
    expect(after[1]).toBeCloseTo(before[1]);
    expect(z.scale).toBeCloseTo(25);
  });

  it('finds the nearest book as a scan of every book would, within reach', () => {
    let seed = 7;
    const rand = () => ((seed = (seed * 16807) % 2147483647) / 2147483647) * 100;
    const points = Array.from({ length: 3000 }, () => ({ x: rand(), y: rand() }));
    const grid = new Grid(points);
    for (let i = 0; i < 200; i++) {
      const [x, y, reach] = [rand(), rand(), rand() / 20];
      let best = -1;
      let bestD = reach * reach;
      points.forEach((p, j) => {
        const d = (p.x - x) ** 2 + (p.y - y) ** 2;
        if (d < bestD) {
          bestD = d;
          best = j;
        }
      });
      expect(grid.nearest(x, y, reach)).toBe(best);
    }
    expect(new Grid([]).nearest(0, 0, 1)).toBe(-1);
  });
});
