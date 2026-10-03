"use strict";
const assert = require('node:assert/strict'), fs = require('node:fs'), path = require('node:path'), http = require('node:http');
const { chromium } = require('../tools/browser-layout/node_modules/playwright');
const h = require('../site/work-history.js'), fixture = require('./fixtures/public-wallet-work-history.json');
const wallet = '0x515d935be52e76f0d9b37aa99fc33947b3b65f62', site = path.resolve(__dirname,'../site');
const server = http.createServer((req,res)=>{
  const name=path.resolve(site,'.'+new URL(req.url,'http://localhost').pathname);
  if(!name.startsWith(site+path.sep))return res.writeHead(403).end();
  try { const body=fs.readFileSync(name); res.writeHead(200,{'Content-Type':({'.html':'text/html','.css':'text/css','.js':'text/javascript','.ttf':'font/ttf','.woff2':'font/woff2'})[path.extname(name)]||'application/octet-stream'}).end(body); } catch (_) {res.writeHead(404).end();}
});
(async()=>{
  await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));const origin=`http://127.0.0.1:${server.address().port}`;
  const browser=await chromium.launch({headless:true,executablePath:process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE||undefined});
  try {
    for(const width of [320,390,1440]){
      const context=await browser.newContext({viewport:{width,height:1000},reducedMotion:'reduce'});let mode='ready',calls=0;
      await context.route('**/*',route=>{
        const url=route.request().url(), source=h.SOURCES.find(s=>s.url===url);
        if(source){calls++;return mode==='unavailable'||(mode==='partial'&&source.id==='open-competition-v1')?route.fulfill({status:503,body:'Unavailable'}):route.fulfill({json:fixture[source.id]});}
        if(new URL(url).origin!==origin)return route.fulfill({status:503,body:'Isolated test'});
        return route.continue();
      });
      const page=await context.newPage(),errors=[];page.on('pageerror',error=>errors.push(error.message));
      await page.goto(`${origin}/leaderboard.html#wallet=${wallet}`);const input=page.getByRole('textbox',{name:'Public Base solver wallet'}),button=page.getByRole('button',{name:'Inspect work history'}),status=page.locator('[data-work-history-status]');
      assert.equal(await input.inputValue(),wallet);assert.equal(calls,0,'fragment prefill does not fetch history');
      await input.focus();await input.press('Enter');await page.waitForFunction(()=>document.querySelector('[data-work-history-status]').textContent.includes('2 settlement observations'));
      assert.equal(calls,3);assert.equal(await page.locator('[data-work-history-results] article').count(),2);
      await page.getByText('Inspect or copy the same JSON for your agent',{exact:true}).click();
      let snapshot=JSON.parse(await page.getByRole('textbox',{name:'Wallet work history JSON'}).inputValue());
      assert.equal(snapshot.solver_reward_total_base_units,'1600000');assert.equal(snapshot.records[0].returned_bond_base_units,'100000');assert.equal(snapshot.chain_revalidated,false);
      assert.ok((await status.textContent()).includes('1.6 USDC'));assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),'narrow layout has no page overflow');
      const refs=await page.locator('[data-work-history-results] article a').evaluateAll(links=>links.map(a=>a.href));assert.equal(refs.filter(url=>url.startsWith('https://basescan.org/tx/')).length,2);
      await page.reload();assert.equal(await input.inputValue(),wallet);assert.equal(calls,3,'reload does not refetch history');
      mode='partial';await button.click();await page.waitForFunction(()=>document.querySelector('[data-work-history-status]').textContent.startsWith('Partial results'));assert.equal(await page.locator('[data-work-history-results] article').count(),2);
      mode='unavailable';await button.click();await page.waitForFunction(()=>document.querySelector('[data-work-history-status]').textContent.startsWith('Sources unavailable'));assert.equal(await page.locator('[data-work-history-results] article').count(),0);assert.equal(await input.inputValue(),wallet);assert.equal(await button.isEnabled(),true);
      mode='ready';await input.fill('0x'+'1'.repeat(40));await input.press('Enter');await page.waitForFunction(()=>document.querySelector('[data-work-history-status]').textContent.includes('No matching indexed settlement'));assert.equal(await input.inputValue(),'0x'+'1'.repeat(40));
      const before=calls;await input.fill('invalid');await input.press('Enter');assert.equal(calls,before,'invalid input is rejected before requests');assert.equal(await input.evaluate(el=>el.validity.valid),false);
      assert.deepEqual(errors,[]);await context.close();console.log(`work-history keyboard, recovery, scoped JSON, proof links and layout pass at ${width}px`);
    }
  }finally{await browser.close();await new Promise(resolve=>server.close(resolve));}
})().catch(error=>{console.error(error);server.close();process.exitCode=1;});
