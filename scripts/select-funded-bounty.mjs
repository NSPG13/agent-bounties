#!/usr/bin/env node

import { readFileSync } from 'fs';

function parseFeed(feedPath) {
    try {
        const feedContent = readFileSync(feedPath, 'utf8');
        return JSON.parse(feedContent);
    } catch (error) {
        console.error('Error reading or parsing feed:', error.message);
        process.exit(2);
    }
}

function isValidEntry(entry, agentWallet) {
    return (
        entry &&
        entry.id &&
        entry.creator_wallet &&
        entry.creator_wallet !== agentWallet &&
        entry.solver_reward > 0 &&
        entry.claim_bond > 0 &&
        entry.is_funded &&
        entry.is_claimable &&
        entry.terms_valid &&
        entry.verification_ready
    );
}

function rankEntries(entries) {
    return entries.sort((a, b) => {
        if (a.solver_reward !== b.solver_reward) {
            return b.solver_reward - a.solver_reward;
        }
        if (a.claim_bond !== b.claim_bond) {
            return a.claim_bond - b.claim_bond;
        }
        return a.id.localeCompare(b.id);
    });
}

function selectBestBounty(feedPath, agentWallet) {
    const feed = parseFeed(feedPath);
    if (!Array.isArray(feed)) {
        console.error('Invalid feed format: expected an array of entries');
        process.exit(2);
    }

    const validEntries = feed.filter(entry => isValidEntry(entry, agentWallet));
    if (validEntries.length === 0) {
        process.exit(1);
    }

    const rankedEntries = rankEntries(validEntries);
    const bestBounty = rankedEntries[0];

    console.log(JSON.stringify({
        bounty_id: bestBounty.id,
        solver_reward: bestBounty.solver_reward,
        claim_bond: bestBounty.claim_bond,
        next_action: 'agent_native_claim'
    }));
}

if (process.argv.length !== 4) {
    console.error('Usage: node select-funded-bounty.mjs <feed-path> <agent-wallet>');
    process.exit(2);
}

const [, , feedPath, agentWallet] = process.argv;
selectBestBounty(feedPath, agentWallet);