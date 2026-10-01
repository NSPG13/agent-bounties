import algosdk from 'algosdk';

export const FACILITATOR = 'https://facilitator.goplausible.xyz';
export const TAG = 'x402-global-challenge';
export const NETWORKS = Object.freeze({
  // Use the identifiers advertised by GoPlausible /supported. The SDK accepts
  // these legacy full-genesis identifiers as well as the newer short CAIP form.
  mainnet: { network: 'algorand:wGHE2Pwdvd7S12BL5FaOP20EGYesN73ktiC1qzkkit8=', asset: '31566704', algod: 'https://mainnet-api.algonode.cloud' },
  testnet: { network: 'algorand:SGO1GKSzyE7IEPItTxCByw9x8FmnrCDexi9/cOUJOiI=', asset: '10458941', algod: 'https://testnet-api.algonode.cloud' },
});
export const FEED_URL = 'https://api.agentbounties.app/v1/base/autonomous-bounties/feed?network=base-mainnet&claimable_only=true';

export function readConfig(env = process.env) {
  const chain = env.ALGORAND_NETWORK || 'mainnet';
  if (!NETWORKS[chain]) throw new Error('ALGORAND_NETWORK must be mainnet or testnet');
  const payTo = env.ALGORAND_PAY_TO;
  if (!algosdk.isValidAddress(payTo || '')) throw new Error('A valid ALGORAND_PAY_TO public address is required');
  const origin = new URL(env.PUBLIC_ORIGIN || env.RENDER_EXTERNAL_URL || 'http://localhost:4021');
  if (origin.protocol !== 'https:' && !['localhost', '127.0.0.1'].includes(origin.hostname)) throw new Error('Public origin must use HTTPS');
  if (origin.username || origin.password || origin.pathname !== '/' || origin.search || origin.hash) throw new Error('PUBLIC_ORIGIN must be a bare origin');
  return { ...NETWORKS[chain], chain, payTo, origin: origin.origin, amount: '10000', facilitator: FACILITATOR, tag: TAG };
}
