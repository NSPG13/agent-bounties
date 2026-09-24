#!/usr/bin/env node
/**
 * Find open bounty issues whose body contains an "Archived duplicate"
 * notice and a link to a canonical issue, then archive them.
 *
 * This is a heuristic scanner: it looks for the pattern
 *   "Archived duplicate" + a link to another issue in the same repo
 * and, if found, runs the archive-bounty-issue.ts script for each match.
 *
 * Usage:
 *   node scripts/find-duplicate-bounties.mjs \
 *     --owner NSPG13 --repo agent-bounties [--dry-run]
 */

import { Octokit } from "@octokit/rest";
import { execSync } from "node:child_process";
import process from "node:process";

const ARCHIVE_LABEL = "archived-duplicate";
const BOUNTY_LABEL = "bounty";

function parseArgs(argv) {
  const args = {};
  for (let i = 2; i < argv.length; i++) {
    const arg = argv[i];
    if (arg.startsWith("--")) {
      const key = arg.slice(2);
      const value = argv[i + 1];
      if (value && !value.startsWith("--")) {
        args[key] = value;
        i++;
      } else {
        args[key] = "true";
      }
    }
  }
  return args;
}

/**
 * Extract the canonical issue number from an issue body.
 * Looks for patterns like:
 *   - "Follow [the current bounty thread #1210](...)"
 *   - "canonical thread is [#1210](...)"
 *   - "See issue #1210"
 */
function extractCanonicalIssue(body) {
  // Pattern 1: markdown link with issue number in text
  const linkMatch = body.match(
    /(?:canonical|current|bounty thread)\s*(?:is\s*)?\[.*?#(\d+)\]/i
  );
  if (linkMatch) return parseInt(linkMatch[1], 10);

  // Pattern 2: bare issue reference
  const bareMatch = body.match(/(?:canonical|current|bounty thread)\s*(?:is\s*)?#(\d+)/i);
  if (bareMatch) return parseInt(bareMatch[1], 10);

  // Pattern 3: any issue link in the body (fallback — first one found)
  const anyLink = body.match(/issues\/(\d+)/);
  if (anyLink) return parseInt(anyLink[1], 10);

  return null;
}

async function main() {
  const args = parseArgs(process.argv);
  const owner = args["owner"] || process.env["GITHUB_REPOSITORY_OWNER"];
  const repo = args["repo"] || process.env["GITHUB_REPOSITORY"]?.split("/")[1];
  const dryRun = args["dry-run"] === "true";

  if (!owner || !repo) {
    console.error("Error: --owner and --repo are required (or set GITHUB_REPOSITORY).");
    process.exit(1);
  }

  const token = process.env["GITHUB_TOKEN"];
  if (!token) {
    console.error("Error: GITHUB_TOKEN is required.");
    process.exit(1);
  }

  const octokit = new Octokit({ auth: token });

  console.log(`Scanning ${owner}/${repo} for duplicate bounty issues...`);

  // Fetch open issues with the bounty label
  const { data: issues } = await octokit.issues.listForRepo({
    owner,
    repo,
    labels: BOUNTY_LABEL,
    state: "open",
    per_page: 100,
  });

  let archived = 0;
  let skipped = 0;

  for (const issue of issues) {
    const labels = issue.labels.map((l) =>
      typeof l === "string" ? l : l.name
    );

    // Skip if already archived
    if (labels.includes(ARCHIVE_LABEL)) {
      skipped++;
      continue;
    }

    // Check if the body contains an "Archived duplicate" notice
    const body = issue.body || "";
    if (!/archived\s+duplicate/i.test(body)) {
      skipped++;
      continue;
    }

    // Extract the canonical issue number
    const canonicalIssue = extractCanonicalIssue(body);
    if (!canonicalIssue || canonicalIssue === issue.number) {
      console.log(
        `  ⚠ Issue #${issue.number}: "Archived duplicate" found but no valid canonical issue reference. Skipping.`
      );
      skipped++;
      continue;
    }

    console.log(
      `  → Issue #${issue.number} is a duplicate of #${canonicalIssue}. Archiving...`
    );

    if (dryRun) {
      console.log(`    [dry-run] Would archive issue #${issue.number}.`);
      archived++;
      continue;
    }

    // Run the archive script
    const cmd =
      `npx tsx scripts/archive-bounty-issue.ts ` +
      `--owner ${owner} --repo ${repo} ` +
      `--issue-number ${issue.number} ` +
      `--canonical-issue ${canonicalIssue} ` +
      `--reason "Superseded by canonical thread #${canonicalIssue}"`;

    try {
      execSync(cmd, { stdio: "inherit", env: process.env });
      archived++;
    } catch (err) {
      console.error(`  ✗ Failed to archive issue #${issue.number}: ${err.message}`);
      skipped++;
    }
  }

  console.log(`\nDone. Archived: ${archived}, Skipped: ${skipped}.`);
}

main().catch((err) => {
  console.error("Fatal error:", err);
  process.exit(1);
});
