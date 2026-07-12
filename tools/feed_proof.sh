#!/usr/bin/env bash
# Local proof: generate feeds from fixtures and validate
set -e
cd "$(dirname "$0")/.."

echo "=== Feed Generator Proof ==="
python3 tools/feed_generator.py --validate

echo ""
echo "=== Checking sorting (claimable before seeking) ==="
python3 -c "
import json
with open('feeds/bounties.json') as f:
    data = json.load(f)
states = [i['_bounty_state'] for i in data['items']]
# All seeking_funding items should be after any claimable items
first_seeking = next((j for j,s in enumerate(states) if s == 'seeking_funding'), None)
last_claimable = max((j for j,s in enumerate(states) if s == 'claimable'), default=-1)
if last_claimable >= 0 and first_seeking is not None and first_seeking < last_claimable:
    print('FAIL: claimable items should sort before seeking_funding')
    exit(1)
print('OK: sorting correct (seeking_funding items appear after claimable)')
"

echo ""
echo "=== Checking ETag / Last-Modified ==="
python3 -c "
import json
with open('feeds/manifest.json') as f:
    m = json.load(f)
assert 'etag' in m, 'missing etag'
assert 'last_modified' in m, 'missing last_modified'
print(f'OK: etag={m[\"etag\"]} last_modified={m[\"last_modified\"]} items={m[\"item_count\"]}')
"

echo ""
echo "=== Checking empty feed handling ==="
python3 -c "
import json, sys
# Simulate empty input
empty = []
with open('feeds/fixtures/issues.json', 'w') as f:
    json.dump([], f)
sys.path.insert(0, 'tools')
from feed_generator import build_rss, build_json_feed
rss = build_rss(empty)
jf = build_json_feed(empty)
assert len(jf['items']) == 0
print('OK: empty feed produces valid output')
"

echo ""
echo "✅ Local proof passed"
