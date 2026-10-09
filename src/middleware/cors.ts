import { Request, Response, NextFunction } from 'express';

/**
 * CORS middleware configuration.
 *
 * This middleware is used across the API to ensure that browser‑based SDK clients
 * can safely make cross‑origin requests.  The recent bounty requires that the
 * `x402-payment-challenge` and `x402-payment-confirmation` headers be readable
 * by browsers, so they must be added to the `Access‑Control‑Expose‑Headers`
 * response header (and also to the allowed request headers list for completeness).
 */
export function corsMiddleware(req: Request, res: Response, next: NextFunction) {
  // Allow any origin that is configured for the API (fallback to '*')
  const allowedOrigin = process.env.ALLOWED_ORIGIN || '*';
  res.header('Access-Control-Allow-Origin', allowedOrigin);

  // Standard CORS pre‑flight handling
  if (req.method === 'OPTIONS') {
    res.header(
      'Access-Control-Allow-Methods',
      'GET,POST,PUT,PATCH,DELETE,OPTIONS'
    );

    // Existing allowed request headers
    const allowedHeaders = [
      'Content-Type',
      'Authorization',
      // The x402 payment headers are now part of the allowed request headers
      // to allow clients to send them when required.
      'x402-payment-challenge',
      'x402-payment-confirmation',
    ];
    res.header('Access-Control-Allow-Headers', allowedHeaders.join(','));

    // Expose the x402 payment headers to browser SDKs
    const exposedHeaders = [
      // Existing exposed headers
      'X-Total-Count',
      // New additive compatibility headers
      'x402-payment-challenge',
      'x402-payment-confirmation',
    ];
    res.header('Access-Control-Expose-Headers', exposedHeaders.join(','));

    // Short‑circuit the pre‑flight request
    return res.sendStatus(204);
  }

  // For non‑OPTIONS requests we still need to expose the headers
  const exposedHeaders = [
    'X-Total-Count',
    'x402-payment-challenge',
    'x402-payment-confirmation',
  ];
  res.header('Access-Control-Expose-Headers', exposedHeaders.join(','));

  next();
}
