# Wallet capability review — September 22, 2026

MetaMask is not the only capable wallet. Provider capability, our adapter and an
observed canonical paid journey are separate evidence levels.

| Wallet | Verified official capabilities | Current platform evidence / gap |
| --- | --- | --- |
| MetaMask | Mobile and extension dapp connections; Base; user-approved transactions | Detected provider plus existing WalletConnect phone route. User reports successful desktop funding; no new paid phone run observed. |
| Coinbase Wallet / Base app | Buy native Base USDC inside the app; open dapps and approve transactions in the app browser; typed-data signatures through Base Account | Generic injected provider and contract-account posting branch exist. Full phone run is unverified. Base app, Base Account and CDP email-created wallet are distinct; never substitute addresses. Cross-device Base app QR connection is discontinued. |
| Trust Wallet | Native Base USDC support; USDC purchases through partners; WalletConnect v2 for EVM networks, QR/deep links, signatures and contract transactions | Phone route already lists Trust Wallet; generic EIP-6963 discovery also exists. Direct Base-USDC purchase availability for this person's region/provider needs a real quote. No paid phone run observed. |
| Rainbow | Base in app and extension; mobile WalletConnect | Potential compatible signing wallet; this review did not verify the direct fiat-to-native-Base-USDC purchase path or an AgentBounties paid run. |
| MoonPay wallet | Buy, receive and send Base assets in its mobile app | No verified dapp signing/connection route. A MoonPay purchase provider is not proof its account wallet can approve our bounty contract. |

## Sources read

- [MetaMask connection and supported platforms](https://docs.metamask.io/metamask-connect/)
- [Buy native Base USDC in Coinbase Wallet](https://help.coinbase.com/en/wallet/managing-account/usdc-coinbase-wallet)
- [Current Base app connection routes](https://help.coinbase.com/en/wallet/other-topics/mobile-app-sign-in-discontinued)
- [Base Account typed-data support](https://docs.base.org/sdks/base-account/reference/core/provider-rpc-methods/eth_signTypedData_v4)
- [Base app Simple mode excludes dapp transactions](https://help.coinbase.com/en/wallet/managing-account/coinbase-wallet-simple-mode)
- [Trust Wallet mobile protocol support](https://developer.trustwallet.com/developer/develop-for-trust/mobile)
- [Trust Wallet EVM methods](https://developer.trustwallet.com/developer/develop-for-trust/browser-extension/evm)
- [Trust Wallet USDC buying](https://trustwallet.com/buy-crypto/usdc)
- [Trust Wallet native Base USDC support](https://trustwallet.com/blog/campaigns/swap-selected-stablecoins-with-0-fees-on-trust-wallet)
- [Rainbow Base support](https://learn.rainbow.me/learn-more-about-base)
- [Rainbow mobile WalletConnect](https://rainbow.me/support/extension/supported-browsers-and-systems)
- [MoonPay wallet abilities](https://support.moonpay.com/en/articles/383215-managing-your-wallets)

## Local checks

`site/phone-wallet.js` lists Trust Wallet and MetaMask universal links and requests
Base plus `eth_sendTransaction`, `personal_sign` and `eth_signTypedData_v4`.
`site/bounty-composer-v2.js` uses bounded typed-data authorization for ordinary
accounts, and contract-account calls otherwise; lack of batch support falls back
only on explicit unsupported-method errors. Lost replies keep recovery state.
Contract source supports ERC-1271 verification. The public account-wallet linking
API still uses ECDSA personal-sign recovery, so smart-wallet ownership linking
needs separate validation; generic provider connection is not evidence it passed.

No wallet was imported, connected anew, signed, funded or charged by this review.
A complete provider acceptance run must confirm same-address receipt on Base,
connection, exact transaction approval, canonical creation/funding and public
claimability. Purchase quotes and fees depend on region, provider and current
network conditions. An app may open its purchase partner while keeping the same
wallet; this does not require a second wallet or an extra transfer.
