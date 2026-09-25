'use strict';
// Real production reads only. No fixture routes, sign-in, wallet, or record writes.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const { chromium } = require('../tools/browser-layout/node_modules/playwright');
const origin = 'https://agentbounties.app';
const contract = '0xe11965e2c68f6f4da4b97c8e62fda5015ae0137b';
const receipt = '0x618bfb43f359f7f21432f55b646a61b4251452f13071cfa4e5d6854142bc770b';
const url = `${origin}/submissions.html?opportunity=canonical:base-mainnet:${contract}`;
(async () => {
  fs.mkdirSync('proof', { recursive: true });
  const browser = await chromium.launch({ headless: true });
  const report = { observed_at: new Date().toISOString(), url, signed_in: false, cases: [] };
  try {
    for (const width of [1440, 390, 320]) {
      const context = await browser.newContext({ viewport: { width, height: 900 }, locale: 'en-US', timezoneId: 'UTC', reducedMotion: 'reduce' });
      const blockedWrites = [], errors = [];
      await context.route('**/*', route => {
        if (!['GET', 'HEAD', 'OPTIONS'].includes(route.request().method())) {
          blockedWrites.push(new URL(route.request().url()).origin);
          return route.abort();
        }
        return route.continue();
      });
      const page = await context.newPage();
      page.on('pageerror', error => errors.push(error.message));
      await page.goto(url, { waitUntil: 'domcontentloaded' });
      const verify = async phase => {
        const card = page.locator('#submission-1');
        await card.getByText('Review expired', { exact: true }).waitFor();
        const text = await card.innerText();
        assert.match(text, /2 USDC of claim bond was returned/);
        assert.match(text, /This round did not pay a solver reward/);
        assert.match(text, /bounty reopened for another attempt/);
        assert.equal(await card.getByRole('link', { name: 'View confirmed round outcome' }).getAttribute('href'), `https://basescan.org/tx/${receipt}`);
        assert.equal(await card.getByRole('link', { name: 'View confirmed payment' }).count(), 0);
        assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'Page must fit viewport');
        assert.equal(await page.locator('script[src="submissions.js?v=2"]').count(), 1);
        report.cases.push({ width, phase, status: 'Review expired', bond_returned_usdc: 2, solver_payment: false, receipt, passed: true });
      };
      await verify('initial');
      const refreshed = page.waitForResponse(response => response.url().includes('/v1/base/autonomous-bounties/feed?') && response.status() === 200);
      await page.getByRole('button', { name: 'Refresh', exact: true }).click();
      await refreshed;
      await verify('refresh');
      await page.reload({ waitUntil: 'domcontentloaded' });
      await verify('reload');
      await page.screenshot({ path: `proof/production-submissions-${width}.png`, fullPage: true });
      fs.writeFileSync(`proof/production-submissions-${width}.txt`, await page.locator('[data-submissions-page]').innerText());
      assert.deepEqual(errors, []);
      report.cases.push({ width, blocked_non_read_requests: blockedWrites, page_errors: errors });
      await context.close();
    }
    report.passed = true;
  } catch (error) {
    report.passed = false;
    report.error = error.message;
    process.exitCode = 1;
  } finally {
    await browser.close();
    fs.writeFileSync('proof/browser-proof.json', JSON.stringify(report, null, 2) + '\n');
    console.log('PRODUCTION_BROWSER_PROOF=' + JSON.stringify(report));
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
