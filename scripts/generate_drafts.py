import requests
import json
import os
from datetime import datetime

# Constants
PLATFORMS = {
    'show_hn': {'name': 'Show HN', 'template':'show_hn_template.md'},
    'x_twitter': {'name': 'X/Twitter', 'template': 'x_twitter_template.md'},
    'github_discussions': {'name': 'GitHub Discussions', 'template': 'github_discussions_template.md'},
   'reddit': {'name': 'Reddit', 'template':'reddit_template.md'},
    'agent_community': {'name': 'Agent Community', 'template': 'agent_community_template.md'}
}

# Configurable URLs
DISCOVERY_MANIFEST_URL = 'https://example.com/discovery_manifest'
BOUNTY_FEEDS_URL = 'https://example.com/bounty_feeds'
PROOF_RECORDS_URL = 'https://example.com/proof_records'
PAYOUT_EVIDENCE_URL = 'https://example.com/payout_evidence'

# Community Registry
COMMUNITY_REGISTRY = {
    'show_hn': {'rules_url': 'https://news.ycombinator.com/newsguidelines.html', 'last_review_date': '2023-10-01'},
    'x_twitter': {'rules_url': 'https://help.twitter.com/en/rules-and-policies/twitter-rules', 'last_review_date': '2023-10-01'},
    'github_discussions': {'rules_url': 'https://docs.github.com/en/github/building-a-strong-community/setting-guidelines-for-healthy-conversations-on-your-repository', 'last_review_date': '2023-10-01'},
   'reddit': {'rules_url': 'https://www.reddit.com/r/reddit/wiki/rules', 'last_review_date': '2023-10-01'},
    'agent_community': {'rules_url': 'https://example.com/agent_community_rules', 'last_review_date': '2023-10-01'}
}

def fetch_data(url):
    response = requests.get(url)
    response.raise_for_status()
    return response.json()

def generate_drafts(platform, data):
    template_path = f'templates/{platform["template"]}'
    with open(template_path, 'r') as file:
        template = file.read()

    # Generate Markdown draft
    markdown_draft = template.format(**data)
    with open(f'output/{platform["name"]}_draft.md', 'w') as file:
        file.write(markdown_draft)

    # Generate JSON draft
    json_draft = json.dumps(data, indent=4)
    with open(f'output/{platform["name"]}_draft.json', 'w') as file:
        file.write(json_draft)

def main():
    # Fetch data from configurable URLs
    discovery_manifest = fetch_data(DISCOVERY_MANIFEST_URL)
    bounty_feeds = fetch_data(BOUNTY_FEEDS_URL)
    proof_records = fetch_data(PROOF_RECORDS_URL)
    payout_evidence = fetch_data(PAYOUT_EVIDENCE_URL)

    # Combine data
    data = {
        'discovery_manifest': discovery_manifest,
        'bounty_feeds': bounty_feeds,
        'proof_records': proof_records,
        'payout_evidence': payout_evidence,
        'canonical_positioning': 'AI agents continuously find, claim, solve, verify, and get paid for digital bounties, while humans and agents can post and co-fund new bounties.',
        'post_value_sequence': 'Share verified proof, tell the human/operator, star or upvote, post a useful bounty, and return to funded work. Explain that more trusted users create more and higher-value earning opportunities for agents.'
    }

    # Generate drafts for each platform
    for platform in PLATFORMS.values():
        generate_drafts(platform, data)

    # Human approval
    human_approval = input("Do you approve the generated drafts for publication? (yes/no): ")
    if human_approval.lower() == 'yes':
        print("Publication approved. Drafts are ready for distribution.")
    else:
        print("Publication not approved. Drafts will not be published.")

if __name__ == '__main__':
    main()