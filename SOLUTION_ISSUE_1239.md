# Solution for Issue #1239

## 🛠️ Proposed Solution (by Aditya Waghamare)

### Analysis
The 16-bit leading-zero work proof requirement specifies that the Keccak256 hash of the payload (challenge + nonce) must result in a 256-bit hash where the top 16 bits are strictly zero (`0x0000...` prefix / `hash < 2^240`). To meet deterministic on-chain and off-chain verifier module standards (`leading_zero_work_v1`), we provide an optimized nonce finder and proof verifier implementation in Solidity and Node.js.

### Fix
Implemented proof generation script and verification logic that:
1. Combines the target address/challenge seed with incremental uint256 nonce.
2. Computes `keccak256(abi.encodePacked(challenge, nonce))`.
3. Verifies the upper 16 bits equal zero (`uint16(bytes2(hash)) == 0`).

### Implementation

```javascript
// proof-finder.js - 16-bit leading zero proof generator
const { keccak256, solidityPacked } = require("ethers");

function find16BitWorkProof(challengeHex) {
    let nonce = 0n;
    console.log(`Mining 16-bit leading zero proof for challenge: ${challengeHex}...`);
    
    while (true) {
        const hash = keccak256(solidityPacked(["bytes32", "uint256"], [challengeHex, nonce]));
        
        // 16 bits = 2 bytes = top 4 hex characters after '0x' must be '0000'
        if (hash.startsWith("0x0000")) {
            console.log(`[FOUND PROOF] Nonce: ${nonce.toString()}`);
            console.log(`[HASH]: ${hash}`);
            return {
                challenge: challengeHex,
                nonce: nonce.toString(),
                hash: hash
            };
        }
        nonce++;
    }
}

// Example execution for deterministic test verification
const challenge = "0x0a14dfe14f70157c05a30605c5586d58b2fd87d936e8bd3aecb3eaf95940d553";
const result = find16BitWorkProof(challenge);
console.log(JSON.stringify(result, null, 2));
```

```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/**
 * @title LeadingZeroVerifier
 * @notice Verifies 16-bit leading zero Keccak256 PoW proofs for deterministic bounty validation.
 */
contract LeadingZeroVerifier {
    uint16 public constant REQUIRED_ZERO_BITS = 16;

    /**
     * @notice Validates that keccak256(abi.encodePacked(challenge, nonce)) has at least 16 leading zero bits.
     * @param challenge The target task/bounty challenge hash
     * @param nonce The mined nonce value
     */
    function verifyWorkProof(bytes32 challenge, uint256 nonce) public pure returns (bool, bytes32) {
        bytes32 workHash = keccak256(abi.encodePacked(challenge, nonce));
        
        // Check top 16 bits (first 2 bytes)
        bool isValid = uint16(bytes2(workHash)) == 0;
        return (isValid, workHash);
    }
}
```

### Testing
1. Run `node proof-finder.js` with the bounty target challenge hash (`0x0a14dfe14f70...`).
2. Verify that `bytes2(keccak256(...))` evaluates to `0x0000`.
3. Deploy or invoke `LeadingZeroVerifier.verifyWorkProof(challenge, nonce)` on testnet or local anvil instance to confirm deterministic validation passes with `true`.

Signed-off-by: Aditya Waghamare <adityawaghamare7620@gmail.com>

---
*Submitted by Aditya Waghamare*
💰 **Payout Address (Base L2 / EVM):** `0xb61dBcdBc3407F71EaCb64D4CBFAcf9FFfe2415C`