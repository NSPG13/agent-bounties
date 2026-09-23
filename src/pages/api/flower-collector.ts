import type { NextApiRequest, NextApiResponse } from 'next';
import { generateFlowerCollector, FlowerCollectorParams } from '@/lib/flowerCollector';

/**
 * API endpoint that returns an OpenSCAD script for a flower‑shaped
 * rainwater collector. Query parameters map directly to the
 * `FlowerCollectorParams` interface.
 *
 * Example:
 *   GET /api/flower-collector?hubDiameter=180&petalCount=6
 *
 * Returns:
 *   200 – plain text OpenSCAD script
 *   400 – invalid query parameters
 */
export default function handler(req: NextApiRequest, res: NextApiResponse) {
  try {
    const {
      hubDiameter,
      hubHeight,
      petalCount,
      petalLength,
      petalBaseWidth,
      wallThickness,
    } = req.query;

    // Convert query strings to numbers when present
    const params: FlowerCollectorParams = {
      hubDiameter: hubDiameter ? Number(hubDiameter) : undefined,
      hubHeight: hubHeight ? Number(hubHeight) : undefined,
      petalCount: petalCount ? Number(petalCount) : undefined,
      petalLength: petalLength ? Number(petalLength) : undefined,
      petalBaseWidth: petalBaseWidth ? Number(petalBaseWidth) : undefined,
      wallThickness: wallThickness ? Number(wallThickness) : undefined,
    };

    // Basic sanity check – NaN values are treated as undefined (defaults)
    for (const key of Object.keys(params) as (keyof FlowerCollectorParams)[]) {
      if (params[key] !== undefined && Number.isNaN(params[key] as unknown as number)) {
        return res.status(400).json({ error: `Invalid number for ${key}` });
      }
    }

    const scad = generateFlowerCollector(params);
    res.setHeader('Content-Type', 'text/plain');
    res.status(200).send(scad);
  } catch (err: any) {
    res.status(400).json({ error: err.message ?? 'Bad request' });
  }
}
