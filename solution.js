#!/usr/bin/env node

// scripts/select-funded-bounty.mjs
// Selects the safest funded bounty from a canonical feed
// Dependency-free Node.js CLI for autonomous agents

const fs = require('fs');
const path = require('path');
const https = require('https');
const http = require('http');

// __dirname available natively in CommonJS

// Constants
const BOUNTY_FEED_URL = process.env.BOUNTY_FEED_URL || 'https://raw.githubusercontent.com/example/bounties/main/feed.json';
const MIN_FUNDING_USDC = 2.00;
const CLAIM_BOND_USDC = 0.10;
const VERIFIER_REWARD_USDC = 0.10;

// Helper: fetch JSON with built-in https
async function fetchJSON(url) {
  return new Promise((resolve, reject) => {
    const protocol = url.startsWith('https') ? https : http;
    const req = protocol.get(url, (res) => {
      let data = '';
      res.on('data', chunk => data += chunk);
      res.on('end', () => {
        try {
          resolve(JSON.parse(data));
        } catch (e) {
          reject(new Error(`Invalid JSON: ${e.message}`));
        }
      });
    });
    req.on('error', reject);
    req.end();
  });
}

// Helper: validate bounty structure
function isValidBounty(bounty) {
  return (
    bounty &&
    typeof bounty.id === 'string' &&
    typeof bounty.title === 'string' &&
    typeof bounty.funding_usdc === 'number' &&
    bounty.funding_usdc >= MIN_FUNDING_USDC &&
    typeof bounty.status === 'string' &&
    bounty.status === 'open' &&
    (!bounty.labels || Array.isArray(bounty.labels)) &&
    (!bounty.requirements || typeof bounty.requirements === 'object')
  );
}

// Safety scoring: higher is safer
function calculateSafetyScore(bounty) {
  let score = 0;
  
  // Funding amount (higher funding = more reliable)
  score += Math.min(bounty.funding_usdc / 10, 10);
  
  // Has verifier reward (indicates verification process)
  if (bounty.verifier_reward_usdc && bounty.verifier_reward_usdc >= VERIFIER_REWARD_USDC) {
    score += 5;
  }
  
  // Has claim bond
  if (bounty.claim_bond_usdc && bounty.claim_bond_usdc >= CLAIM_BOND_USDC) {
    score += 3;
  }
  
  // Has specific requirements (clear scope)
  if (bounty.requirements && Object.keys(bounty.requirements).length > 0) {
    score += 4;
  }
  
  // Has labels (better described)
  if (bounty.labels && bounty.labels.length > 0) {
    score += 2;
  }
  
  // Has a deadline (time-bound bounties are safer)
  if (bounty.deadline) {
    score += 3;
  }
  
  // Has canonical commit reference (immutability)
  if (bounty.commit_sha) {
    score += 5;
  }
  
  // Lower risk: no external dependencies
  if (bounty.allow_external_dependencies === false) {
    score += 2;
  }
  
  return score;
}

// Main selection logic
async function selectSafestBounty() {
  try {
    // Fetch bounty feed
    const feed = await fetchJSON(BOUNTY_FEED_URL);
    
    if (!Array.isArray(feed)) {
      throw new Error('Feed must be an array of bounties');
    }
    
    // Filter valid bounties
    const validBounties = feed.filter(isValidBounty);
    
    if (validBounties.length === 0) {
      console.log('No valid funded bounties found');
      process.exit(1);
    }
    
    // Score and select safest
    let safestBounty = null;
    let highestScore = -1;
    
    for (const bounty of validBounties) {
      const score = calculateSafetyScore(bounty);
      if (score > highestScore) {
        highestScore = score;
        safestBounty = bounty;
      }
    }
    
    // Output result as JSON for agent consumption
    const result = {
      selected_bounty: safestBounty,
      safety_score: highestScore,
      reason: `Selected bounty "${safestBounty.title}" with funding ${safestBounty.funding_usdc} USDC and safety score ${highestScore}`
    };
    
    console.log(JSON.stringify(result, null, 2));
    
  } catch (error) {
    console.error(`Error: ${error.message}`);
    process.exit(1);
  }
}

// Execute
selectSafestBounty();