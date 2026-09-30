import assert from 'node:assert/strict';
import test from 'node:test';
import {AgentBountiesClient} from '../dist/index.js';

test('broad checks preserve exact plan, file key and caller authority', async () => {
  const previous=globalThis.fetch, calls=[];
  globalThis.fetch=async (url,init)=>{calls.push({url,init});return new Response(JSON.stringify({payment_authorized:false}),{status:200});};
  try {
    const client=new AgentBountiesClient({baseUrl:'https://api.example',sessionToken:`abws_${'a'.repeat(64)}`});
    const id='00000000-0000-4000-8000-000000000001', plan={schema_version:'broad-verification/v1',criteria:[{id:'work',text:'Actual files',review:'Does it solve the brief?'}],checks:[]};
    await client.uploadVerificationArtifact([{path:'work.txt',base64:'d29yaw=='}],'same-files');
    await client.runVerificationChecks(id,plan);
    await client.assignReviewTask(id,'account:'+'1'.repeat(64));
    await client.getVerificationRun(id);
    await client.openArtifactDispute(id,'Please keep these files.');
    await client.resolveArtifactDispute(id,'My review is done.');
    assert.equal(calls[0].init.headers['Idempotency-Key'],'same-files');
    assert.deepEqual(JSON.parse(calls[1].init.body),{artifact_id:id,plan});
    assert.deepEqual(JSON.parse(calls[2].init.body),{assignee_principal:'account:'+'1'.repeat(64)});
    assert.ok(calls[3].url.endsWith('/runs/'+id));
    assert.ok(calls[4].url.endsWith('/artifacts/'+id+'/disputes'));
    assert.deepEqual(JSON.parse(calls[4].init.body),{reason:'Please keep these files.'});
    assert.ok(calls[5].url.endsWith('/disputes/'+id+'/resolve'));
    for(const {init} of calls){assert.equal(init.headers.authorization,`Bearer abws_${'a'.repeat(64)}`);assert.equal(init.headers['x-operator-token'],undefined);}
  }finally{globalThis.fetch=previous;}
});
