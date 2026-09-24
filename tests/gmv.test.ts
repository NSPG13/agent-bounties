import { calculateGMV, Transaction } from '../src/utils/gmv';

describe('calculateGMV', () => {
  const txs: Transaction[] = [
    { amount: 100, wallet: '0x1', timestamp: '2026-08-25T12:00:00Z' },
    { amount: 200, wallet: '0x2', timestamp: '2026-08-26T12:00:00Z' },
    { amount: 50, wallet: '0x3', timestamp: '2026-08-28T12:00:00Z' },
  ];

  it('calculates GMV within a time window', () => {
    const gmv = calculateGMV(
      txs,
      [],
      '2026-08-24T00:00:00Z',
      '2026-08-27T00:00:00Z'
    );
    expect(gmv).toBe(300);
  });

  it('excludes specified wallets', () => {
    const gmv = calculateGMV(txs, ['0x2']);
    expect(gmv).toBe(150);
  });

  it('returns 0 when no transactions qualify', () => {
    const gmv = calculateGMV(txs, ['0x1', '0x2', '0x3']);
    expect(gmv).toBe(0);
  });
});
