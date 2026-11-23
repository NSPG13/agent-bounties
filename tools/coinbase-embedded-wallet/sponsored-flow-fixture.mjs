// Opt-in integration: real site + real local API/Postgres + real EIP-712
// signatures from a public test key. OAuth/CDP transport/Base are simulated.
// Never point this at a hosted API or a real wallet.
import assert from 'node:assert/strict';
import {before, after} from 'node:test';
import {readFile} from 'node:fs/promises';
import {createServer} from 'node:http';
import {createHmac} from 'node:crypto';
import {execFileSync} from 'node:child_process';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {build} from 'esbuild';
import {chromium} from 'playwright';
import {privateKeyToAccount} from 'viem/accounts';
import {verifyTypedData} from 'viem';
import {fakeSdk} from './test-auth-sdk.mjs';

export const api = process.env.POSTING_TEST_API_URL;
if (!api || !/^http:\/\/127\.0\.0\.1:\d+$/.test(api)) throw new Error('POSTING_TEST_API_URL must name an isolated local API with a disposable database.');
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../site');
const account = privateKeyToAccount('0x' + '1'.padStart(64, '0')); // Public fixture, never fund.
const wallet = account.address.toLowerCase(), hash = '0x'+'ab'.repeat(32);
const database=process.env.POSTING_TEST_DATABASE_URL;
if(!database||!/^postgres:\/\/posting_test@127\.0\.0\.1:\d+\/posting_test$/.test(database))throw new Error('Use only the disposable posting_test database on loopback.');
const secret='local-posting-test-only-not-a-real-secret';
const hmac=value=>createHmac('sha256',secret).update(value).digest('hex');
const user={provider:'google',sub:'local-posting-test',name:'Local test creator',email:'test@example.invalid',avatar:'',iat:Math.floor(Date.now()/1000),exp:Math.floor(Date.now()/1000)+3600};
const encoded=Buffer.from(JSON.stringify(user)).toString('hex');
const cookie=`agent_bounties_session=${encoded}.${hmac(encoded)}`;
let browser, server, origin, adapter;
before(async () => {
  const built = await build({entryPoints:[path.resolve(root,'../tools/coinbase-embedded-wallet/src/index.js')],bundle:true,write:false,format:'iife',platform:'browser',define:{'process.env.NODE_ENV':'"production"'},
    plugins:[{name:'local-cdp-fixture',setup(builder){builder.onResolve({filter:/^@coinbase\/cdp-/},()=>({path:'sdk',namespace:'fixture'}));builder.onLoad({filter:/.*/,namespace:'fixture'},()=>({contents:fakeSdk,loader:'js',resolveDir:path.resolve(root,'../tools/coinbase-embedded-wallet')}));}}]});
  adapter = built.outputFiles[0].text;
  server=createServer(async(req,res)=>{
    const pathname=new URL(req.url,'http://localhost').pathname;
    const filename=path.resolve(root,'.'+(pathname==='/'?'/index.html':pathname));
    if(!filename.startsWith(root+path.sep)){res.writeHead(403).end();return;}
    try {let body=await readFile(filename);
      if(pathname==='/wallet-config.js') body=Buffer.from(body.toString().replace('__COINBASE_CDP_PROJECT_ID__','qa-wallet-project'));
      if(pathname==='/protocol.json') body=Buffer.from(JSON.stringify({...JSON.parse(body),api_base_url:api}));
      res.setHeader('Content-Type',({'.js':'text/javascript','.css':'text/css','.html':'text/html','.json':'application/json','.svg':'image/svg+xml','.webp':'image/webp'})[path.extname(filename)]||'application/octet-stream');res.end(body);
    }catch{res.writeHead(404).end();}
  });
  await new Promise(resolve=>server.listen(39230,'127.0.0.1',resolve));origin=`http://127.0.0.1:${server.address().port}`;
  // Each suite starts with only this public fixture account's drafts removed.
  // The URI guard above prevents pointing this cleanup at a hosted database.
  execFileSync(process.env.PSQL_BIN||'psql',[database,'-v','ON_ERROR_STOP=1','-c',`DELETE FROM site_posting_drafts WHERE account_id='${hmac('google\0local-posting-test')}'`],{stdio:'pipe'});
  execFileSync(process.env.PSQL_BIN||'psql',[database,'-v','ON_ERROR_STOP=1','-c',`INSERT INTO site_auth_accounts(account_key,provider,provider_subject,display_name) VALUES ('${hmac('google\0local-posting-test')}','google','local-posting-test','Local test creator') ON CONFLICT DO NOTHING`],{stdio:'pipe'});
  const headers={Cookie:cookie,Origin:origin,'Content-Type':'application/json'};
  const challenge=await fetch(api+'/v1/site-auth/wallet/challenge',{method:'POST',headers,body:JSON.stringify({address:wallet})});
  assert.equal(challenge.status,201,await challenge.clone().text());const proof=await challenge.json();
  const verified=await fetch(api+'/v1/site-auth/wallet/verify',{method:'POST',headers,body:JSON.stringify({address:wallet,challenge_id:proof.challenge_id,signature:await account.signMessage({message:proof.message}),provider_id:'coinbase-embedded'})});
  assert.equal(verified.status,200,await verified.text());
  browser=await chromium.launch({headless:true,executablePath:process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE||undefined});
});
after(async()=>{await browser?.close();if(server)await new Promise(resolve=>server.close(resolve));});

export async function setup(options={}) {
  // Sequential scenarios have independent drafts. The exact fixture principal
  // and strict loopback/disposable database guard above bound this reset. Keep
  // every journal within a scenario (including concurrent tabs) unchanged.
  // Do not raise the application's 50-draft limit for a larger test matrix.
  execFileSync(process.env.PSQL_BIN||'psql',[database,'-v','ON_ERROR_STOP=1','-c',`DELETE FROM site_posting_drafts WHERE account_id='${hmac('google\0local-posting-test')}'`],{stdio:'pipe'});

  // Each sequential scenario owns a fresh local daily allowance. Keep the real
  // API's limit at three; never change it to make browser tests pass. The strict
  // loopback/database URI guard above and exact fixture principal bound this reset.
  execFileSync(process.env.PSQL_BIN||'psql',[database,'-v','ON_ERROR_STOP=1','-c',`DELETE FROM security_quota_reservations WHERE principal_key='account:${hmac('google\0local-posting-test')}' AND action='publication'`],{stdio:'pipe'});
  const context=await browser.newContext({viewport:options.mobile?{width:390,height:844}:{width:1440,height:1000},timezoneId:'America/Mexico_City'});
  const page=await context.newPage();page.setDefaultTimeout(15000);
  const state={authenticated:options.authenticated!==false,calls:[],errors:[],requests:[],tools:[],canonicalChecks:[],draftWrites:[],plan:null,create:null,terms:null,sent:0,signed:0,relayRequests:[],relayStatus:options.relayStatus??200,canonical:false,inventory:false,usdc:options.usdc??'0x5f5e100',eth:options.eth??'0x2386f26fc10000',rpcUnavailable:false,inventoryContract:null};
  page.on('pageerror',error=>state.errors.push(error.message));
  const eventData=()=>state.canonical&&state.plan?[
    {kind:'canonical_bounty_created',contract_address:state.plan.factory_contract,data:{bounty_contract:state.plan.predicted_bounty_contract,terms_hash:state.terms.terms_hash}},
    {kind:'funding_added',contract_address:state.plan.predicted_bounty_contract,data:{funded_amount:'2010000',target_amount:'2010000'}},
    {kind:'bounty_became_claimable',contract_address:state.plan.predicted_bounty_contract,data:{funded_amount:'2010000'}},
  ].map(event=>({...event,network:'base-mainnet',bounty_id:state.plan.bounty_id,tx_hash:hash,block_number:100})):[];
  const inventory=()=>state.inventory&&state.plan?[{opportunity_id:`canonical:base-mainnet:${state.plan.predicted_bounty_contract}`,source_id:state.plan.predicted_bounty_contract,network:'base-mainnet',source_type:'canonical_base',source_status:'claimable',work_state:'claimable',payment_state:'escrowed',payment_committed:true,verification_ready:true,terms_hash:state.terms.terms_hash,title:'Local integration test',reward:{amount:'2000000',unit:'base_units',decimals:6},funded_amount:{amount:'2010000',unit:'base_units',decimals:6},funding_target:{amount:'2010000',unit:'base_units',decimals:6}}]:[];
  await context.exposeFunction('testRecordTool',entry=>{state.tools.push(entry);});
  await context.exposeFunction('testCdpRequest',async request=>{
    state.calls.push(request);
    if(request.method==='eth_signTypedData_v4'){state.signed++;state.typedData=JSON.parse(request.params[1]);if(options.sdkTimeout)throw new Error('Simulated lost signing reply');return account.signTypedData(state.typedData);}
    if(request.method==='eth_sendTransaction'){state.sent++;throw new Error('Sponsored browser fixture forbids customer transactions');}
    throw new Error('Unexpected signing request '+request.method);
  });
  await context.exposeFunction('testRecordCanonical',result=>state.canonicalChecks.push(result));
  await context.addInitScript(({wallet})=>{
    window.addEventListener('agent-bounties:posting-canonical',event=>void window.testRecordCanonical(event.detail));
    window.testCdpAddress=wallet;window.walletTestCalls=[];window.localPageTools=new Map();window.testToolInvocations=[];
    window.testNativeWebMCP=typeof document.modelContext?.registerTool==='function';
    Object.defineProperty(document,'modelContext',{value:{registerTool(tool){window.localPageTools.set(tool.name,{...tool,execute:async input=>{const entry={name:tool.name,at:new Date().toISOString()};window.testToolInvocations.push(entry);await window.testRecordTool(entry);return tool.execute(input);}});}},configurable:true});
  },{wallet});
  await context.route('**/*',async route=>{
    const request=route.request(),url=new URL(request.url()),pathname=url.pathname;
    if(['/auth/session','/auth/account','/v1/site-auth/session','/v1/site-auth/account'].includes(pathname)||/^\/v1\/site-auth\/posting-drafts\/[0-9a-f-]{36}$/.test(pathname)){
      const routePath=pathname.startsWith('/auth/')?pathname.replace('/auth/','/v1/site-auth/'):pathname;
      if(request.method()==='POST'&&routePath.includes('/posting-drafts/')){const body=request.postDataJSON();state.draftWrites.push({operation_id:routePath.split('/').at(-1),expected_revision:body.expected_revision,recovery:body.recovery_state});if(body.recovery_state?.submission_attempt_id&&state.reservationBarrier)await state.reservationBarrier();}
      const response=await fetch(api+routePath,{method:request.method(),headers:{Cookie:state.authenticated?cookie:'',Origin:origin,'Content-Type':'application/json'},body:request.postData()||undefined});
      const body=await response.text();state.requests.push({path:routePath,status:response.status,body:response.ok?null:body});
      return route.fulfill({status:response.status,contentType:'application/json',body});
    }
    if(pathname==='/v1/base/gas-sponsorship')return route.fulfill({json:{schema:'agent-bounties/gas-sponsorship-v1',network:'base-mainnet',creation:{available:options.sponsorAvailable!==false}}});
    if(pathname==='/v1/base/autonomous-bounties/creation-relay'){
      const body=request.postDataJSON();state.relayRequests.push(body);
      try {
        assert.equal(request.method(),'POST');assert.equal(body.network,'base-mainnet');
        assert.deepEqual(body.create,state.create);assert.equal(body.authorization_valid_before,Number(state.typedData.message.validBefore));
        const sig=body.signature;
        const signature=sig.r+sig.s.slice(2)+Number(sig.v).toString(16).padStart(2,'0');
        assert.equal(await verifyTypedData({...state.typedData,address:wallet,signature}),true);
        // Pure local plan validation only. The real relay and all chain writes are
        // intentionally excluded: this route is a simulated sponsor boundary.
        const response=await fetch(api+'/v1/base/autonomous-bounties/authorized-creation-plan',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(body)});
        const text=await response.text();state.requests.push({path:'fixture:authorized-creation-plan',status:response.status});
        assert.equal(response.status,200,text);
        if(state.relayBarrier)await state.relayBarrier();
        if(state.relayStatus!==200)return route.fulfill({status:state.relayStatus,json:{error:'simulated sponsor unavailable'}});
        if(options.lostReply)return route.abort('failed');
        const relay={network:'base-mainnet',bountyContract:options.wrongRelayBounty?'0x'+'99'.repeat(20):state.plan.predicted_bounty_contract,transaction:hash};
        if(!options.deferCanonical&&!options.wrongRelayBounty&&!options.wrongRelayGas){state.canonical=true;state.inventory=!options.delayInventory;}
        return route.fulfill({json:{schema:'agent-bounties/sponsored-creation-v1',customer_gas_wei:options.wrongRelayGas?'1':'0',relay}});
      }catch(error){state.errors.push(error.message);return route.fulfill({status:500,json:{error:'fixture rejected request'}});}
    }
    if(/^\/v1\/base\/autonomous-bounties\/terms\/0x[0-9a-f]{64}$/.test(pathname)){
      const response=await fetch(api+pathname);return route.fulfill({status:response.status,contentType:'application/json',body:await response.text()});
    }
    if(pathname==='/v1/base/autonomous-bounties/events')return route.fulfill({json:eventData()});
    if(pathname==='/v1/opportunities')return route.fulfill({json:{schema_version:'agent-bounties/opportunity-projection-v1',network:'base-mainnet',applied_view:'ready_to_earn',degraded:false,coverage:'complete',coverage_scope:'inventory_page',has_more:false,source_statuses:[{source_type:'canonical_base',available:true,coverage:'complete',protocol_statuses:['autonomous-v1','open-competition-v1','open-competition-v2-beta3'].map(protocol=>({protocol,coverage:'complete',error:null,item_count:protocol==='autonomous-v1'?inventory().length:0}))}],items:inventory().map(item=>state.inventoryContract?{...item,source_id:state.inventoryContract}:item)}});
    if(pathname==='/v1/base/autonomous-bounties/feed')return route.fulfill({json:inventory().map(item=>({...item,bounty_contract:item.source_id,terms_valid:true}))});
    if(url.origin==='https://mainnet.base.org'){
      const rpc=request.postDataJSON();state.requests.push(rpc);
      if(state.rpcUnavailable)return route.fulfill({status:503,json:{error:'temporary failure'}});
      const result={eth_chainId:'0x2105',eth_blockNumber:'0x64',eth_getBalance:state.eth,eth_getCode:'0x',eth_call:rpc.params?.[0]?.data?.startsWith('0x70a08231')?state.usdc:'0x64',eth_gasPrice:'0x3',eth_estimateGas:'0x100000',eth_getTransactionReceipt:{status:'0x1',transactionHash:hash,blockNumber:'0x64'}}[rpc.method];
      assert.notEqual(result,undefined,'Unexpected RPC '+rpc.method);return route.fulfill({json:{jsonrpc:'2.0',id:rpc.id,result}});
    }
    if(pathname==='/vendor/coinbase-embedded-wallet.bundle.js')return route.fulfill({contentType:'text/javascript',body:adapter});
    if(pathname==='/vendor/coinbase-embedded-wallet.bundle.css')return route.fulfill({contentType:'text/css',body:''});
    if(pathname==='/v1/legal/policy'&&state.legalUnavailable)return route.fulfill({status:503,json:{error:'simulated policy outage'}});
    // Only these harmless local API routes are permitted. No broadcast/indexing.
    if(['/v1/legal/policy','/v1/legal/acceptances','/v1/base/autonomous-bounties/terms','/v1/base/autonomous-bounties/creation-plan','/v1/base/autonomous-bounties/authorized-creation-plan'].includes(pathname)){
      const headers={'Content-Type':'application/json',Origin:origin};
      if(state.authenticated)headers.Cookie=cookie;
      // Preserve the actual browser key; an omitted key must fail in the real API.
      const supplied=await request.allHeaders();
      for(const name of ['idempotency-key','authorization','x-agent-bounties-acquisition-id','x-agent-bounties-handoff-id'])if(supplied[name])headers[name]=supplied[name];
      const response=await fetch(api+pathname,{method:request.method(),headers,body:request.postData()||undefined});
      let body=await response.text();if(pathname==='/v1/legal/policy'&&state.legalInvalidHash)body=JSON.stringify({...JSON.parse(body),statement_hash:'sha256:'+'0'.repeat(64)});if(pathname==='/v1/legal/policy'&&state.legalChanged)body=JSON.stringify({...JSON.parse(body),terms_version:'test-new-policy'});state.requests.push({path:pathname,status:response.status,body:response.ok?null:body});
      if(response.ok&&pathname.endsWith('/terms')){state.terms=JSON.parse(body);state.publicationWire=request.postDataJSON();state.publicationKey=supplied['idempotency-key'];if(options.corruptTerms)body=JSON.stringify({...state.terms,document:{...state.terms.document,title:'Unapproved changed title'}});}
      if(response.ok&&pathname.endsWith('/creation-plan')){state.plan=JSON.parse(body);state.create=request.postDataJSON().create;}
      return route.fulfill({status:response.status,contentType:'application/json',body});
    }
    if(url.origin===origin)return route.continue();
    return route.fulfill({status:404,json:{error:'isolated test: request not permitted'}});
  });
  await page.goto(origin+'/post.html?from=webmcp&analytics=off');
  await page.waitForFunction(()=>window.localPageTools?.has('agent_bounties_stage_funded_bounty')&&window.AgentBountiesComposer);
  state.nativeWebMCP=await page.evaluate(()=>window.testNativeWebMCP);
  await page.evaluate(()=>window.localPageTools.get('agent_bounties_get_posting_options').execute({work_type:'outreach'}));
  const draft={title:'Local company response test',goal:'Deliver one company response with a concrete digital task.',acceptance_criteria:['Provide a company domain and a dated response describing a specific task.'],solver_reward_usdc:'2.00',verifier_reward_usdc:'0.01',task_window_days:30,review_mode:'creator',delivery_deadline:new Date(Date.now()+30*86400000-6*3600000).toISOString().replace('Z','-06:00')};
  await page.evaluate(draft=>window.localPageTools.get('agent_bounties_stage_funded_bounty').execute(draft),draft);
  return {context,page,state,draft};
}
export async function connect({page,state}) {
  await page.locator('[data-approve-card]').click();
  await page.locator('[data-wallet-options] button').filter({hasText:'Saved to your account'}).first().click();
  await page.getByRole('button',{name:'Use this wallet',exact:true}).click();
  await page.getByRole('button',{name:'Coinbase Use your Base app or Coinbase account wallet.',exact:true}).click();
  await page.getByText('Use an email wallet instead',{exact:true}).click();
  await page.getByRole('button',{name:/Use Coinbase with email/}).click();
  await page.getByRole('button',{name:'Complete email verification',exact:true}).click();
  await page.waitForFunction(()=>document.querySelector('[data-wallet-state]').textContent.includes('Connected for this session'));
  await page.waitForFunction(()=>['review','topup'].includes(document.querySelector('#funding-dialog').dataset.fundingView));
  assert.equal(state.signed,0);assert.equal(state.sent,0);
}
export async function signFunding({page,state}) {
  await page.locator('[data-legal-consent-checkbox]').check();
  await page.getByRole('button',{name:'Review and post',exact:true}).click();
  try{await page.getByRole('dialog',{name:'Approve bounty funding signature',exact:true}).waitFor();}
  catch(error){throw new Error(`${error.message}\nAPI: ${JSON.stringify(state.requests.filter(request=>request.path))}\nPage: ${await page.locator('[data-payment-status]').innerText()}`);}
  await page.getByRole('button',{name:'Sign funding authorization',exact:true}).click();
  // The current sponsor flow continues directly after one authorization.

}
