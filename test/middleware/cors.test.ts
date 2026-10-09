import request from 'supertest';
import express from 'express';
import { corsMiddleware } from '../../src/middleware/cors';

describe('CORS middleware – x402 header exposure', () => {
  const app = express();
  app.use(corsMiddleware);
  // Dummy endpoint to trigger normal (non‑OPTIONS) flow
  app.get('/test', (_req, res) => {
    // Simulate the presence of the payment headers in the response
    res.set('x402-payment-challenge', 'challenge-token');
    res.set('x402-payment-confirmation', 'confirmation-token');
    res.json({ ok: true });
  });

  it('should expose x402-payment-challenge and x402-payment-confirmation on OPTIONS preflight', async () => {
    const response = await request(app)
      .options('/test')
      .set('Origin', 'http://example.com')
      .set('Access-Control-Request-Method', 'GET');

    expect(response.status).toBe(204);
    const expose = response.headers['access-control-expose-headers'];
    expect(expose).toContain('x402-payment-challenge');
    expect(expose).toContain('x402-payment-confirmation');
  });

  it('should expose x402-payment-challenge and x402-payment-confirmation on normal responses', async () => {
    const response = await request(app).get('/test').set('Origin', 'http://example.com');

    expect(response.status).toBe(200);
    const expose = response.headers['access-control-expose-headers'];
    expect(expose).toContain('x402-payment-challenge');
    expect(expose).toContain('x402-payment-confirmation');

    // Verify that the headers themselves are present (they are set by the handler)
    expect(response.headers['x402-payment-challenge']).toBe('challenge-token');
    expect(response.headers['x402-payment-confirmation']).toBe('confirmation-token');
  });
});
