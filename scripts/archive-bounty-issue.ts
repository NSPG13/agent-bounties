#!/usr/bin/env npx tsx
/**
 * Archive a duplicate bounty issue by:
 * 1. Adding the "archived-duplicate" label
 * 2. Adding a cross-reference comment to the canonical issue
 * 3. Closing the issue with a standard archival message
 *
 * Usage:
 *   npx tsx scripts/archive-bounty-issue.ts \
 *     --owner NSPG13 \
 *     --repo agent-bounties \
 *     --issue-number 1376 \
 *     --canonical-issue 1210 \
 *     --reason "Superseded by canonical thread #1210"
 *
 * Requires GITHUB_TOKEN env var with repo scope.
 */

import { Octokit } from "@octokit/rest";

interface ArchiveOptions {
  owner: string;
  repo: string;
  issueNumber: number;
  canonicalIssue: number;
  reason: string;
  dryRun?: boolean;
}

const ARCHIVE_LABEL = "archived-duplicate";
const BOUNTY_LABEL = "bounty";

function parseArgs(argv: string[]): ArchiveOptions {
  const args: Record<string, string> = {};
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

  const owner = args["owner"];
  const repo = args["repo"];
  const issueNumber = parseInt(args["issue-number"] ?? "", 10);
  const canonicalIssue = parseInt(args["canonical-issue"] ?? "", 10);
  const reason = args["reason"] ?? "Archived as duplicate";
  const dryRun = args["dry-run"] === "true";

  if (!owner || !repo || isNaN(issueNumber) || isNaN(canonicalIssue)) {
    console.error(
      "Usage: npx tsx scripts/archive-bounty-issue.ts " +
        "--owner <owner> --repo <repo> " +
        "--issue-number <n> --canonical-issue <n> " +
        "--reason <text> [--dry-run]"
    );
    process.exit(1);
  }

  return { owner, repo, issueNumber, canonicalIssue, reason, dryRun };
}

function buildArchiveComment(
  issueNumber: number,
  canonicalIssue: number,
  reason: string
): string {
  return [
    `<!-- agent-bounties/archive-notice-v1 -->`,
    `## 📦 Archived Duplicate`,
    ``,
    `This issue is an **archived duplicate**. The canonical bounty thread is ` +
      `[#${canonicalIssue}](https://github.com/NSPG13/agent-bounties/issues/${canonicalIssue}).`,
    ``,
    `**Reason:** ${reason}`,
    ``,
    `> Existing work, comments, and payment evidence remain here for reference.`,
    `> This is a saved historical snapshot, not a current invitation to claim or spend.`,
    ``,
    `---`,
    `_Automatically archived by the bounty issue management tooling._`,
  ].join("\n");
}

async function main(): Promise<void> {
  const options = parseArgs(process.argv);
  const token = process.env["GITHUB_TOKEN"];

  if (!token) {
    console.error("Error: GITHUB_TOKEN environment variable is required.");
    process.exit(1);
  }

  const octokit = new Octokit({ auth: token });

  console.log(`Archiving issue #${options.issueNumber} in ${options.owner}/${options.repo}`);
  console.log(`Canonical issue: #${options.canonicalIssue}`);
  console.log(`Reason: ${options.reason}`);
  if (options.dryRun) {
    console.log("DRY RUN — no changes will be made.");
  }

  // 1. Fetch the issue to verify it exists and is open
  const { data: issue } = await octokit.issues.get({
    owner: options.owner,
    repo: options.repo,
    issue_number: options.issueNumber,
  });

  if (issue.state === "closed") {
    console.log(`Issue #${options.issueNumber} is already closed. Skipping.`);
    return;
  }

  // 2. Add the archived-duplicate label
  const existingLabels = issue.labels.map((l) =>
    typeof l === "string" ? l : l.name
  );

  if (!existingLabels.includes(ARCHIVE_LABEL)) {
    if (!options.dryRun) {
      await octokit.issues.addLabels({
        owner: options.owner,
        repo: options.repo,
        issue_number: options.issueNumber,
        labels: [ARCHIVE_LABEL],
      });
      console.log(`Added label: ${ARCHIVE_LABEL}`);
    } else {
      console.log(`[dry-run] Would add label: ${ARCHIVE_LABEL}`);
    }
  } else {
    console.log(`Label ${ARCHIVE_LABEL} already present.`);
  }

  // 3. Add the archive comment
  const commentBody = buildArchiveComment(
    options.issueNumber,
    options.canonicalIssue,
    options.reason
  );

  if (!options.dryRun) {
    await octokit.issues.createComment({
      owner: options.owner,
      repo: options.repo,
      issue_number: options.issueNumber,
      body: commentBody,
    });
    console.log(`Added archive comment to issue #${options.issueNumber}`);
  } else {
    console.log(`[dry-run] Would add archive comment:\n${commentBody}`);
  }

  // 4. Add a cross-reference comment on the canonical issue
  const crossRefBody = [
    `<!-- agent-bounties/cross-ref-v1 -->`,
    `> 📎 Issue [#${options.issueNumber}](https://github.com/${options.owner}/${options.repo}/issues/${options.issueNumber}) ` +
      `has been archived as a duplicate of this thread.`,
    `>`,
    `> **Reason:** ${options.reason}`,
  ].join("\n");

  if (!options.dryRun) {
    await octokit.issues.createComment({
      owner: options.owner,
      repo: options.repo,
      issue_number: options.canonicalIssue,
      body: crossRefBody,
    });
    console.log(`Added cross-reference comment to issue #${options.canonicalIssue}`);
  } else {
    console.log(`[dry-run] Would add cross-reference to issue #${options.canonicalIssue}`);
  }

  // 5. Close the duplicate issue
  if (!options.dryRun) {
    await octokit.issues.update({
      owner: options.owner,
      repo: options.repo,
      issue_number: options.issueNumber,
      state: "closed",
      state_reason: "completed",
    });
    console.log(`Closed issue #${options.issueNumber}`);
  } else {
    console.log(`[dry-run] Would close issue #${options.issueNumber}`);
  }

  console.log("\n✅ Archive complete.");
}

main().catch((err) => {
  console.error("Fatal error:", err);
  process.exit(1);
});
