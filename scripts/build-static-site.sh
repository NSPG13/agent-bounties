#!/usr/bin/env bash
# Provider-independent static build; no GitHub Actions runner or token required.
set -euo pipefail
cd "$(dirname "$0")/.."
: "${COINBASE_CDP_PROJECT_ID:?Set the existing public Coinbase project ID}"
: "${WALLETCONNECT_PROJECT_ID:?Set the existing public WalletConnect project ID}"
python3 -m pip install -r scripts/requirements-site.txt
npm ci --prefix tools/coinbase-embedded-wallet --ignore-scripts --no-audit --no-fund
npm rebuild --prefix tools/coinbase-embedded-wallet esbuild
npm run check --prefix tools/coinbase-embedded-wallet
COINBASE_WALLET_OUTDIR=site/vendor npm run build --prefix tools/coinbase-embedded-wallet
python3 scripts/configure-wallet-providers.py --verify-origin https://agentbounties.app
python3 scripts/configure-phone-wallet.py
python3 scripts/prepare-site-fonts.py
mkdir -p site/schemas
cp schemas/discovery-manifest.v2.json site/schemas/discovery-manifest.v2.json
python3 scripts/refresh_site_metrics.py
python3 - <<'PY'
import json, os, re, subprocess
from pathlib import Path
value = os.environ.get('GA_MEASUREMENT_ID', '')
if value and not re.fullmatch(r'G-[A-Z0-9]+', value):
    raise SystemExit('Invalid public analytics configuration')
p = Path('site/analytics-config.js')
source, count = re.subn(r'googleMeasurementId:\s*"(?:|G-[A-Z0-9]+)"',
                      'googleMeasurementId: '+json.dumps(value), p.read_text(), count=1)
if count != 1:
    raise SystemExit('Expected exactly one analytics configuration assignment')
p.write_text(source)
revision = subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip()
Path('site/.well-known').mkdir(exist_ok=True)
Path('site/.well-known/site-build.json').write_text(json.dumps({
    'schema':'agent-bounties/static-build-v1','revision':revision,
    'participation_metrics':'refreshed from public GitHub activity during this build',
    'participation_generated_at':json.loads(Path('site/generated/github-participation.json').read_text())['generated_at']
})+'\n')
PY
python3 scripts/check-site.py --require-wallet-bundle
node scripts/test-creator-push.js
node --test scripts/test-wallet-link.js scripts/test-solarpunk-home.js
