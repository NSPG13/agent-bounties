import { strict as assert } from 'node:assert';
import { createServer } from 'node:http';

// Test 1: Module loads without errors
let healthChecker, circuitBreaker, retry;
try {
  healthChecker = await import('../../src/api-reliability/health-checker.js');
  circuitBreaker = await import('../../src/api-reliability/circuit-breaker.js');
  retry = await import('../../src/api-reliability/retry.js');
  console.log('PASS: All modules load successfully');
} catch (e) {
  console.error('FAIL: Module import failed:', e.message);
  process.exit(1);
}

// Test 2: Circuit breaker starts CLOSED
const cb = circuitBreaker.createCircuitBreaker({ failureThreshold: 5, resetTimeout: 30000 });
assert.strictEqual(cb.getState(), 'CLOSED', 'Circuit breaker should start CLOSED');
console.log('PASS: Circuit breaker initial state is CLOSED');

// Test 3: Circuit breaker opens after threshold
for (let i = 0; i < 5; i++) {
  cb.recordFailure();
}
assert.strictEqual(cb.getState(), 'OPEN', 'Circuit breaker should be OPEN after 5 failures');
console.log('PASS: Circuit breaker opens after failure threshold');

// Test 4: Retry with exponential backoff
const retryConfig = retry.createRetryConfig({ maxAttempts: 3, baseDelay: 100 });
assert.strictEqual(retryConfig.maxAttempts, 3, 'Retry config should have maxAttempts=3');
assert.ok(retryConfig.baseDelay >= 100, 'Retry config should have baseDelay >= 100');
console.log('PASS: Retry configuration is valid');

// Test 5: Health checker monitors endpoints
const endpoints = ['http://localhost:1', 'http://localhost:2', 'http://localhost:3'];
const checker = healthChecker.createHealthChecker(endpoints, { interval: 1000, timeout: 200 });
assert.ok(checker, 'Health checker should be created');
assert.ok(Array.isArray(checker.getEndpoints()), 'Should return endpoints array');
assert.strictEqual(checker.getEndpoints().length, 3, 'Should monitor 3 endpoints');
console.log('PASS: Health checker monitors 3 endpoints');

// Test 6: Metrics reporting
const metrics = checker.getMetrics();
assert.ok(metrics !== undefined, 'Metrics object should exist');
assert.ok(typeof metrics === 'object', 'Metrics should be an object');
console.log('PASS: Health checker provides metrics');

// Test 7: Endpoint status tracking
checker.recordResult('http://localhost:1', true, 15);
checker.recordResult('http://localhost:2', false, 0);
const endpointStatus = checker.getEndpointStatus();
assert.ok(endpointStatus['http://localhost:1'], 'Endpoint 1 should have status');
assert.strictEqual(endpointStatus['http://localhost:1'].lastSuccess, true, 'Endpoint 1 last should be success');
assert.strictEqual(endpointStatus['http://localhost:2'].lastSuccess, false, 'Endpoint 2 last should be failure');
console.log('PASS: Endpoint status tracking works');

console.log('\nAll tests passed!');
