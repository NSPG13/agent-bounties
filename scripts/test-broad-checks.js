'use strict';
const {test}=require('node:test'),assert=require('node:assert/strict');
const {webcrypto}=require('node:crypto');
const checks=require('../site/check-work.js');
const browser={crypto:webcrypto,btoa:value=>Buffer.from(value,'binary').toString('base64')};
function file(name,body,path='') {
  const bytes=Buffer.from(body);
  return {name,size:bytes.length,webkitRelativePath:path,arrayBuffer:async()=>bytes.buffer.slice(bytes.byteOffset,bytes.byteOffset+bytes.length)};
}
test('retry keys survive reloads and selection order but change with paths or bytes',async()=>{
  const work=[file('a.txt','one'),file('b.txt','two')];
  const first=await checks.prepareFiles(browser,work);
  assert.equal((await checks.prepareFiles(browser,[...work].reverse())).key,first.key);
  assert.notEqual((await checks.prepareFiles(browser,[file('a.txt','changed'),work[1]])).key,first.key);
  assert.notEqual((await checks.prepareFiles(browser,[file('renamed.txt','one'),work[1]])).key,first.key);
  assert.equal((await checks.prepareFiles(browser,[file('a.txt','one','chosen/a.txt'),file('b.txt','two','chosen/b.txt')])).key,first.key);
  const nested=await checks.prepareFiles(browser,[file('main.py','code','root/src/main.py')]);
  assert.equal(nested.files[0].path,'src/main.py');
  await assert.rejects(checks.prepareFiles(browser,[file('a','one'),file('a','two')]),/same path/);
});
test('unsafe integer plans are not rounded into different accepted rules',()=>{
  assert.equal(checks.safeNumbers({rule:{min:9007199254740992}}),false);
  assert.equal(checks.safeNumbers({rule:{min:42}}),true);
});
test('a later plan selection wins even when an earlier validation finishes last',async()=>{
  const elements=new Map(),calls=[],pending=[];
  function element(){return {files:[],value:'',textContent:'',children:[],handlers:{},addEventListener(name,fn){this.handlers[name]=fn;},append(...children){this.children.push(...children);},replaceChildren(){this.children=[];}};}
  function get(key){if(!elements.has(key))elements.set(key,element());return elements.get(key);}
  const form=get('[data-check-work]');form.querySelector=get;
  const win={...browser,document:{querySelector:get,createElement:element},location:{},sessionStorage:{getItem:()=>null,setItem(){throw Error('storage disabled');}},AgentBountiesWorkflow:{apiBase:()=> 'https://api.example'},fetch:async(url,init)=>{
    const body=init.body?JSON.parse(init.body):null;calls.push({url,body});
    if(url.endsWith('/plans/validate'))return new Promise(resolve=>pending.push(()=>resolve({ok:true,json:async()=>({valid:true})})));
    const data=url.endsWith('/artifacts')?{id:'00000000-0000-4000-8000-000000000001'}:url.endsWith('/runs')?{id:'00000000-0000-4000-8000-000000000002'}:{artifact_id:'00000000-0000-4000-8000-000000000001',status:'queued'};
    return {ok:true,json:async()=>data};
  }};
  await checks.start(win);
  const planInput=get('[name=plan]');
  const plan=n=>({criteria:[{id:n,text:n,review:'Review meaning'}],checks:[]});
  planInput.files=[{size:100,text:async()=>JSON.stringify(plan('old'))}];
  const old=planInput.handlers.change();await new Promise(setImmediate);
  planInput.files=[{size:100,text:async()=>JSON.stringify(plan('new'))}];
  const newer=planInput.handlers.change();await new Promise(setImmediate);
  pending[1]();await newer;pending[0]();await old;
  get('[name=files]').files=[file('work.txt','work')];
  await form.handlers.submit({preventDefault(){}});
  assert.deepEqual(calls.find(call=>call.url.endsWith('/runs')).body.plan,plan('new'));
  assert.equal(get('[data-check-status]').textContent,'Checks are queued or running. You can check again shortly.');
  assert.equal(get('[type=submit]').disabled,false);
});

test('submission and normal review deadlines are clear, with recovery in full rules',()=>{
  const element=tag=>({tag,textContent:'',children:[],append(...items){this.children.push(...items);}});
  const node=element('div'),doc={createElement:element};
  checks.showTimeline(doc,node,{funding_cutoff:100,submission_cutoff:200,checks_cutoff:86600,review_cutoff:259400,final_cutoff:345800});
  const section=node.children[0],visible=section.children.filter(n=>n.tag==='p').map(n=>n.textContent).join(' ');
  assert.match(visible,/Submit work by/);
  assert.match(visible,/Finish checks by/);
  assert.match(visible,/Reviewer must decide by/);
  assert.doesNotMatch(visible,/Late review recovery ends/);
  const details=section.children.find(n=>n.tag==='details');
  assert.match(details.children[1].textContent,/closes the new contest without a winner/);
  const unused=element('div');checks.showTimeline(doc,unused,{funding_cutoff:100,submission_cutoff:200,checks_cutoff:86600,review_cutoff:86600,final_cutoff:86600});
  assert.ok(!unused.children[0].children.some(n=>n.textContent.startsWith('Reviewer must')));
});
