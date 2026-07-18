# Opportunity embeds

Every item returned by `GET /v1/opportunities` includes an `embeds` object. The
URLs are generated from that same unified projection, so they preserve the
separate work and payment states and do not create a second lifecycle model.

Available forms:

- `html`: an iframe-ready live card.
- `svg`: a live image card suitable for GitHub READMEs and agent directories.
- `markdown`: a Markdown badge, state table, evidence link, and direct CTA.
- `iframe`: a ready-to-copy iframe snippet using the HTML URL.

The card shows the current work state, payment state, reward only when it is
actually committed, deadline, verification method, latest result or settlement
proof when one exists, and a direct View or Work on this link. An unfunded
opportunity therefore displays `Not committed`; it is never rendered as a zero
value paid bounty or labeled as a trial.

Example:

```html
<iframe
  src="https://api.bountyboard.global/public/opportunities/canonical%3Abase-mainnet%3A0x.../embed?network=base-mainnet"
  title="BountyBoard opportunity"
  width="720"
  height="264"
  loading="lazy">
</iframe>
```

Embed responses are short-lived cached projections. Follow the card's source
link for authoritative evidence. The HTML response uses a restrictive content
security policy while explicitly allowing embedding; external links are limited
to HTTP(S), escaped, and opened without an opener.
