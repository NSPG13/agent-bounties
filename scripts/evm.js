(() => {
  "use strict";

  const KECCAK_RATE_BYTES = 136;
  const KECCAK_ROTATIONS = Object.freeze([
    0, 1, 62, 28, 27,
    36, 44, 6, 55, 20,
    3, 10, 43, 25, 39,
    41, 45, 15, 21, 8,
    18, 2, 61, 56, 14,
  ]);
  const KECCAK_ROUND_CONSTANTS = Object.freeze([
    0x0000000000000001n, 0x0000000000008082n, 0x800000000000808an,
    0x8000000080008000n, 0x000000000000808bn, 0x0000000080000001n,
    0x8000000080008081n, 0x8000000000008009n, 0x000000000000008an,
    0x0000000000000088n, 0x0000000080008009n, 0x000000008000000an,
    0x000000008000808bn, 0x800000000000008bn, 0x8000000000008089n,
    0x8000000000008003n, 0x8000000000008002n, 0x8000000000000080n,
    0x000000000000800an, 0x800000008000000an, 0x8000000080008081n,
    0x8000000000008080n, 0x0000000080000001n, 0x8000000080008008n,
  ]);

  function rotateLeft64(value, bits) {
    if (bits === 0) return value;
    const shift = BigInt(bits);
    return ((value << shift) | (value >> (64n - shift))) & ((1n << 64n) - 1n);
  }

  function keccakPermutation(words) {
    for (const roundConstant of KECCAK_ROUND_CONSTANTS) {
      const column = Array(5).fill(0n);
      for (let x = 0; x < 5; x += 1) {
        for (let y = 0; y < 5; y += 1) column[x] ^= words[x + (5 * y)];
      }
      const delta = column.map((_, x) => (
        column[(x + 4) % 5] ^ rotateLeft64(column[(x + 1) % 5], 1)
      ));
      for (let x = 0; x < 5; x += 1) {
        for (let y = 0; y < 5; y += 1) words[x + (5 * y)] ^= delta[x];
      }
      const rotated = Array(25).fill(0n);
      for (let x = 0; x < 5; x += 1) {
        for (let y = 0; y < 5; y += 1) {
          rotated[y + (5 * ((2 * x + 3 * y) % 5))] = rotateLeft64(
            words[x + (5 * y)], KECCAK_ROTATIONS[x + (5 * y)],
          );
        }
      }
      for (let x = 0; x < 5; x += 1) {
        for (let y = 0; y < 5; y += 1) {
          words[x + (5 * y)] = rotated[x + (5 * y)]
            ^ ((~rotated[((x + 1) % 5) + (5 * y)])
              & rotated[((x + 2) % 5) + (5 * y)]);
        }
      }
      words[0] ^= roundConstant;
    }
  }

  function absorbKeccakBlock(words, bytes) {
    for (let index = 0; index < KECCAK_RATE_BYTES; index += 1) {
      words[Math.floor(index / 8)] ^= BigInt(bytes[index]) << BigInt((index % 8) * 8);
    }
    keccakPermutation(words);
  }

  function keccak256Hex(value) {
    const input = String(value || "");
    if (!/^0x(?:[0-9a-fA-F]{2})*$/.test(input)) throw new Error("Keccak input must be hex bytes.");
    const bytes = new Uint8Array((input.length - 2) / 2);
    for (let index = 0; index < bytes.length; index += 1) {
      bytes[index] = Number.parseInt(input.slice(2 + (index * 2), 4 + (index * 2)), 16);
    }
    const words = Array(25).fill(0n);
    let offset = 0;
    while (offset + KECCAK_RATE_BYTES <= bytes.length) {
      absorbKeccakBlock(words, bytes.subarray(offset, offset + KECCAK_RATE_BYTES));
      offset += KECCAK_RATE_BYTES;
    }
    const finalBlock = new Uint8Array(KECCAK_RATE_BYTES);
    finalBlock.set(bytes.subarray(offset));
    finalBlock[bytes.length - offset] ^= 0x01;
    finalBlock[KECCAK_RATE_BYTES - 1] ^= 0x80;
    absorbKeccakBlock(words, finalBlock);
    let digest = "";
    for (let index = 0; index < 32; index += 1) {
      const byte = Number((words[Math.floor(index / 8)] >> BigInt((index % 8) * 8)) & 0xffn);
      digest += byte.toString(16).padStart(2, "0");
    }
    return `0x${digest}`;
  }

  function textHex(value) {
    return `0x${Array.from(new TextEncoder().encode(value), (byte) => byte.toString(16).padStart(2, "0")).join("")}`;
  }

  function randomBytes32() {
    const bytes = new Uint8Array(32);
    crypto.getRandomValues(bytes);
    return `0x${Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("")}`;
  }

  function bytes32Word(value, label = "bytes32") {
    const normalized = String(value || "").toLowerCase();
    if (!/^0x[0-9a-f]{64}$/.test(normalized) || /^0x0{64}$/.test(normalized)) {
      throw new Error(`${label} must be a nonzero 32-byte hex value.`);
    }
    return normalized.slice(2);
  }

  function addressWord(value) {
    const normalized = String(value || "").toLowerCase();
    if (!/^0x[0-9a-f]{40}$/.test(normalized)) throw new Error("Expected an EVM address.");
    return `${"0".repeat(24)}${normalized.slice(2)}`;
  }

  function uint256Word(value) {
    const parsed = BigInt(value);
    if (parsed < 0n || parsed >= (1n << 256n)) throw new Error("uint256 value is out of bounds.");
    return parsed.toString(16).padStart(64, "0");
  }

  function transferWithAuthorizationTypes() {
    return {
      EIP712Domain: [
        { name: "name", type: "string" },
        { name: "version", type: "string" },
        { name: "chainId", type: "uint256" },
        { name: "verifyingContract", type: "address" },
      ],
      TransferWithAuthorization: [
        { name: "from", type: "address" },
        { name: "to", type: "address" },
        { name: "value", type: "uint256" },
        { name: "validAfter", type: "uint256" },
        { name: "validBefore", type: "uint256" },
        { name: "nonce", type: "bytes32" },
      ],
    };
  }

  window.AgentBountiesEvm = Object.freeze({
    addressWord,
    bytes32Word,
    keccak256Hex,
    randomBytes32,
    textHex,
    transferWithAuthorizationTypes,
    uint256Word,
  });
})();
