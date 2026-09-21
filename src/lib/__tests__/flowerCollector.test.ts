import { generateFlowerCollector, FlowerCollectorParams } from '@/lib/flowerCollector';

describe('generateFlowerCollector', () => {
  it('produces a non‑empty OpenSCAD script with default parameters', () => {
    const scad = generateFlowerCollector();
    expect(scad).toContain('module hub()');
    expect(scad).toContain('module petal()');
    expect(scad).toContain('collector();');
    expect(scad.length).toBeGreaterThan(100);
  });

  it('respects custom parameters', () => {
    const params: FlowerCollectorParams = {
      hubDiameter: 150,
      hubHeight: 120,
      petalCount: 5,
      petalLength: 250,
      petalBaseWidth: 80,
      wallThickness: 3,
    };
    const scad = generateFlowerCollector(params);
    expect(scad).toContain('cylinder(d=150, h=120');
    expect(scad).toContain('for (i = [0:4])'); // 5 petals => indices 0‑4
    expect(scad).toContain('translate([75,0,117]'); // hubDiameter/2 = 75, hubHeight - wallThickness = 117
  });

  it('throws on invalid petalCount', () => {
    expect(() => generateFlowerCollector({ petalCount: 2 })).toThrow(
      'petalCount must be at least 3'
    );
  });
});
