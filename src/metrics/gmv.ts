/**
 * Utility functions for calculating Gross Merchandise Value (GMV) for
 * the Agent Bounties platform.
 *
 * The GMV calculation is used by the `forward-canonical-gmv-attribution-metric-v2`
 * verifier to determine the highest externally funded canonical GMV for a
 * given time window.  The implementation below is intentionally
 * lightweight and deterministic so that it can be used both in the
 * backend API and in unit tests.
 */

export interface Transaction {
  /** The amount of the transaction in the native token (e.g. USDC). */
  amount: number;
  /** The wallet address that performed the transaction. */
  wallet: string;
  /** Whether the transaction was performed by a contract. */
  contract: boolean;
  /** ISO 8601 timestamp of the transaction. */
  timestamp: string;
}

/**
 * Calculates the GMV for a list of transactions.
 *
 * @param transactions - Array of transactions to consider.
 * @param excludedWallets - Optional list of wallet addresses that should be
 *                          excluded from the GMV calculation (e.g. known
 *                          excluded wallets or contracts).
 * @returns The total GMV as a number.
 */
export function calculateGMV(
  transactions: Transaction[],
  excludedWallets: string[] = []
): number {
  return transactions
    .filter(
      (tx) =>
        !excludedWallets.includes(tx.wallet) && !tx.contract
    )
    .reduce((sum, tx) => sum + tx.amount, 0);
}

/**
 * Generates the qualifying GMV for a specific week.
 *
 * @param transactions - Array of all transactions in the system.
 * @param weekStart - ISO 8601 string or Date representing the start of the week.
 * @param weekEnd - ISO 8601 string or Date representing the end of the week.
 * @param excludedWallets - Optional list of wallet addresses to exclude.
 * @returns The GMV for the specified week.
 */
export function generateQualifyingGMV(
  transactions: Transaction[],
  weekStart: Date | string,
  weekEnd: Date | string,
  excludedWallets: string[] = []
): number {
  const start = new Date(weekStart);
  const end = new Date(weekEnd);

  const weekTxs = transactions.filter((tx) => {
    const txDate = new Date(tx.timestamp);
    return txDate >= start && txDate < end;
  });

  return calculateGMV(weekTxs, excludedWallets);
}
