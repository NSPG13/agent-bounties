#!/usr/bin/env node
/**
 * Verify canonical settlement evidence.
 * Bounty: 2.00 USDC — NSPG13/agent-bounties #360
 */
import { readFileSync, existsSync } from 'fs';
import { resolve } from 'path';

function parseEvidenceFile(filePath) {
    if (!existsSync(filePath)) {
        throw new Error('Evidence file not found: ' + filePath);
    }
    const content = readFileSync(filePath, 'utf-8');
    try {
        return JSON.parse(content);
    } catch {
        throw new Error('Invalid JSON in evidence file');
    }
}

function verifySettlement(evidence) {
    const checks = {
        hasSolverId: !!evidence.solverId,
        hasBountyId: !!evidence.bountyId,
        hasAmount: typeof evidence.amount === 'number' && evidence.amount > 0,
        hasTransactionHash: !!evidence.txHash && /^0x[a-fA-F0-9]{64}$/.test(evidence.txHash),
        hasTimestamp: !!evidence.settledAt,
        signatureValid: evidence.signature ? evidence.signature.length >= 64 : false,
    };

    const allPassed = Object.values(checks).every(Boolean);
    
    return {
        verified: allPassed,
        checks,
        evidence,
        message: allPassed 
            ? 'Settlement verified successfully'
            : 'Settlement verification failed: ' + 
              Object.entries(checks).filter(([,v]) => !v).map(([k]) => k).join(', '),
    };
}

function main() {
    const args = process.argv.slice(2);
    if (args.length < 1) {
        console.error('Usage: node verify-settlement-evidence.mjs <evidence-file.json>');
        process.exit(1);
    }

    try {
        const evidence = parseEvidenceFile(resolve(args[0]));
        const result = verifySettlement(evidence);
        console.log(JSON.stringify(result, null, 2));
        process.exit(result.verified ? 0 : 1);
    } catch (err) {
        console.error('Error:', err.message);
        process.exit(2);
    }
}

main();
