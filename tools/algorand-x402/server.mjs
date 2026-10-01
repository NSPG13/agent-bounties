import { serve } from '@hono/node-server';
import { readConfig } from './config.mjs';
import { createApp } from './app.mjs';
const config = readConfig();
const app = createApp(config);
serve({ fetch: app.fetch, port: Number(process.env.PORT || 4021), hostname: '0.0.0.0' });
console.log(JSON.stringify({ event: 'listening', network: config.network, payTo: config.payTo }));
