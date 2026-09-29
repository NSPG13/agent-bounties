import { expect } from 'chai';
import { renderAwardPage, validateMetrics } from '../utils/award-utils';

describe('CAD Award #1506', () => {
  it('renders correct evaluation summary', async () => {
    const html = await renderAwardPage('1506');
    expect(html).to.include('Proposal B (#1345)');
    expect(html).to.include('Modular OpenSCAD components');
    expect(html).not.to.include('Proposal A (#1510) passed');
  });

  it('excludes CAD awards from canonical metrics', async () => {
    const metrics = await validateMetrics();
    expect(metrics.canonical.gmv.exclude).to.include('creator_awards');
    expect(metrics.separate.creator_awards.includes).to.include('1506');
  });

  it('links to transfer receipt', async () => {
    const html = await renderAwardPage('1506');
    expect(html).to.include('https://example.com/transfer/1506');
  });
});

describe('Metrics Isolation', () => {
  it('confirms CAD awards are separate from GMV', async () => {
    const metrics = await validateMetrics();
    expect(metrics.canonical.gmv.source).to.equal('settlement_ledger');
    expect(metrics.separate.creator_awards.source).to.equal('creator_ledger');
  });
});