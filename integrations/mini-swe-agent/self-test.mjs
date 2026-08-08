/**
 * Self-test for mini-SWE-agent paid-work environment (#774).
 * Validates the benchmark requirements: config, selector, fixtures, README.
 */
import { readFileSync, existsSync } from 'fs';
import { execSync } from 'child_process';
import { join, dirname } from 'path';
import { fileURLToPath } from 'url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const ROOT = join(__dirname, '..', '..', '..');
const BASE = join(ROOT, 'integrations', 'mini-swe-agent');

const TESTS = [];

function test(name, fn) {
  TESTS.push({ name, fn });
}

function assert(condition, message) {
  if (!condition) throw new Error(`FAIL: ${message}`);
}

// 1. Required files exist
test('all required files exist', () => {
  const required = [
    'config.yaml',
    'select_bounty.py',
    'README.md',
    'fixtures/multiple.json',
    'fixtures/empty.json',
    'fixtures/stale.json',
    'fixtures/no-margin.json',
    'fixtures/exclusive-claimant.json',
  ];
  for (const file of required) {
    const path = join(BASE, file);
    assert(existsSync(path), `${file} is missing`);
  }
});

// 2. Config validation
test('config.yaml contains required phrases', () => {
  const config = readFileSync(join(BASE, 'config.yaml'), 'utf8').toLowerCase();
  const required = ['inventory', 'claim', 'evidence', 'settlement', 'direct argv'];
  for (const phrase of required) {
    assert(config.includes(phrase), `config.yaml missing: ${phrase}`);
  }
  const forbidden = ['private_key', 'seed phrase', 'mnemonic', 'eth_sendtransaction'];
  for (const phrase of forbidden) {
    assert(!config.includes(phrase), `config.yaml contains forbidden: ${phrase}`);
  }
});

// 3. README validation
test('README.md contains required phrases', () => {
  const readme = readFileSync(join(BASE, 'README.md'), 'utf8').toLowerCase();
  const required = ['source_snapshot_digest', 'discovery_source', 'bountysettled'];
  for (const phrase of required) {
    assert(readme.includes(phrase), `README.md missing: ${phrase}`);
  }
});

// 4. Selector runs on all fixtures
test('select_bounty.py handles all fixtures correctly', () => {
  const selector = join(BASE, 'select_bounty.py');
  const expectations = {
    'multiple.json': 'claim',
    'empty.json': 'wait',
    'stale.json': 'refresh',
    'no-margin.json': 'skip',
    'exclusive-claimant.json': 'skip',
  };

  for (const [fixture, expectedAction] of Object.entries(expectations)) {
    const fixturePath = join(BASE, 'fixtures', fixture);
    const cmd = `python "${selector}" --input "${fixturePath}"`;
    try {
      const output = execSync(cmd, { encoding: 'utf8', timeout: 10000 });
      const result = JSON.parse(output);
      assert(
        result.action === expectedAction,
        `${fixture}: expected action=${expectedAction}, got ${result.action}`
      );
      assert(
        result.next_action && result.next_action.trim().length > 0,
        `${fixture}: next_action is empty`
      );
    } catch (err) {
      if (err.stdout) {
        try {
          const result = JSON.parse(err.stdout);
          assert(
            result.action === expectedAction,
            `${fixture}: expected action=${expectedAction}, got ${result.action} (exit code ${err.status})`
          );
        } catch {
          throw new Error(`${fixture}: selector failed with exit code ${err.status}: ${err.stdout}`);
        }
      } else {
        throw new Error(`${fixture}: selector threw: ${err.message}`);
      }
    }
  }
});

// 5. Fixture JSON validity
test('all fixtures are valid JSON', () => {
  const fixtures = [
    'multiple.json', 'empty.json', 'stale.json',
    'no-margin.json', 'exclusive-claimant.json',
  ];
  for (const fixture of fixtures) {
    const content = readFileSync(join(BASE, 'fixtures', fixture), 'utf8');
    try {
      JSON.parse(content);
    } catch {
      throw new Error(`${fixture} is not valid JSON`);
    }
  }
});

// Run all tests
let passed = 0;
let failed = 0;
const failures = [];

for (const { name, fn } of TESTS) {
  try {
    fn();
    passed++;
    console.log(`✓ ${name}`);
  } catch (err) {
    failed++;
    failures.push({ name, error: err.message });
    console.log(`✗ ${name}: ${err.message}`);
  }
}

console.log(`\n${passed}/${TESTS.length} passed, ${failed} failed`);
if (failed > 0) {
  console.log('\nFailures:');
  for (const f of failures) {
    console.log(`  - ${f.name}: ${f.error}`);
  }
  process.exit(1);
}
