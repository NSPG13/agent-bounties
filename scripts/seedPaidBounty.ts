import { ethers } from 'ethers';
import dotenv from 'dotenv';

dotenv.config();

const RPC_URL = process.env.RPC_URL;
const PRIVATE_KEY = process.env.PRIVATE_KEY;
const CONTRACT_ADDRESS = process.env.CONTRACT_ADDRESS;

if (!RPC_URL || !PRIVATE_KEY || !CONTRACT_ADDRESS) {
  console.error(
    'Missing environment variables. Please set RPC_URL, PRIVATE_KEY, and CONTRACT_ADDRESS.'
  );
  process.exit(1);
}

const provider = new ethers.providers.JsonRpcProvider(RPC_URL);
const wallet = new ethers.Wallet(PRIVATE_KEY, provider);

const ABI = [
  // Minimal ABI for creating a paid bounty
  'function createPaidBounty(string title, string description, uint256 reward, address verifier, bool isPaid) public returns (uint256)',
];

const contract = new ethers.Contract(CONTRACT_ADDRESS, ABI, wallet);

/**
 * Seed a paid CLI child bounty on the Agent Bounties contract.
 *
 * @param title - The title of the bounty.
 * @param description - A short description of the bounty.
 * @param reward - The reward amount in wei (or a string that can be parsed by ethers).
 * @param verifier - The address of the verifier contract.
 * @param isPaid - Whether the bounty is paid (default true).
 * @returns The transaction response for the bounty creation.
 */
export async function seedPaidBounty(
  title: string,
  description: string,
  reward: ethers.BigNumberish,
  verifier: string,
  isPaid = true
): Promise<ethers.providers.TransactionResponse> {
  const tx = await contract.createPaidBounty(
    title,
    description,
    reward,
    verifier,
    isPaid
  );
  console.log(`Transaction sent: ${tx.hash}`);
  await tx.wait();
  console.log('Bounty created successfully.');
  return tx;
}

if (require.main === module) {
  const [,, title, description, reward, verifier] = process.argv;
  if (!title || !description || !reward || !verifier) {
    console.error(
      'Usage: ts-node scripts/seedPaidBounty.ts <title> <description> <reward> <verifier>'
    );
    process.exit(1);
  }

  seedPaidBounty(
    title,
    description,
    ethers.utils.parseEther(reward),
    verifier
  ).catch((err) => {
    console.error(err);
    process.exit(1);
  });
}
