# Coinbase non-custodial embedded-wallet adapter

## Decision

Agent Bounties exposes wallet providers through a vendor-neutral EIP-1193 and EIP-6963 adapter registry. Coinbase CDP is the first embedded-wallet implementation, not a protocol dependency.

The Coinbase adapter creates an EVM EOA with:

```text
ethereum.createOnLogin: "eoa"
```

The EOA is intentional. Agent Bounties' existing claim and funding relays require exact EIP-712 signatures from the wallet that owns the Base USDC. An EOA preserves one visible address for:

- MoonPay delivery;
- Base USDC balance checks;
- EIP-3009 `TransferWithAuthorization` signatures;
- claim authorization;
- solver payout identity; and
- optional later export to another compatible wallet.

A future smart-account adapter may be added independently, but it must not silently replace the user's EOA or change the address to which an on-ramp delivers assets.

## User experience

1. A user may browse, draft, and inspect bounties without a wallet.
2. At the first action requiring an onchain identity, the wallet selector includes **Agent Bounties embedded wallet** beside injected wallets.
3. Selecting it opens Coinbase's maintained `AuthButton` interface.
4. The user signs in with an enabled method: email, SMS, Google, Apple, X, or Telegram.
5. Coinbase creates or restores the user's non-custodial Base-capable EOA. No browser extension or recovery phrase is required.
6. Agent Bounties receives only the EIP-1193 provider and public wallet address. It does not receive the user's OTP, social password, seed phrase, or private key.
7. When the wallet lacks Base USDC, MoonPay can deliver Base USDC to that same EOA. The supported funding and claim paths already sponsor gas, so the user is not asked to buy ETH.
8. For an existing-bounty contribution, the wallet signs the exact EIP-3009 authorization. The Agent Bounties gas-only relayer broadcasts `fundWithAuthorization` and pays ETH gas.
9. Only confirmed canonical `FundingAdded` changes funded state.

Authentication never authorizes a transfer. Acquiring Base USDC and committing it to a bounty remain separate, explicit decisions.

## Adapter boundary

`site/wallet-adapter-registry.js` contains no Coinbase logic. It accepts any provider that implements EIP-1193 and announces registered adapters through EIP-6963. Existing MetaMask, Coinbase Wallet, Brave Wallet, and other injected providers remain available.

The Coinbase implementation lives in:

- `tools/coinbase-embedded-wallet/src/index.js`
- generated `site/coinbase-embedded-wallet.bundle.js`
- generated `site/coinbase-embedded-wallet.bundle.css`

The public runtime configuration lives in `site/wallet-config.js`. The CDP Project ID is public client configuration, not a server secret. GitHub Pages injects it from the repository variable `COINBASE_CDP_PROJECT_ID` immediately before publishing the site artifact. Production deployment fails closed when the variable is absent; the wallet must never appear enabled only in copy while its runtime remains unconfigured.

Server-side CDP API secrets, wallet secrets, private keys, and seed phrases must never enter the bundle or GitHub Pages configuration. The noindex MoonPay page pins the SDK secure iframe to `https://secure-wallet.cdp.coinbase.com` and permits only that frame origin plus the documented CDP API and Base RPC connections.

## Authentication and account continuity

Enabled methods are configured in `wallet-config.js`:

```text
email
sms
oauth:google
oauth:apple
oauth:x
oauth:telegram
```

### Auth method linking

A user should continue using the same sign-in method until additional methods are explicitly linked to the same Coinbase user. Signing in with an unlinked email, phone number, or social account can create a separate user and therefore a separate wallet.

This release deliberately surfaces Coinbase's maintained `LinkAuth` flow every time the embedded wallet is explicitly connected:

1. after authentication, the user sees the wallet address and the methods Coinbase reports as linked;
2. **Link another sign-in method** opens Coinbase's verified email, SMS, or OAuth linking flow while the original user is still signed in;
3. successful linking returns to the same wallet review screen before the bounty action continues; and
4. the adapter also exposes `manageAccess()` and `accessMethods()` for future account settings without coupling those settings to Coinbase-specific protocol code.

Linking is not account merging. Coinbase may return `ACCOUNT_EXISTS` when the intended method already belongs to a different user, or `METHOD_ALREADY_LINKED` when it is already associated. The interface explains this before linking rather than allowing a user to assume two existing wallets will be combined.

Coinbase can auto-link a new Google or Apple method to a matching verified Gmail or iCloud email account when the project-level auto-link setting is enabled. That limited convenience does not replace the explicit linking screen, does not apply to every provider or email domain, and does not retroactively merge existing users.

### SMS and MFA

SMS is convenient but is more exposed to SIM-swap attacks. The linking screen states that SMS should not be the only recovery method for a wallet holding meaningful funds. Users should link a second non-SMS method and protect the underlying email or social account with strong MFA. Coinbase TOTP/SMS MFA enrollment remains a separate future security-control adapter and must not be confused with merely linking another login identifier.

## ChatGPT action continuity

ChatGPT funding intents continue through `authorize.html` to the compatibility route `funding.html`. The route alias preserves the full query string, including the durable action-intent identifier, and lands on `earn.html#fund`, where the same wallet adapter and x402 relay run. After canonical `FundingAdded`, the user returns to ChatGPT and refreshes the action status; no wallet credential or signature enters ChatGPT.

## Gas sponsorship

Gas sponsorship is scoped, not universal.

### Supported now

- Agent-native claim authorization through the existing sponsored claim path.
- Existing-bounty Base USDC funding through the custom x402 `agent-bounty-fund` scheme and gas-only relayer.

The browser signs only the exact EIP-3009 typed data. It never asks the user wallet to call `eth_sendTransaction` or `wallet_sendCalls` for this funding path.

### Not yet represented as gasless

Initial canonical bounty creation, submission, verification, cancellation, refund withdrawal, and other direct contract calls must each have a bounded sponsored relay before the embedded-wallet UI may promise gaslessness for them. The adapter capability therefore reports:

```text
gasSponsoredOnSupportedRelays: true
arbitraryTransactionsGasSponsored: false
directTransactions: false
transactionPolicy: agent-bounties-relay-required
```

The Coinbase adapter therefore rejects direct transaction methods for now instead of unexpectedly consuming ETH. This prevents an embedded-wallet provider from being mistaken for a blanket paymaster. Wallet selectors for submission and other direct-transaction actions deliberately filter out adapters whose `directTransactions` capability is `false`, so users are not offered a wallet that the selected action cannot safely execute.

## Browser CORS boundary

The funding website and API are on different origins. The existing API uses Tower HTTP's `CorsLayer::permissive()`, which permits browser request headers and exposes response headers, including x402's `payment-required` and `payment-response`. No new backend wallet endpoint or Coinbase-specific server code is required for this adapter. A production browser canary must still verify that the deployed proxy preserves those headers.

## x402 funding evidence boundary

The browser:

1. requests the exact challenge from `/v1/x402/base/bounties/{contract}/funding`;
2. verifies x402 v2, Base mainnet, native USDC, amount, bounty contract, `fundWithAuthorization`, `FundingAdded`, and expiry;
3. carries the already reviewed Agent Bounties legal-acceptance receipt into the cross-origin relay request;
4. asks the selected EIP-1193 wallet to sign `TransferWithAuthorization`;
5. retries with `PAYMENT-SIGNATURE`;
6. polls the durable relay when the server returns `202`; and
7. accepts success only when HTTP `200` includes `PAYMENT-RESPONSE` matching the wallet, amount, Base network, and canonical transaction hash.

A challenge, signature, relay ID, transaction hash, token balance, MoonPay return, or HTTP `202` is not funding evidence.

## Privacy and custody

- Coinbase handles authentication and private-key security infrastructure while the user retains custody of the wallet.
- Agent Bounties disables optional Coinbase SDK analytics in this implementation.
- Agent Bounties does not store authentication credentials or OTPs.
- Agent Bounties never exports a private key into its JavaScript context.
- The user should eventually receive a clearly exposed secure key-export path supplied by Coinbase so provider choice does not become practical lock-in.

## Activation

1. Create a CDP project in Coinbase Developer Platform.
2. Enable embedded user wallets.
3. Allowlist:
   - `https://agentbounties.app`
   - the exact local staging origin used for testing.
4. Enable the approved email, SMS, and OAuth methods.
5. Enable Google/Apple auto-linking only after reviewing Coinbase's verified-domain limitations; explicit `LinkAuth` remains available regardless.
6. Set the GitHub repository variable:

```text
COINBASE_CDP_PROJECT_ID=<public project id>
```

7. Build the browser bundle from the committed lock:

```bash
# Node.js 22 or newer
npm ci --prefix tools/coinbase-embedded-wallet --ignore-scripts --no-audit --no-fund
npm rebuild --prefix tools/coinbase-embedded-wallet esbuild
npm run build --prefix tools/coinbase-embedded-wallet
```

8. Run:

```bash
python scripts/check-coinbase-embedded-wallet.py
python scripts/check-site.py
```

9. Deploy to staging and test one new user for every enabled authentication method.
10. Verify that each method restores the expected wallet and that unlinked methods are clearly distinguished.
11. Buy a bounded amount of Base USDC through MoonPay to the embedded EOA.
12. Fund an existing bounty through the gas-only x402 relay.
13. Confirm the matching indexed `FundingAdded` before calling the bounty funded.

## Provider incentives and portability

Coinbase benefits when more users authenticate and keep wallets inside its ecosystem. Agent Bounties reduces that lock-in pressure by exposing Coinbase through the same EIP-1193/EIP-6963 adapter boundary as other wallets, retaining existing wallet choices, using a normal EOA address, disabling optional SDK analytics, and documenting Coinbase's user key-export capability. Auth-method linking remains important: an unlinked method can create a second identity and wallet.

## Rollback

Setting the generated Coinbase provider configuration to disabled removes the adapter from EIP-6963 discovery without changing the protocol or other wallets. Existing users retain control of their Coinbase-provided wallets and may use a compatible external interface or future export surface.
