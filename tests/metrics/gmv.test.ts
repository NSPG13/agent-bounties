import { calculateGMV, generateQualifyingGMV, Transaction } from '../../src/metrics/gmv';

describe('GMV utilities', () => {
  const sampleTxs: Transaction[] = [
    {
      amount: 100,
      wallet: '0xAAA',
      contract: false,
      timestamp: '2026-08-31T10:00:00Z',
    },
    {
      amount: 200,
      wallet: '0xBBB',
      contract: false,
      timestamp: '2026-08-31T12:00:00Z',
    },
    {
      amount: 50,
      wallet: '0xCCC',
      contract: true,
      timestamp: '2026-08-31T14:00:00Z',
    },
    {
      amount: 75,
      wallet: '0xDDD',
      contract: false,
      timestamp: '2026-09-01T09:00:00Z',
    },
  ];

  it('calculates GMV excluding contracts and specified wallets', () => {
    const excluded = ['0xBBB'];
    const gmv = calculateGMV(sampleTxs, excluded);
    // 100 (0xAAA) + 75 (0xDDD) = 175
    expect(gmv).toBe(175);
  });

  it('generates qualifying GMV for a specific week', () => {
    const weekStart = '2026-08-31T00:00:00Z';
    const weekEnd = '2026-09-07T00:00:00Z';
    const gmv = generateQualifyingGMV(sampleTxs, weekStart, weekEnd);
    // 100 + 200 + 75 = 375 (contract tx is excluded)
    expect(gmv).toBe(375);
  });

  it('returns 0 when no transactions fall within the week', () => {
    const weekStart = '2026-07-01T00:00:00Z';
    const weekEnd = '2026-07-08T00:00:00Z';
    const gmv = generateQualifyingGMV(sampleTxs, weekStart, weekEnd);
    expect(gmv).toBe(0);
  });
});
