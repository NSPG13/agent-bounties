# Hosted Relayer Guide (Updated for x402 Header Compatibility)

> **Note:** This guide has been updated to reflect the additive compatibility change required for the
> `x402-payment-challenge` and `x402-payment-confirmation` headers. No SDK call changes or migrations are
> necessary.

## Overview

The hosted relayer service exposes a browser‑friendly API that now includes the
`x402-payment-challenge` and `x402-payment-confirmation` response headers. These
headers contain the payment challenge token and the confirmation token
respectively, allowing browser‑based SDK clients to read them directly.

## What Changed?

* The **CORS policy** now lists the two `x402-*` headers in the
  `Access‑Control‑Expose‑Headers` response header.
* No other payment logic (amounts, wallet authorisation, canonical confirmation
  rules, etc.) has been altered.
* The change is additive and fully backward compatible.

## How to Verify

1. Perform a request to the relayer endpoint from a browser or using `curl`:

   ```bash
   curl -i -H "Origin: https://myapp.example" \
        -H "Access-Control-Request-Method: GET" \
        -X OPTIONS https://api.agentbounties.com/relayer
   ```

   You should see `Access-Control-Expose-Headers` containing
   `x402-payment-challenge, x402-payment-confirmation`.

2. After a successful payment flow, inspect the response headers of the final
   confirmation request; the two `x402-*` headers will be present and readable
   by JavaScript running in the browser.

## No Migration Required

Existing SDK integrations continue to work unchanged. The added headers are
simply exposed for those that need them.

--- 

*For any questions or further assistance, please open an issue in the repository
or contact the maintainers.*
