#!/usr/bin/env node

import { readFileSync } from 'fs';

async function verifySettlementEvidence() {
    // Parse command line arguments
    const args = process.argv.slice(2);
    if (args.length !== 3) {
        console.error('Usage: node verify-settlement-evidence.mjs <feed-item-path> <expected-bounty-contract> <expected-solver-wallet>');
        process.exit(1);
    }

    const [feedItemPath, expectedBountyContract, expectedSolverWallet] = args;

    try {
        // Read and parse the feed item JSON
        const feedItemContent = readFileSync(feedItemPath, 'utf8');
        const feedItem = JSON.parse(feedItemContent);

        // Check for exactly one bounty_settled event
        const settledEvents = feedItem.events.filter(event => event.name === 'bounty_settled');
        if (settledEvents.length !== 1) {
            console.log(JSON.stringify({ paid: false }));
            process.exit(0);
        }

        const event = settledEvents[0];

        // Validate all required fields
        if (
            event.bountyId !== feedItem.bountyId ||
            event.contract !== expectedBountyContract ||
            event.solver !== expectedSolverWallet ||
            event.round <= 0 ||
            event.logIndex < 0 ||
            event.solverReward <= 0 ||
            event.returnedBond <= 0 ||
            event.verifierReward <= 0 ||
            event.timeoutBonus <= 0 ||
            !event.hash1 || !event.hash2 || !event.hash3 || !event.hash4
        ) {
            console.log(JSON.stringify({ paid: false }));
            process.exit(0);
        }

        // Validate payout arithmetic
        const totalPayout = event.solverReward + event.returnedBond + event.verifierReward + event.timeoutBonus;
        if (totalPayout !== event.totalPayout) {
            console.log(JSON.stringify({ paid: false }));
            process.exit(0);
        }

        // If all checks pass
        console.log(JSON.stringify({ paid: true }));
    } catch (error) {
        console.error('Error:', error.message);
        console.log(JSON.stringify({ paid: false }));
        process.exit(1);
    }
}

verifySettlementEvidence();