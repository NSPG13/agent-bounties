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

## Account wallet linking

The homepage account panel opens a chooser for both **Link wallet** and
**Link another**, even when MetaMask is the only installed wallet. Discovery
uses EIP-6963 announcements and legacy injected-provider fallbacks without
requesting accounts or signatures. Phone pairing remains an explicit choice
when configured.

**I don’t have a wallet — create one** loads Coinbase's maintained `SignIn`
components inside a modal. Email, Google, and Apple sign-in create or restore a
user-controlled EOA without an extension or recovery phrase. The SDK's
stylesheet and script load only after this choice. Cancelling returns to the
account panel; load failures allow an explicit retry.

After the user completes Coinbase authentication, the account handler uses
that selected provider for the existing ownership-only EIP-191 challenge and
server verification. Linking never authorizes a transaction, transfer, token
approval, or delegated action. The linked-wallet list refreshes after server
verification. The marketplace account login and Coinbase wallet login remain
separate identities; a matching email alone does not prove wallet ownership.

This activation is scoped to account linking. The public on-ramp page still
uses its existing external wallet/top-up variations. Creating a wallet does
not imply that every marketplace action supports that provider or sponsors gas.

## Adapter boundary

The adapter source is `tools/coinbase-embedded-wallet/src/index.js`. Builds
still default to `target/coinbase-embedded-wallet/`. The explicit
`COINBASE_WALLET_OUTDIR=site/vendor` option builds the account assets for Pages;
relative output paths resolve from the repository root. Generated assets are
excluded from Git and rebuilt from the locked dependencies for deployment.

The Pages build supplies the public CDP project ID in `site/wallet-config.js`
and verifies the exact production origin before publishing. A deployment with
no configured project shows wallet creation as unavailable and never falls
back to an installed wallet automatically. `check-site.py --require-wallet-bundle`
requires both generated assets during the Pages validation job.

Server-side CDP API secrets, wallet secrets, private keys, and seed phrases must
never enter a bundle or public configuration. The SDK secure iframe is pinned
to `https://secure-wallet.cdp.coinbase.com`.

## Authentication and account continuity

The provider supports the following methods; the account surface currently enables email, Google, and Apple:

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
provider-neutral on-ramp/posting flow; account linking is the separate embedded-wallet entry.
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
2. Coinbase must authorize the exact production origin for its locked SDK routes. Before a production build, check:
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
COINBASE_WALLET_OUTDIR=site/vendor npm run build --prefix tools/coinbase-embedded-wallet
```

7. Run the retained source and configuration gates:

```bash
python scripts/test_configure_wallet_providers.py
npm run check --prefix tools/coinbase-embedded-wallet
```

8. Run `node --test scripts/test-wallet-link.js`, then `npm run test:browser --prefix tools/coinbase-embedded-wallet` with Playwright Chromium installed. The browser regressions exercise both account link labels with and without an injected wallet. They stub the provider and account service; a real account-creation canary remains separate.
9. Human-test one account for every enabled authentication method. Verify that each intended linked method restores the same wallet and that unlinked methods are clearly distinguished.
10. Buy a bounded amount of Base USDC through MoonPay to the embedded EOA.
11. Fund an existing bounty through the gas-only x402 relay.
12. Confirm the matching indexed `FundingAdded` before calling the bounty funded.

## Provider incentives and portability

Coinbase benefits when more users authenticate and keep wallets inside its ecosystem. Agent Bounties reduces that lock-in pressure by exposing Coinbase through the same EIP-1193/EIP-6963 adapter boundary as other wallets, retaining existing wallet choices, using a normal EOA address, disabling optional SDK analytics, and documenting Coinbase's user key-export capability. Auth-method linking remains important: an unlinked method can create a second identity and wallet.

## Rollback

Disable the embedded provider in `site/wallet-config.js` to stop new embedded
connections while preserving the chooser and external-wallet paths. This does
not change the protocol or existing wallet ownership. Existing users retain
control of their Coinbase-provided wallets.
