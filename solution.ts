import express, { Request, Response, NextFunction } from 'express';
import cors from 'cors';
import helmet from 'helmet';
import apiRouter from './routes/api';

const app = express();

// Middleware
app.use(cors());
app.use(helmet());
app.use(express.json());

// Routes
app.use('/api', apiRouter);

// Global error handler – ensures every request gets a response
app.use((err: any, _req: Request, res: Response, _next: NextFunction) => {
  console.error('Unhandled error:', err);
  if (res.headersSent) {
    return;
  }
  res.status(err.status || 500).json({
    error: err.message || 'Internal Server Error',
  });
});

// Graceful shutdown handling
function startServer(port: number = 3000) {
  const server = app.listen(port, () => {
    console.log(`🚀 Server listening on http://localhost:${port}`);
  });

  // Handle SIGTERM & SIGINT for graceful shutdown
  const shutdown = () => {
    console.log('🛑 Shutting down server...');
    server.close(() => {
      console.log('✅ Server closed');
      process.exit(0);
    });
    // Force exit after 10 seconds
    setTimeout(() => process.exit(1), 10_000);
  };

  process.on('SIGTERM', shutdown);
  process.on('SIGINT', shutdown);
}

export { app, startServer };
