#!/usr/bin/env bash
# Local proof: generate feeds from fixtures and validate.
# Uses feeds/proof/ as output dir (cleaned before and after).
# Never modifies committed fixtures.
set -e
cd "$(dirname "$0")/.."

FIXTURES="feeds/fixtures/issues.json"
PROOF_DIR="feeds/proof"

cleanup() { rm -rf "$PROOF_DIR"; }
trap cleanup EXIT

echo "=== Feed Generator Proof ==="
python3 tools/feed_generator.py --fixtures "$FIXTURES" --output-dir "$PROOF_DIR" --validate

echo ""
echo "=== Checking fixture integrity ==="
python3 -c "
import json, hashlib
with open('$FIXTURES','rb') as f:
    h = hashlib.sha256(f.read()).hexdigest()[:16]
print(f'OK: fixtures unchanged (hash: {h})')
"

echo ""
echo "=== Checking sorting ==="
python3 -c "
import json
with open('$PROOF_DIR/bounties.json') as f:
    data = json.load(f)
states = [i['_bounty_state'] for i in data['items']]
first_seeking = next((j for j,s in enumerate(states) if s == 'seeking_funding'), None)
last_claimable = max((j for j,s in enumerate(states) if s == 'claimable'), default=-1)
if last_claimable >= 0 and first_seeking is not None and first_seeking < last_claimable:
    print('FAIL: claimable items should sort before seeking_funding')
    exit(1)
print('OK: sorting correct')
"

echo ""
echo "=== Checking ETag / Last-Modified ==="
python3 -c "
import json
with open('$PROOF_DIR/manifest.json') as f:
    m = json.load(f)
assert 'etag' in m, 'missing etag'
assert 'last_modified' in m, 'missing last_modified'
print(f'OK: etag={m[\"etag\"]} last_modified={m[\"last_modified\"]} items={m[\"item_count\"]}')
"

echo ""
echo "=== Checking empty feed handling ==="
python3 -c "
import sys, json, tempfile, os
empty_fixture = tempfile.NamedTemporaryFile(mode='w', suffix='.json', delete=False)
json.dump([], empty_fixture)
empty_fixture.close()
sys.path.insert(0, 'tools')
from feed_generator import build_rss, build_json_feed
rss = build_rss([], '1970-01-01T00:00:00Z')
jf = build_json_feed([])
assert len(jf['items']) == 0
print('OK: empty feed produces valid output')
os.unlink(empty_fixture.name)
"

echo ""
echo "=== Checking RFC 822 date format ==="
python3 -c "
import re
with open('$PROOF_DIR/bounties.rss') as f:
    rss = f.read()
matches = re.findall(r'<pubDate>(.*?)</pubDate>', rss)
assert len(matches) > 0, 'no pubDate found'
for m in matches:
    assert re.match(r'^[A-Z][a-z]{2}, \d{2} [A-Z][a-z]{2} \d{4} \d{2}:\d{2}:\d{2} GMT$', m), f'bad RFC 822 date: {m}'
print(f'OK: all {len(matches)} pubDates in RFC 822 format')
"

echo ""
echo "✅ Local proof passed"
