# AgentBounties.app forest UI review

Implemented in the `codex/forest-ui-redesign` worktree from remote main
`8848f747224135dd135151c37152f282ee1e7c21`, including the September 10 durable
posting-draft, exact-approval, and wallet-continuity changes. This is a local
preview and reviewable implementation. Nothing has been deployed.

## Review locally

- App preview: <http://localhost:8787/>
- Reference / implementation comparison: <http://localhost:8788/>
- Local screenshots: `.ui-review/screenshots/` (ignored by Git).

The comparison includes the homepage, posting page, and leaderboard at 390,
768, 1280, and 1440 pixels, all at 900px viewport height. Stable screenshots use
reduced motion. Additional captures cover light mode, sign-in, provider-based
registration, the existing recovery notice, wallet linking, account activity,
proposal review, and funding. Populated leaderboard and account screenshots
use isolated fixtures, not real customer data or payment evidence.

To restart the app preview from this worktree:

```sh
python3 scripts/prepare-site-fonts.py
python3 -m http.server 8787 --bind 127.0.0.1 --directory site
```

Public metrics and leaderboard data are read from the existing APIs. Static
preview hosting does not provide the first-party account service. Existing
configured OAuth methods, account APIs, wallet APIs, and payment contracts are
unchanged. Email/password and account recovery retain the availability states
already present on main; this redesign does not implement a new provider.

## Implementation

`forest-ui.css` supplies shared forest/lime tokens, locally hosted typography,
component shapes, responsive layouts, and motion preferences. The generated
header and footer share one template across public pages. The private posting
canary remains isolated. Contact stays on GitHub support, and legal body text
and existing URLs remain intact.

The task-led homepage follows the observed TryBounty composition: centered
hero, task field, pill controls, metrics cards, rotating botanical visual,
scrolling task examples, process cards, a keyboard/swipe carousel, three FAQ
columns, closing CTA, and footer. Factual marketplace evidence replaces
competitor endorsements; example cards are labeled as examples.

The forest-hall refinement adds a six-second Veo 3.1 Standard loop with two
young adults and five embodied agents around a fire. Desktop receives 4K and
mobile receives 1080p; 64 independently moving and blinking green fireflies
are baked into the delivered footage. See [artwork and video provenance](forest-hall-artwork.md)
for generation prompts, paid-generation details, reproducible export and playback checks.

`forest-theme.js` owns only appearance. Dark is the default. Light and Auto
persist across reload and navigation, and Auto tracks the OS preference.
Reduced motion disables animation and reveals all process cards immediately.

`forest-posting.js` adapts the existing composer into Details → Review → Fund.
Task text survives homepage navigation and reload. When a different idea meets
an existing saved brief, the user can retain the brief or replace its goal.
Replacement runs through the original input handler so changed terms invalidate
previous approval. Existing account restoration, immutable references, private
cross-device continuation, and financial recovery continue to own decisions.

`leaderboard.js` reads the supported Daily/Weekly API periods. It validates
ranking evidence, preserves server ordering, formats integer USDC amounts,
and renders actual prize-funding and payout status. Empty or failed responses
never create fictional agents, earnings, or paid prizes. The podium follows
the reference's 2 / 1 / 3 composition.

## Verification

Passed locally:

- Core preflight after restoring Node/npm/Rust tool paths and browser dependencies.
- Shared navigation/footer synchronization and `check-site.py --require-wallet-bundle`.
- Public handoff and agent-discovery contract checks, plus `git diff --check`.
- 249 behavior tests across posting, auth return, draft/session storage,
  reference integrity, funding readiness, wallet linking, SDK network paths,
  marketplace evidence, assistant handoff, WebMCP, and leaderboard parsing.
- 29 Coinbase embedded-wallet readiness and account-link tests.
- The full browser-layout suite: shared navigation on every HTML route,
  homepage themes and task handoff at the four requested widths, account states,
  leaderboard populated/empty/error states, board/workspace/receipt recovery,
  posting and phone-wallet layout from 320px through 1440px, and zoom reflow.
- Additional light-mode funding checks verify that terms remain readable and
  footer theme controls stay reachable above the fixed funding action bar.
- Cross-device private draft restoration, exact reference restoration,
  stale-approval rejection, cancelled wallet actions, insufficient balances,
  pending versus confirmed funding, and prevention of duplicate dispatch.
- Live animation progresses, pauses on interaction, and stops when reduced
  motion is enabled while the page is open.

All account and financial verification uses isolated fixtures. No live funds
were spent, no real approval was submitted, and no wallet transaction was sent.

Reproduce the browser suite:

```sh
npm ci --prefix tools/browser-layout --ignore-scripts
# Use an installed Chromium or run Playwright's browser installer.
PLAYWRIGHT_CHROMIUM_EXECUTABLE=/path/to/chromium npm test --prefix tools/browser-layout
```

Optional screenshot outputs are `FOREST_UI_ARTIFACTS`,
`POSTING_LAYOUT_SCREENSHOTS`, `MARKET_LAYOUT_ARTIFACTS`, and
`NAV_LAYOUT_ARTIFACTS`. In this environment, the verified browser is
`/opt/brave.com/brave/brave` and recovered command paths are available with
`PATH=/tmp/agentbounties-ui-tools/bin:$PATH`.

## Font licensing

Cal Sans is bundled under its included OFL. Satoshi is acquired unmodified
from the official Fontshare download for this site's own use. Its included FFL
permits self-hosting but excludes repository redistribution of the font binary.
The binary is ignored by Git and fetched with a pinned SHA-256 by
`prepare-site-fonts.py` before preview/build. The Pages workflow prepares it in
both validation and deployment artifacts. Browsers make no third-party font
requests.

## Maintainer notice prepared for review

Replace the former scene-led homepage with shared forest visual components,
connected-AI posting presentation, and a read-only leaderboard. Preserve
account APIs, wallet authority, saved drafts, exact-content approvals, canonical
funding evidence, existing URLs, and WebMCP contracts.

The open PR queue was inspected before edits. Potential overlaps were #1196
(email/password accounts; not mergeable), #908 (canonical homepage links; not
mergeable), #910 (analytics; draft, not mergeable), and #1411 (wallet UX bounty
fixtures; draft, mergeable). Their backend behavior and bounty terms are outside
this change. This notice has not been posted because authorization is for local
implementation.
