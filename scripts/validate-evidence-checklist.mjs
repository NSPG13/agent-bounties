#!/usr/bin/env node
import { readFileSync } from "node:fs";

const SCHEMA_PATH = new URL("../schemas/direct-evidence-checklist-v1.json", import.meta.url).pathname;
const schema = JSON.parse(readFileSync(SCHEMA_PATH, "utf8"));

function validate(instance, path = "") {
  const errors = [];

  if (!instance || typeof instance !== "object" || Array.isArray(instance)) {
    errors.push(`root_object_required`);
    return { ok: false, errors };
  }

  const allowedKeys = new Set(Object.keys(schema.properties));
  for (const key of Object.keys(instance)) {
    if (!allowedKeys.has(key)) {
      errors.push(`unknown_property:${path}${key}`);
    }
  }

  for (const field of schema.required) {
    if (!(field in instance) || instance[field] === null || instance[field] === undefined) {
      errors.push(`missing_required:${path}${field}`);
    } else {
      const value = instance[field];
      const propSchema = schema.properties[field];
      const fieldPath = `${path}${field}.`;

      switch (field) {
        case "schema_version":
          if (value !== propSchema.const) errors.push(`invalid_schema_version:${fieldPath}`);
          break;
        case "bounty_id":
          if (typeof value !== "string" || !value.trim()) errors.push(`empty_field:${fieldPath}`);
          break;
        case "submission_evidence":
          errors.push(...validateSubmission(value, fieldPath));
          break;
        case "verification_evidence":
          errors.push(...validateVerification(value, fieldPath));
          break;
        case "payment_evidence":
          errors.push(...validatePayment(value, fieldPath));
          break;
        default:
          if (!value) errors.push(`empty_field:${fieldPath}`);
      }
    }
  }

  return { ok: errors.length === 0, errors };
}

function rejectUnknown(obj, allowed, path) {
  const errs = [];
  for (const key of Object.keys(obj)) {
    if (!allowed.has(key)) errs.push(`unknown_property:${path}${key}`);
  }
  return errs;
}

function validateSubmission(obj, path) {
  const errs = [];
  if (!obj || typeof obj !== "object") return [ `invalid_object:${path}` ];
  errs.push(...rejectUnknown(obj, new Set(["commit","repository","subdirectory","pull_request_url"]), path));
  for (const f of ["commit","repository","subdirectory","pull_request_url"]) {
    if (!(f in obj) || obj[f] === null || obj[f] === undefined) errs.push(`missing_required:${path}${f}`);
  }
  if (typeof obj.commit === "string" && !/^[0-9a-f]{40}$/.test(obj.commit)) errs.push(`invalid_commit:${path}commit`);
  if (typeof obj.repository === "string" && !/^https:\/\/github\.com\/[^/]+\/[^/]+$/.test(obj.repository)) errs.push(`invalid_repository:${path}repository`);
  if (typeof obj.subdirectory === "string" && !obj.subdirectory.trim()) errs.push(`empty_subdirectory:${path}subdirectory`);
  if (typeof obj.pull_request_url === "string" && !/^https:\/\/github\.com\/[^/]+\/[^/]+\/pull\/[1-9][0-9]*$/.test(obj.pull_request_url)) errs.push(`invalid_pr_url:${path}pull_request_url`);
  return errs;
}

function validateVerification(obj, path) {
  const errs = [];
  if (!obj || typeof obj !== "object") return [ `invalid_object:${path}` ];
  errs.push(...rejectUnknown(obj, new Set(["check_run_urls","artifact"]), path));
  if (!("check_run_urls" in obj) || !Array.isArray(obj.check_run_urls) || obj.check_run_urls.length === 0) {
    errs.push(`missing_or_empty_check_run_urls:${path}check_run_urls`);
  } else {
    for (let i = 0; i < obj.check_run_urls.length; i++) {
      const u = obj.check_run_urls[i];
      if (typeof u !== "string" || !/^https:\/\//.test(u)) errs.push(`non_https_check_run_url:${path}check_run_urls[${i}]`);
    }
  }
  if (!("artifact" in obj) || !obj.artifact || typeof obj.artifact !== "object") {
    errs.push(`missing_artifact:${path}artifact`);
  } else {
    const art = obj.artifact;
    errs.push(...rejectUnknown(art, new Set(["url","digest"]), `${path}artifact.`));
    if (!("url" in art) || typeof art.url !== "string" || !/^https:\/\//.test(art.url)) errs.push(`missing_or_non_https_artifact_url:${path}artifact.url`);
    if (!("digest" in art) || !art.digest || typeof art.digest !== "object") {
      errs.push(`missing_artifact_digest:${path}artifact.digest`);
    } else {
      const d = art.digest;
      errs.push(...rejectUnknown(d, new Set(["algorithm","value"]), `${path}artifact.digest.`));
      if (!d.algorithm || !["sha256","sha512"].includes(d.algorithm)) errs.push(`invalid_algorithm:${path}artifact.digest.algorithm`);
      if (typeof d.value !== "string" || !/^[0-9a-f]{64,128}$/.test(d.value)) errs.push(`invalid_digest_value:${path}artifact.digest.value`);
      else if (d.algorithm === "sha256" && d.value.length !== 64) errs.push(`digest_length_mismatch_sha256:${path}artifact.digest.value`);
      else if (d.algorithm === "sha512" && d.value.length !== 128) errs.push(`digest_length_mismatch_sha512:${path}artifact.digest.value`);
    }
  }
  return errs;
}

function validatePayment(obj, path) {
  const errs = [];
  if (!obj || typeof obj !== "object") return [ `invalid_object:${path}` ];
  errs.push(...rejectUnknown(obj, new Set(["canonical_settlement"]), path));
  if (!("canonical_settlement" in obj) || !obj.canonical_settlement || typeof obj.canonical_settlement !== "object") {
    errs.push(`missing_canonical_settlement:${path}canonical_settlement`);
  } else {
    const cs = obj.canonical_settlement;
    errs.push(...rejectUnknown(cs, new Set(["transaction_hash"]), `${path}canonical_settlement.`));
    if (!("transaction_hash" in cs) || typeof cs.transaction_hash !== "string" || !/^0x[0-9a-fA-F]{64}$/.test(cs.transaction_hash)) errs.push(`invalid_tx_hash:${path}canonical_settlement.transaction_hash`);
  }
  return errs;
}

// --- CLI / self-test --------------------------------------------------------
const argv = process.argv.slice(2);
if (argv.includes("--self-test")) {
  const tests = {
    valid: {
      instance: {
        schema_version: "agent-bounties/direct-evidence-checklist-v1",
        bounty_id: "test-686",
        submission_evidence: {
          commit: "a".repeat(40),
          repository: "https://github.com/NSPG13/agent-bounties",
          subdirectory: "scripts",
          pull_request_url: "https://github.com/NSPG13/agent-bounties/pull/9999",
        },
        verification_evidence: {
          check_run_urls: ["https://github.com/NSPG13/agent-bounties/actions/runs/123"],
          artifact: {
            url: "https://example.com/artifact.zip",
            digest: { algorithm: "sha256", value: "b".repeat(64) },
          },
        },
        payment_evidence: {
          canonical_settlement: { transaction_hash: "0x" + "c".repeat(64) },
        },
      },
      expected: "ok",
    },
    missing_payment: {
      instance: {
        schema_version: "agent-bounties/direct-evidence-checklist-v1",
        bounty_id: "test-686",
        submission_evidence: {
          commit: "a".repeat(40),
          repository: "https://github.com/NSPG13/agent-bounties",
          subdirectory: "scripts",
          pull_request_url: "https://github.com/NSPG13/agent-bounties/pull/9999",
        },
        verification_evidence: {
          check_run_urls: ["https://github.com/NSPG13/agent-bounties/actions/runs/123"],
          artifact: {
            url: "https://example.com/artifact.zip",
            digest: { algorithm: "sha256", value: "b".repeat(64) },
          },
        },
      },
      expected: "fail:missing_required:payment_evidence",
    },
    non_https_artifact: {
      instance: {
        schema_version: "agent-bounties/direct-evidence-checklist-v1",
        bounty_id: "test-686",
        submission_evidence: {
          commit: "a".repeat(40),
          repository: "https://github.com/NSPG13/agent-bounties",
          subdirectory: "scripts",
          pull_request_url: "https://github.com/NSPG13/agent-bounties/pull/9999",
        },
        verification_evidence: {
          check_run_urls: ["https://github.com/NSPG13/agent-bounties/actions/runs/123"],
          artifact: {
            url: "http://insecure/artifact",
            digest: { algorithm: "sha256", value: "b".repeat(64) },
          },
        },
        payment_evidence: {
          canonical_settlement: { transaction_hash: "0x" + "c".repeat(64) },
        },
      },
      expected: "fail:missing_or_non_https_artifact_url",
    },
    digest_length_sha256: {
      instance: {
        schema_version: "agent-bounties/direct-evidence-checklist-v1",
        bounty_id: "test-686",
        submission_evidence: {
          commit: "a".repeat(40),
          repository: "https://github.com/NSPG13/agent-bounties",
          subdirectory: "scripts",
          pull_request_url: "https://github.com/NSPG13/agent-bounties/pull/9999",
        },
        verification_evidence: {
          check_run_urls: ["https://github.com/NSPG13/agent-bounties/actions/runs/123"],
          artifact: {
            url: "https://example.com/artifact.zip",
            digest: { algorithm: "sha256", value: "b".repeat(128) },
          },
        },
        payment_evidence: {
          canonical_settlement: { transaction_hash: "0x" + "c".repeat(64) },
        },
      },
      expected: "fail:digest_length_mismatch_sha256",
    },
    unknown_property: {
      instance: {
        schema_version: "agent-bounties/direct-evidence-checklist-v1",
        bounty_id: "test-686",
        extra_field: "should be rejected",
        submission_evidence: {
          commit: "a".repeat(40),
          repository: "https://github.com/NSPG13/agent-bounties",
          subdirectory: "scripts",
          pull_request_url: "https://github.com/NSPG13/agent-bounties/pull/9999",
        },
        verification_evidence: {
          check_run_urls: ["https://github.com/NSPG13/agent-bounties/actions/runs/123"],
          artifact: {
            url: "https://example.com/artifact.zip",
            digest: { algorithm: "sha256", value: "b".repeat(64) },
          },
        },
        payment_evidence: {
          canonical_settlement: { transaction_hash: "0x" + "c".repeat(64) },
        },
      },
      expected: "fail:unknown_property:extra_field",
    },
    check_run_only_no_payment: {
      instance: {
        schema_version: "agent-bounties/direct-evidence-checklist-v1",
        bounty_id: "test-686",
        submission_evidence: {
          commit: "a".repeat(40),
          repository: "https://github.com/NSPG13/agent-bounties",
          subdirectory: "scripts",
          pull_request_url: "https://github.com/NSPG13/agent-bounties/pull/9999",
        },
        verification_evidence: {
          check_run_urls: ["https://github.com/NSPG13/agent-bounties/actions/runs/123"],
          artifact: {
            url: "https://example.com/artifact.zip",
            digest: { algorithm: "sha256", value: "b".repeat(64) },
          },
        },
        payment_evidence: {
          canonical_settlement: { transaction_hash: "0x" + "d".repeat(64) },
        },
      },
      expected: "ok",
    },
  };

  let passed = 0, failed = 0;
  for (const [name, { instance, expected }] of Object.entries(tests)) {
    const result = validate(instance);
    if (expected === "ok") {
      if (result.ok) {
        console.log(`PASS ${name}`);
        passed++;
      } else {
        console.error(`FAIL ${name}: expected ok, got errors: ${result.errors.join("; ")}`);
        failed++;
      }
    } else {
      const prefix = expected.replace(/^fail:/, "");
      if (!result.ok && result.errors.some(e => e.startsWith(prefix) || e.includes(prefix))) {
        console.log(`PASS ${name} -> ${result.errors.join("; ")}`);
        passed++;
      } else {
        console.error(`FAIL ${name}: expected ${expected}, got ${JSON.stringify(result)}`);
        failed++;
      }
    }
  }
  console.log(`\n${passed}/${passed + failed} tests passed`);
  if (failed > 0) process.exit(1);
} else if (argv.length === 1) {
  const instance = JSON.parse(readFileSync(argv[0], "utf8"));
  console.log(JSON.stringify(validate(instance), null, 2));
} else {
  console.log("Usage: node validate-evidence-checklist.mjs <checklist.json>");
  console.log("       node validate-evidence-checklist.mjs --self-test");
}
