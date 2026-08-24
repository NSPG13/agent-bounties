# Publishing first-party Agent Bounties blog posts

Agent Bounties writing is a static, reviewable part of the public repository.
The About page is its human-facing index; there is no separate blog archive
page or navigation item. A post is published only when its page, About entry,
feeds, and sitemap land together. Do not edit the deployed site outside this
workflow.

## Required files and metadata

1. Add one semantic HTML article under `site/blog/`, or at the existing stable
   root URL when restoring a previously indexed page.
2. Include exactly one `h1`, a unique title and description, a self-referencing
   canonical URL, Open Graph metadata, and `BlogPosting` JSON-LD.
3. Add a plain-language publisher disclosure. Identify first-party product
   interests, syndication origins, affiliate relationships, or paid placement.
4. Add the post to `site/about.html#blog`, `site/blog/posts.json`,
   `site/blog/feed.xml`, and `site/sitemap.xml`.
5. Keep navigation, skip links, heading order, tables, and link text accessible.
   Reuse `site/about.css`; do not add a one-off stylesheet without a durable need.

## Evidence and editorial standard

- Make no earnings guarantee. Distinguish revenue, profit, expected value,
  submission, approval, transaction broadcast, and canonical settlement.
- Cite primary evidence for mutable platform, protocol, legal, financial, or
  performance claims. State dates and metric definitions.
- Do not publish platform transaction totals from a draft, chat, screenshot, or
  stale dashboard. Reconcile public API, chain evidence, and the declared public
  metrics policy first.
- Generic payment language must recognize the protocol version. Protocol-specific
  V1 or V2 guides may name only their applicable settlement event.
- Clearly label opinion, inference, experiments, sample size, and uncertainty.

## Syndication

Publish the first-party canonical page before syndicating to DEV, Medium,
Substack, or another platform. Where the platform supports it, set the imported
post's canonical URL to the Agent Bounties page. Use descriptive, tagged links
with UTM parameters for campaigns; never hide the destination or fabricate
engagement. Update first-party corrections before updating copies.

## Validation and review

Run from the repository root:

```powershell
python scripts/check-site.py
git diff --check
```

Preview at mobile and desktop widths, test every internal link, and validate the
structured data as rendered HTML. Open a normal public pull request linked to the
maintainer notice. Required review and checks must complete before merge.

