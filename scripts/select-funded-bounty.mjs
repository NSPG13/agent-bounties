#!/usr/bin/env node
/**
 * Select the safest funded bounty for an agent from a canonical feed.
 * Bounty: 2.00 USDC — NSPG13/agent-bounties #359
 */
import { readFileSync, existsSync } from 'fs';
import { resolve } from 'path';

const SAFETY_WEIGHTS = {
    verifiedSponsor: 30,
    bountyAgeDays: 5,
    fundedAmount: 10,
    openDuration: 15,
    claimSuccessRate: 40,
};

function loadFeed(filePath) {
    if (!existsSync(filePath)) {
        throw new Error('Feed file not found: ' + filePath);
    }
    const content = readFileSync(filePath, 'utf-8');
    try {
        return JSON.parse(content);
    } catch {
        throw new Error('Invalid JSON in feed file');
    }
}

function scoreBounty(bounty) {
    const now = Date.now();
    const createdAt = new Date(bounty.createdAt || bounty.created_at || now).getTime();
    const ageDays = Math.max(0, (now - createdAt) / (1000 * 60 * 60 * 24));
    
    let score = 0;
    
    // Verified sponsor bonus
    if (bounty.sponsorVerified || bounty.isVerified) {
        score += SAFETY_WEIGHTS.verifiedSponsor;
    }
    
    // Bounty age (older = more established, but not too old)
    if (ageDays > 1 && ageDays < 90) {
        score += SAFETY_WEIGHTS.bountyAgeDays * (1 - ageDays / 180);
    }
    
    // Funded amount (log scale for diminishing returns)
    const amount = bounty.amount || bounty.rewardAmount || 0;
    if (amount > 0) {
        score += SAFETY_WEIGHTS.fundedAmount * Math.min(Math.log10(amount) / 4, 1);
    }
    
    // Open duration remaining
    if (bounty.deadline) {
        const deadline = new Date(bounty.deadline).getTime();
        const remaining = (deadline - now) / (1000 * 60 * 60 * 24);
        if (remaining > 0) {
            score += SAFETY_WEIGHTS.openDuration * Math.min(remaining / 30, 1);
        }
    }
    
    return score;
}

function selectBestBounty(bounties) {
    if (!Array.isArray(bounties) || bounties.length === 0) {
        return { selected: null, message: 'No funded bounties available' };
    }

    const funded = bounties.filter(b => (b.amount || b.rewardAmount || 0) > 0);
    
    if (funded.length === 0) {
        return { selected: null, message: 'No funded bounties available' };
    }

    let best = null;
    let bestScore = -Infinity;

    for (const bounty of funded) {
        const bountyScore = scoreBounty(bounty);
        if (bountyScore > bestScore) {
            bestScore = bountyScore;
            best = bounty;
        }
    }

    return {
        selected: {
            id: best.id || best.bountyId,
            title: best.title,
            amount: best.amount || best.rewardAmount,
            score: Math.round(bestScore * 100) / 100,
            safetyFactors: {
                verified: !!best.sponsorVerified,
                age: new Date(best.createdAt || best.created_at).toISOString(),
                deadline: best.deadline || null,
            }
        },
        alternativesConsidered: funded.length,
        message: 'Best bounty selected by safety score',
    };
}

function main() {
    const args = process.argv.slice(2);
    if (args.length < 1) {
        console.error('Usage: node select-funded-bounty.mjs <feed.json>');
        process.exit(1);
    }

    try {
        const feed = loadFeed(resolve(args[0]));
        const bounties = feed.bounties || feed.items || feed;
        const result = selectBestBounty(bounties);
        console.log(JSON.stringify(result, null, 2));
        process.exit(result.selected ? 0 : 1);
    } catch (err) {
        console.error('Error:', err.message);
        process.exit(2);
    }
}

main();
