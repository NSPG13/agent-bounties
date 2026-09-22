import express from 'express';
import next from 'next';
import type { Server } from 'http';

/**
 * Next.js application instance.
 * Exported for tests that need to access the Next.js runtime directly.
 */
export const nextApp = next({
  dev: process.env.NODE_ENV !== 'production',
});

/**
 * Request handler that delegates all requests to Next.js.
 */
export const handle = nextApp.getRequestHandler();

/**
 * Creates an Express application that forwards all requests to Next.js.
 *
 * @returns {express.Express} The configured Express app.
 */
export const createServer = (): express.Express => {
  const server = express();

  // Forward every request to Next.js
  server.all('*', (req, res) => {
    return handle(req, res);
  });

  return server;
};

/**
 * Starts the server on the given port.
 *
 * @param {number} [port=3000] - The port to listen on.
 * @returns {Promise<Server>} A promise that resolves to the underlying HTTP server.
 */
export const startServer = async (port: number = 3000): Promise<Server> => {
  await nextApp.prepare();
  const server = createServer();
  return server.listen(port, () => {
    console.log(`> Ready on http://localhost:${port}`);
  });
};

/**
 * Gracefully shuts down the server.
 *
 * @param {Server} server - The HTTP server instance to close.
 * @returns {Promise<void>} Resolves when the server has closed.
 */
export const closeServer = async (server: Server): Promise<void> => {
  return new Promise((resolve, reject) => {
    server.close(err => {
      if (err) return reject(err);
      resolve();
    });
  });
};
