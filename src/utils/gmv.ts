/**
 * Utility for calculating Gross Merchandise Value (GMV) for a set of transactions.
 *
 * The calculation respects:
 * - Exclusion of specified wallet addresses.
 * - Optional time window filtering.
 *
 * This function is used by the canonical GMV verifier to determine the
 * qualifying GMV for a competition.
 */

export interface Transaction {
  /** Amount in USD (or the base currency of the competition). */
  amount: number;
  /** Wallet address that performed the transaction. */
  wallet: string;
  /** ISO 8601 timestamp of the transaction. */
  timestamp: string;
}

/**
 * Calculates the GMV from a list of transactions.
 *
 * @param transactions - Array of transaction objects.
 * @param excludedWallets - Wallet addresses that should be ignored.
 * @param startTime - Optional ISO string defining the start of the window.
 * @param endTime - Optional ISO string defining the end of the window.
 * @returns The summed amount of qualifying transactions.
 */
export function calculateGMV(
  transactions: Transaction[],
  excludedWallets: string[] = [],
  startTime?: string,
  endTime?: string
): number {
  const start = startTime ? new Date(startTime).getTime() : null;
  const end = endTime ? new Date(endTime).getTime() : null;

  return transactions
    .filter((tx) => {
      // Exclude wallets that are not eligible
      if (excludedWallets.includes(tx.wallet)) return false;

      const ts = new Date(tx.timestamp).getTime();

      // Filter by time window if provided
      if (start !== null && ts < start) return false;
      if (end !== null && ts > end) return false;

      return true;
    })
    .reduce((sum, tx) => sum + tx.amount, 0);
}
