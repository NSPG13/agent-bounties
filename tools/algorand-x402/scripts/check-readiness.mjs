import { readFile, writeFile } from 'node:fs/promises';
import { homedir } from 'node:os';
import { FACILITATOR, NETWORKS, TAG } from '../config.mjs';

const origin = process.argv[2];
if (!origin || !origin.startsWith('https://')) throw new Error('Usage: node scripts/check-readiness.mjs https://service-origin [output.json]');
const wallet = JSON.parse(await readFile(`${homedir()}/.config/agent-bounties/algorand-challenge/merchant.json`, 'utf8'));
const expected = NETWORKS.mainnet;
const endpoint = new URL('/v1/opportunity-report', origin).href;
async function json(url) { const r = await fetch(url, {signal:AbortSignal.timeout(20000)}); if(!r.ok)throw new Error(`${url}: HTTP ${r.status}`);return r.json(); }
async function pages(route, params) {
  const items = [];
  for (let offset=0;offset<20000;) {
    const u=new URL(route,FACILITATOR);for(const [k,v] of Object.entries({...params,offset}))u.searchParams.set(k,String(v));
    const d=await json(u);if(!Array.isArray(d.items))throw new Error('Invalid catalog response');items.push(...d.items);offset+=d.items.length;
    const total=d.pagination?.total??d.total;if(!Number.isInteger(total))throw new Error('Missing catalog total');
    if(offset>=total)return items;if(!d.items.length)throw new Error('Incomplete catalog');
  }
  throw new Error('Catalog pagination limit');
}
const report={checkedAt:new Date().toISOString(),endpoint,payTo:wallet.address,checks:{}};
const response=await fetch(endpoint,{signal:AbortSignal.timeout(30000)});
const raw=response.headers.get('payment-required');const challenge=raw?JSON.parse(Buffer.from(raw,'base64').toString()):null;
const acceptance=challenge?.accepts?.find(a=>a.network===expected.network&&a.asset===expected.asset&&a.payTo===wallet.address&&a.amount==='10000'&&a.scheme==='exact');
const [supported,manifest,resources,leaderboard]=await Promise.all([
  json(FACILITATOR+'/supported'),json(new URL('/.well-known/x402.json',origin)),
  pages('/discovery/resources',{includeTestnets:true,limit:1000}),
  pages('/data/leaderboards',{cat:'merchants',limit:50,range:'all',env:'mainnet',src:TAG}),
]);
report.checks.goplausibleConfigured=manifest.facilitator===FACILITATOR&&response.status===402&&!!acceptance&&supported.kinds.some(k=>k.network===expected.network&&k.scheme==='exact'&&k.extra?.feePayer===acceptance.extra?.feePayer);
report.checks.bazaarExtensionAndTag=!!challenge?.extensions?.bazaar&&acceptance?.extra?.tag===TAG;
report.catalogMatches=resources.filter(r=>(r.resourceUrl||r.resource)===endpoint);
report.leaderboardMatches=leaderboard.filter(r=>r.address===wallet.address);
report.checks.catalogVisible=report.catalogMatches.length>0;
report.checks.challengeLeaderboardVisible=report.leaderboardMatches.some(r=>r.challenge===true&&r.settles>0);
report.wallets=[];
for(const role of ['merchant','canary-payer']) {
  const w=JSON.parse(await readFile(`${homedir()}/.config/agent-bounties/algorand-challenge/${role}.json`,'utf8'));
  const a=await json(`${expected.algod}/v2/accounts/${w.address}`);
  const asset=a.assets?.find(a=>String(a['asset-id'])===expected.asset);
  report.wallets.push({role,address:w.address,algoMicroUnits:a.amount,usdcOptedIn:!!asset,usdcMicroUnits:asset?.amount??0});
}
report.checks.recipientReady=report.wallets[0].usdcOptedIn;
report.checks.realMainnetPaidResponseVerified=false;
report.note='This read-only checker cannot prove delivery. A successful canary receipt must also be reconciled on-chain; never infer delivery from a 402 or a listing.';
report.challenge=challenge;
if(process.argv[3])await writeFile(process.argv[3],JSON.stringify(report,null,2)+'\n',{mode:0o600});
console.log(JSON.stringify(report,null,2));
if(!Object.values(report.checks).every(Boolean))process.exitCode=2;
