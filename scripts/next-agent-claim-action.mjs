#!/usr/bin/env node
/**
 * Convert Agent Bounties claim response into safe next action.
 * Bounty: 2.00 USDC — NSPG13/agent-bounties #358
 */

const VALID_ACTIONS = [
    'submit-solution',
    'wait-for-review',
    'claim-reward',
    'verify-payment',
    'report-error',
    'escalate-dispute',
    'retry-submission',
    'abandon-bounty',
];

function parseClaimResponse(response) {
    try {
        return typeof response === 'string' ? JSON.parse(response) : response;
    } catch {
        throw new Error('Invalid JSON in claim response');
    }
}

function determineNextAction(claim) {
    // Priority-ordered action determination
    if (claim.status === 'accepted' && claim.requiresSubmission) {
        return { action: 'submit-solution', reason: 'Bounty accepted, solution required' };
    }
    if (claim.status === 'submitted' || claim.status === 'pending_review') {
        return { action: 'wait-for-review', reason: 'Solution submitted, awaiting review' };
    }
    if (claim.status === 'approved' && claim.rewardPending) {
        return { action: 'claim-reward', reason: 'Solution approved, claim reward' };
    }
    if (claim.status === 'paid' && claim.settlementPending) {
        return { action: 'verify-payment', reason: 'Reward claimed, verify settlement' };
    }
    if (claim.status === 'rejected') {
        return { action: 'report-error', reason: 'Submission rejected', details: claim.rejectionReason };
    }
    if (claim.status === 'disputed') {
        return { action: 'escalate-dispute', reason: 'Dispute raised', details: claim.disputeReason };
    }
    if (claim.status === 'error' || claim.status === 'failed') {
        if (claim.retryCount && claim.retryCount < 3) {
            return { action: 'retry-submission', reason: 'Error occurred, retry available', retryCount: claim.retryCount };
        }
        return { action: 'abandon-bounty', reason: 'Max retries exceeded' };
    }
    
    // Default: wait
    return { action: 'wait-for-review', reason: 'Unknown state, defaulting to wait', rawStatus: claim.status };
}

function main() {
    const args = process.argv.slice(2);
    let input = '';

    if (args.length > 0) {
        const { readFileSync, existsSync } = await import('fs');
        const { resolve } = await import('path');
        const filePath = resolve(args[0]);
        if (existsSync(filePath)) {
            input = readFileSync(filePath, 'utf-8');
        }
    }

    if (!input) {
        // Read from stdin
        const chunks = [];
        process.stdin.setEncoding('utf-8');
        process.stdin.on('data', chunk => chunks.push(chunk));
        process.stdin.on('end', () => {
            try {
                const claim = parseClaimResponse(chunks.join(''));
                const result = determineNextAction(claim);
                console.log(JSON.stringify(result, null, 2));
                process.exit(VALID_ACTIONS.includes(result.action) ? 0 : 1);
            } catch (err) {
                console.error('Error:', err.message);
                process.exit(2);
            }
        });
        return;
    }

    try {
        const claim = parseClaimResponse(input);
        const result = determineNextAction(claim);
        console.log(JSON.stringify(result, null, 2));
        process.exit(VALID_ACTIONS.includes(result.action) ? 0 : 1);
    } catch (err) {
        console.error('Error:', err.message);
        process.exit(2);
    }
}

main();
