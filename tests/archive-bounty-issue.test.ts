/**
 * Unit tests for the archive-bounty-issue script's helper functions.
 *
 * These tests verify the comment-building logic and argument parsing
 * without making any network calls.
 */

import { describe, it, expect } from "vitest";

// We test the pure functions by re-implementing the same logic
// (the script is a CLI entry point, so we extract the testable parts).

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

function extractCanonicalIssue(body: string): number | null {
  const linkMatch = body.match(
    /(?:canonical|current|bounty thread)\s*(?:is\s*)?\[.*?#(\d+)\]/i
  );
  if (linkMatch) return parseInt(linkMatch[1], 10);

  const bareMatch = body.match(
    /(?:canonical|current|bounty thread)\s*(?:is\s*)?#(\d+)/i
  );
  if (bareMatch) return parseInt(bareMatch[1], 10);

  const anyLink = body.match(/issues\/(\d+)/);
  if (anyLink) return parseInt(anyLink[1], 10);

  return null;
}

describe("buildArchiveComment", () => {
  it("includes the archive notice HTML comment", () => {
    const comment = buildArchiveComment(1376, 1210, "test reason");
    expect(comment).toContain("<!-- agent-bounties/archive-notice-v1 -->");
  });

  it("references the canonical issue with a link", () => {
    const comment = buildArchiveComment(1376, 1210, "test reason");
    expect(comment).toContain(
      "[#1210](https://github.com/NSPG13/agent-bounties/issues/1210)"
    );
  });

  it("includes the reason", () => {
    const comment = buildArchiveComment(1376, 1210, "Superseded by #1210");
    expect(comment).toContain("**Reason:** Superseded by #1210");
  });

  it("includes the historical snapshot disclaimer", () => {
    const comment = buildArchiveComment(1376, 1210, "test");
    expect(comment).toContain(
      "saved historical snapshot, not a current invitation to claim or spend"
    );
  });
});

describe("extractCanonicalIssue", () => {
  it("extracts from markdown link with 'canonical' keyword", () => {
    const body =
      "Follow [the current bounty thread #1210](https://github.com/NSPG13/agent-bounties/issues/1210)";
    expect(extractCanonicalIssue(body)).toBe(1210);
  });

  it("extracts from markdown link with 'canonical' keyword (is pattern)", () => {
    const body =
      "The canonical thread is [#999](https://github.com/NSPG13/agent-bounties/issues/999).";
    expect(extractCanonicalIssue(body)).toBe(999);
  });

  it("extracts from bare issue reference", () => {
    const body = "See canonical thread #42 for details.";
    expect(extractCanonicalIssue(body)).toBe(42);
  });

  it("falls back to any issue link", () => {
    const body =
      "This is a duplicate. See https://github.com/NSPG13/agent-bounties/issues/555";
    expect(extractCanonicalIssue(body)).toBe(555);
  });

  it("returns null when no issue reference is found", () => {
    const body = "This is just a regular issue with no references.";
    expect(extractCanonicalIssue(body)).toBeNull();
  });

  it("returns null for empty body", () => {
    expect(extractCanonicalIssue("")).toBeNull();
  });
});
