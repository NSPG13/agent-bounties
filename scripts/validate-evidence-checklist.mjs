#!/usr/bin/env node
import { existsSync, readFileSync } from 'node:fs';
import { createHash } from 'node:crypto';

const SCHEMA_ID = 'https://agentbounties.app/schemas/direct-evidence-checklist-v1.json';

const SAMPLE_VALID = {
  schema: SCHEMA_ID,
  source_commit: 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
  repository: 'owner/repo',
  subdirectory: 'src',
  pull_request_url: 'https://github.com/owner/repo/pull/123',
  check_runs: [
    {
      id: 1,
      name: 'ci',
      status: 'completed',
      conclusion: 'success',
      head_sha: 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
      html_url: 'https://github.com/owner/repo/actions/runs/123456789',
      repository: {
        full_name: 'owner/repo',
      },
    },
  ],
  artifact_digest: {
    algorithm: 'sha256',
    value: createHash('sha256').update('artifact').digest('hex'),
  },
};

function fail(message) {
  console.error(message);
  process.exit(1);
}

function readChecklist(filePath) {
  if (!existsSync(filePath)) {
    fail(`Evidence file not found: ${filePath}`);
  }

  const raw = readFileSync(filePath, 'utf8');
  try {
    return JSON.parse(raw);
  } catch {
    fail(`Invalid JSON: ${filePath}`);
  }
}

function isHex(value, len) {
  if (typeof value !== 'string') {
    return false;
  }
  const pattern = len ? new RegExp(`^[0-9a-f]{${len}}$`) : /^[0-9a-f]+$/;
  return pattern.test(value);
}

function validHttpsUrl(value) {
  return typeof value === 'string' && /^https:\/\//.test(value);
}

function validateArtifactDigest(digest) {
  if (!digest || typeof digest !== 'object') {
    return 'artifact_digest must be an object';
  }

  const { algorithm, value } = digest;
  if (algorithm !== 'sha256' && algorithm !== 'sha512') {
    return `artifact_digest.algorithm must be sha256 or sha512 (got ${algorithm})`;
  }

  const expectedLen = algorithm === 'sha256' ? 64 : 128;
  if (!isHex((value || '').toLowerCase(), expectedLen)) {
    return `artifact_digest.value must be ${expectedLen} lowercase hex chars for ${algorithm}`;
  }

  return null;
}

function validateChecklist(evidence) {
  const errors = [];

  if (!evidence || typeof evidence !== 'object') {
    return ['Evidence must be a JSON object'];
  }

  if (evidence.schema !== SCHEMA_ID) {
    errors.push(`schema must be ${SCHEMA_ID}`);
  }

  if (!isHex(evidence.source_commit, 40)) {
    errors.push('source_commit must be a lowercase 40-char SHA-1');
  }

  if (typeof evidence.repository !== 'string' || !/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(evidence.repository)) {
    errors.push('repository must be owner/repo');
  }

  if (typeof evidence.subdirectory !== 'string') {
    errors.push('subdirectory must be a string');
  }

  if (!validHttpsUrl(evidence.pull_request_url)) {
    errors.push('pull_request_url must be https');
  } else if (!/^https:\/\/github\.com\/[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+\/pull\/[0-9]+$/.test(evidence.pull_request_url)) {
    errors.push('pull_request_url must be https://github.com/<owner>/<repo>/pull/<number>');
  }

  if (!Array.isArray(evidence.check_runs) || evidence.check_runs.length === 0) {
    errors.push('check_runs must be a non-empty array');
  } else {
    evidence.check_runs.forEach((item, index) => {
      if (!item || typeof item !== 'object') {
        errors.push(`check_runs[${index}] must be an object`);
        return;
      }
      if (typeof item.id !== 'number' || item.id <= 0) {
        errors.push(`check_runs[${index}].id must be a positive integer`);
      }
      if (typeof item.name !== 'string' || item.name.length === 0) {
        errors.push(`check_runs[${index}].name is required`);
      }
      if (item.status !== 'completed') {
        errors.push(`check_runs[${index}].status must be "completed"`);
      }
      if (item.conclusion !== 'success') {
        errors.push(`check_runs[${index}].conclusion must be "success"`);
      }
      if (!isHex(item.head_sha, 40)) {
        errors.push(`check_runs[${index}].head_sha must be lowercase 40-char sha1`);
      }
      if (!validHttpsUrl(item.html_url)) {
        errors.push(`check_runs[${index}].html_url must be https`);
      } else if (!/^https:\/\/github\.com\/.+\/actions\/runs\/[0-9]+$/.test(item.html_url)) {
        errors.push(`check_runs[${index}].html_url must be a GitHub Actions run URL`);
      }
      if (!item.repository || typeof item.repository !== 'object' || typeof item.repository.full_name !== 'string' || !/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(item.repository.full_name)) {
        errors.push(`check_runs[${index}].repository.full_name must be owner/repo`);
      }
    });
  }

  const artifactError = validateArtifactDigest(evidence.artifact_digest);
  if (artifactError) {
    errors.push(artifactError);
  }

  return errors;
}

function selfTest() {
  const tests = [
    ['valid', SAMPLE_VALID, 'passed'],
    ['missing field', { ...SAMPLE_VALID, source_commit: undefined }, 'failed'],
    ['invalid commit', { ...SAMPLE_VALID, source_commit: 'zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz' }, 'failed'],
    ['invalid digest', { ...SAMPLE_VALID, artifact_digest: { algorithm: 'sha256', value: '11' } }, 'failed'],
    ['non-HTTPS URL', { ...SAMPLE_VALID, pull_request_url: 'http://github.com/owner/repo/pull/123' }, 'failed'],
    ['empty check runs', { ...SAMPLE_VALID, check_runs: [] }, 'failed'],
  ];

  let passed = 0;
  const records = [];

  for (const [name, input, expected] of tests) {
    const errors = validateChecklist(input);
    const valid = errors.length === 0;
    const status = valid ? 'passed' : 'failed';
    const ok = status === expected;
    if (!ok) {
      fail(`self-test failed: ${name}`);
    }
    records.push({
      name,
      ok,
      status,
      errors,
    });
    if (ok) {
      passed += 1;
    }
  }

  console.log(JSON.stringify({
    status: `${passed}/${tests.length} tests passed`,
    tests: records,
  }));
}

function main() {
  const args = process.argv.slice(2);
  if (args.length === 1 && args[0] === '--self-test') {
    selfTest();
    return;
  }

  if (args.length !== 1) {
    fail('Usage: node scripts/validate-evidence-checklist.mjs <checklist.json>');
  }

  const evidence = readChecklist(args[0]);
  const errors = validateChecklist(evidence);
  if (errors.length > 0) {
    console.error(JSON.stringify({ valid: false, errors }, null, 2));
    fail('validation failed');
  }

  console.log(JSON.stringify({
    valid: true,
    schema: SCHEMA_ID,
    repository: evidence.repository,
    checks: evidence.check_runs.length,
  }, null, 2));
}

main();
