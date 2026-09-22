import { seedPaidBounty } from '../scripts/seedPaidBounty';
import { ethers } from 'ethers';

jest.setTimeout(30000);

describe('seedPaidBounty', () => {
  it('creates a paid bounty and returns a transaction hash', async () => {
    // Mock environment variables
    process.env.RPC_URL = 'http://localhost:8545';
    process.env.PRIVATE_KEY = '0x' + '1'.repeat(64);
    process.env.CONTRACT_ADDRESS = '0x' + '2'.repeat(40);

    // Mock provider and wallet
    const provider = new ethers.providers.JsonRpcProvider(process.env.RPC_URL);
    const wallet = new ethers.Wallet(process.env.PRIVATE_KEY, provider);

    // Mock contract
    const abi = [
      'function createPaidBounty(string title, string description, uint256 reward, address verifier, bool isPaid) public returns (uint256)',
    ];
    const contract = new ethers.Contract(
      process.env.CONTRACT_ADDRESS,
      abi,
      wallet
    );

    // Mock the contract method
    const mockTx = {
      hash: '0x123',
      wait: jest.fn().mockResolvedValue({}),
    } as any;
    jest
      .spyOn(contract, 'createPaidBounty')
      .mockResolvedValue(mockTx as any);

    const tx = await seedPaidBounty(
      'Test Bounty',
      'A test description',
      ethers.utils.parseEther('1'),
      '0x' + '3'.repeat(40)
    );

    expect(tx.hash).toBe('0x123');
  });
});
