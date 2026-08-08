import { existsSync, readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const DIR = resolve(import.meta.dirname || '.');
const BENCHMARK_DIR = resolve(DIR, '..', '..', '..', 'benchmarks', 'standing-meta-v2', 'mcp-interoperability');

// Self-test 1: All required files exist
const requiredFiles = ['README.md', 'test.mjs', 'self-test.mjs'];
for (const file of requiredFiles) {
  const path = resolve(BENCHMARK_DIR, file);
  if (!existsSync(path)) {
    console.error(`FAIL: Missing file ${file}`);
    process.exit(1);
  }
}
console.log('PASS: All required benchmark files exist');

// Self-test 2: README contains required sections
const readme = readFileSync(resolve(BENCHMARK_DIR, 'README.md'), 'utf8');
const requiredSections = ['Objective', 'Requirements', 'Deliverable', 'Validation', 'Reward'];
for (const section of requiredSections) {
  if (!readme.includes(section)) {
    console.error(`FAIL: README missing section "${section}"`);
    process.exit(1);
  }
}
console.log('PASS: README contains all required sections');

// Self-test 3: README mentions MCP SDK dependency
if (!readme.includes('@modelcontextprotocol/sdk') && !readme.includes('modelcontextprotocol')) {
  console.error('FAIL: README should reference MCP SDK');
  process.exit(1);
}
console.log('PASS: README references MCP SDK');

// Self-test 4: test.mjs is valid JS
try {
  new Function(readFileSync(resolve(BENCHMARK_DIR, 'test.mjs'), 'utf8'));
  console.log('PASS: test.mjs is syntactically valid');
} catch (e) {
  console.error('FAIL: test.mjs has syntax errors:', e.message);
  process.exit(1);
}

console.log('\nSelf-test passed!');
