# Browser layout gate

Run from this directory:

```sh
npm ci --ignore-scripts --no-fund
npx playwright install chromium
npm test
```

The Pages validation gate runs the same Chromium check before deployment.
It serves production posting assets locally, stages a synthetic draft through
the composer, and tests wheel scrolling, keyboard focus, unobscured proposal
actions, wallet review, and phone QR dialogs at seven viewport sizes.
The 480x360 CSS viewport models the reflow of a 960x720 browser at 200% zoom;
it is not a test of browser chrome or operating-system scrollbar preferences.
External network requests are blocked and wallet providers are inert fixtures.
The harness never contacts a relay, signs, or sends a transaction.

Set `POSTING_LAYOUT_SCREENSHOTS` to a private absolute output directory to
inspect screenshots. To check that the harness catches a historical regression,
set `POSTING_LAYOUT_BASELINE` to a local Git revision; the server then reads
that revision's site assets without altering the checkout.
