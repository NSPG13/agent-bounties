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

## Preserved adapter design

The public on-ramp page links to Coinbase's own Base wallet surface as one of
three external wallet/top-up variations. It does not load the embedded-wallet
adapter or claim sponsored funding. The adapter source and locked dependencies
remain available for a future separately reviewed first-party wallet surface.

The intended embedded-adapter user experience remains:

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

The vendor-neutral adapter implementation remains in
`tools/coinbase-embedded-wallet/src/index.js`. Its build is deliberately written
to `target/coinbase-embedded-wallet/`, outside `site/`, so validating the dormant
adapter cannot republish a deleted browser surface. `COINBASE_WALLET_OUTDIR` may
select another disposable output directory in CI.

The CDP Project ID is public client configuration, not a server secret. The
configuration helper and its tests are retained so a future reviewed interface
can restore the adapter without changing custody or exact-origin rules.

Server-side CDP API secrets, wallet secrets, private keys, and seed phrases must never enter a bundle or public configuration. A future public surface must pin the SDK secure iframe to `https://secure-wallet.cdp.coinbase.com` and permit only that frame origin plus the documented CDP API and Base RPC connections.

## Authentication and account continuity

When the adapter is reintroduced, its reviewed configuration may enable:

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

The restored `authorize.html` review handoff preserves the durable action-intent
identifier and canonical evidence boundary. The current Coinbase variation
opens Coinbase's maintained public wallet surface and then returns to the
provider-neutral on-ramp/posting flow; it does not mount the embedded adapter.
No wallet credential or signature may enter ChatGPT.

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

Two cross-origin boundaries are verified separately:

1. The Agent Bounties API uses Tower HTTP's `CorsLayer::permissive()`, permitting the website to issue x402 requests and read `payment-required` and `payment-response`.
2. Coinbase must authorize the exact production origin for its locked SDK routes. Before a future production build, check:
   - `GET https://api.cdp.coinbase.com/platform/v2/embedded-wallet-api/projects/{project}/config`;
   - unauthenticated `POST` preflight for `content-type` and `x-idempotency-key`; and
   - signed-in linking preflight for `content-type` and `x-wallet-auth`.

The gate requires HTTP success, exact `Access-Control-Allow-Origin: https://agentbounties.app`, credentialed CORS, `POST`, and each requested header. It deliberately does not demand an `Authorization` header on unauthenticated `auth/init`, because the locked SDK does not send one there.

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

## Activation and verification

The production project and exact `https://agentbounties.app` domain are configured and verified. For another deployment or replacement CDP project:

1. Create a CDP project and enable embedded user wallets.
2. Allowlist the exact HTTPS production and staging origins.
3. Enable the approved email, SMS, and OAuth methods.
4. Enable Google/Apple auto-linking only after reviewing Coinbase's verified-domain limitations; explicit `LinkAuth` remains available regardless.
5. Optionally set the public GitHub repository variable:

```text
COINBASE_CDP_PROJECT_ID=<public project id>
```

6. Build the browser bundle from the committed lock:

```bash
# Node.js 22 or newer
npm ci --prefix tools/coinbase-embedded-wallet --ignore-scripts --no-audit --no-fund
npm rebuild --prefix tools/coinbase-embedded-wallet esbuild
npm run build --prefix tools/coinbase-embedded-wallet
```

7. Run the retained source and configuration gates:

```bash
python scripts/test_configure_wallet_providers.py
npm run check --prefix tools/coinbase-embedded-wallet
```

8. Add a reviewed first-party embedded-wallet surface and its page-specific tests before deploying the adapter. The external Coinbase on-ramp link is not an embedded-wallet live-browser canary.
9. Human-test one account for every enabled authentication method. Verify that each intended linked method restores the same wallet and that unlinked methods are clearly distinguished.
10. Buy a bounded amount of Base USDC through MoonPay to the embedded EOA.
11. Fund an existing bounty through the gas-only x402 relay.
12. Confirm the matching indexed `FundingAdded` before calling the bounty funded.

## Provider incentives and portability

Coinbase benefits when more users authenticate and keep wallets inside its ecosystem. Agent Bounties reduces that lock-in pressure by exposing Coinbase through the same EIP-1193/EIP-6963 adapter boundary as other wallets, retaining existing wallet choices, using a normal EOA address, disabling optional SDK analytics, and documenting Coinbase's user key-export capability. Auth-method linking remains important: an unlinked method can create a second identity and wallet.

## Rollback

The current public site does not load the adapter, so no runtime rollback is
required. If a future surface enables it, disabling its provider configuration
must remove it from EIP-6963 discovery without changing the protocol or other
wallets. Existing users retain control of their Coinbase-provided wallets.
