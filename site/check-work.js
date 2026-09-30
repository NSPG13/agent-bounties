/* Evidence UI: private files, explicit check claims, no wallet or payment authority. */
(function(root,factory){const api=factory();if(typeof module==='object'&&module.exports)module.exports=api;if(root)root.AgentBountiesChecks=api;if(root?.document)root.addEventListener('DOMContentLoaded',()=>api.start(root));})(typeof window!=='undefined'?window:null,function(){
  'use strict';
  const labels={pass:'Pass',fail:'Fail',needs_review:'Needs review',unsupported:'Unsupported',blocked:'Blocked',check_unavailable:'Check unavailable'};
  async function request(win,path,body,key){const response=await win.fetch(`${win.AgentBountiesWorkflow.apiBase(win.location)}${path}`,{method:body===undefined?'GET':'POST',credentials:'include',cache:'no-store',headers:{Accept:'application/json',...(body===undefined?{}:{'Content-Type':'application/json'}),...(key?{'Idempotency-Key':key}:{})},...(body===undefined?{}:{body:typeof body==='string'?body:JSON.stringify(body)})});if(!response.ok)throw new Error(response.status===401?'Sign in to save and check your files.':response.status===429?'Checking capacity is full. Try again later with the same files.':response.status===503?'File storage or checking is unavailable. Keep your files and try again later.':`This step could not finish (${response.status}).`);return response.json();}
  function safeNumbers(value){if(typeof value==='number')return Number.isFinite(value)&&(!Number.isInteger(value)||Number.isSafeInteger(value));if(value&&typeof value==='object')return Object.values(value).every(safeNumbers);return true;}
  const claims={integrity_v1:'Stored files match their saved fingerprints.',contents_v1:'The named files and folders are present.',text_v1:'The exact required text is present and blocked text is absent.',json_v1:'The agreed JSON fields have the required types and values.',csv_v1:'The CSV columns, values and row count match the rules.',consistency_v1:'The chosen values match across files as required.',published_tests_v1:'The exact published tests run on these files.'};
  function showTimeline(doc,node,windows) {
    if(!windows)return;
    const section=doc.createElement('section'),heading=doc.createElement('h3');
    heading.textContent='When each step closes';section.append(heading);
    const date=seconds=>new Date(seconds*1000).toLocaleString(undefined,{dateStyle:'medium',timeStyle:'short',timeZoneName:undefined});
    const add=(label,seconds)=>{const p=doc.createElement('p');p.textContent=`${label}: ${date(seconds)} (your time)`;section.append(p);};
    add('Submit work by',windows.submission_cutoff);
    if(windows.checks_cutoff>windows.submission_cutoff)add('Finish checks by',windows.checks_cutoff);
    if(windows.review_cutoff>windows.checks_cutoff)add('Reviewer must decide by',windows.review_cutoff);
    const details=doc.createElement('details'),summary=doc.createElement('summary'),rules=doc.createElement('p');
    summary.textContent='Funding and recovery rules';
    rules.textContent=`Funding closes ${date(windows.funding_cutoff)}. `+(windows.final_cutoff>windows.review_cutoff?`Late review recovery ends ${date(windows.final_cutoff)}. An unresolved required review closes the new contest without a winner; the published refunds apply.`:'There is no separate review recovery window.');
    details.append(summary,rules);section.append(details);
    const notice=doc.createElement('p');notice.textContent='These are proposed rules for a new contest. An existing bounty keeps the dates in its funded terms.';section.append(notice);node.append(section);
  }
  function showPlan(doc,node,plan){node.replaceChildren();showTimeline(doc,node,plan.time_windows);for(const criterion of plan.criteria||[]){const section=doc.createElement('section'),title=doc.createElement('h3');title.textContent=criterion.text;section.append(title);const list=doc.createElement('ul');for(const check of (plan.checks||[]).filter(c=>c.criteria.includes(criterion.id))){const li=doc.createElement('li');li.textContent=`${check.purpose==='required'?'Required':check.purpose==='ranking'?'For ranking':'Optional'}: ${check.id} — ${claims[check.rule.checker]||'Read the agreed rule.'}`;list.append(li);const details=doc.createElement('details'),summary=doc.createElement('summary'),code=doc.createElement('pre');summary.textContent='See exact rule';code.textContent=JSON.stringify(check.rule,null,2);details.append(summary,code);li.append(details);}if(criterion.review){const li=doc.createElement('li');li.textContent=`Needs a reviewer: ${criterion.review}`;list.append(li);}section.append(list);node.append(section);}}
  function showReport(doc,node,report){node.replaceChildren();for(const check of report.checks||[]){const p=doc.createElement('p');p.textContent=`${labels[check.status]||'Check unavailable'} — ${check.id}: ${check.reason.replaceAll('_',' ')}`;node.append(p);}for(const question of report.review_questions||[]){const p=doc.createElement('p');p.textContent=`Needs review — ${question.review}`;node.append(p);}const p=doc.createElement('p');p.textContent='A passed check only proves its stated rule. It does not choose a winner or approve payment.';node.append(p);}
  async function prepareFiles(win, selected) {
    if (!selected.length || selected.length > 1024 || selected.reduce((n,f)=>n+f.size,0)>104857600) {
      throw new Error('Choose up to 100 MiB of files.');
    }
    const files=[], manifest=[], names=new Set();
    const sha=async bytes=>Array.from(new Uint8Array(await win.crypto.subtle.digest('SHA-256',bytes)),b=>b.toString(16).padStart(2,'0')).join('');
    for (const file of selected) {
      const path=file.webkitRelativePath ? file.webkitRelativePath.split('/').slice(1).join('/') : file.name;
      if (!path || names.has(path)) throw new Error('Two files have the same path. Choose a folder to keep their separate paths.');
      names.add(path);
      const bytes=new Uint8Array(await file.arrayBuffer());
      let binary='';
      for(let i=0;i<bytes.length;i+=32768) binary+=String.fromCharCode(...bytes.subarray(i,i+32768));
      files.push({path,base64:win.btoa(binary)});
      manifest.push({path,sha256:await sha(bytes),size:bytes.length});
    }
    manifest.sort((a,b)=>a.path<b.path?-1:a.path>b.path?1:0);
    return {files,key:'web-files-'+await sha(new TextEncoder().encode(JSON.stringify(manifest)))};
  }
  function remember(win,key,value) {
    try { if(value===null)win.sessionStorage.removeItem(key);else win.sessionStorage.setItem(key,value); } catch (_) { /* Storage settings never undo a completed API operation. */ }
  }
  function recalled(win,key) { try{return win.sessionStorage.getItem(key);}catch(_){return null;} }
  async function start(win) {
    const doc=win.document, form=doc.querySelector('[data-check-work]');
    if(!form)return;
    const status=doc.querySelector('[data-check-status]'),preview=doc.querySelector('[data-plan-preview]'),results=doc.querySelector('[data-check-results]');
    const planInput=form.querySelector('[name=plan]'),filesInput=form.querySelector('[name=files]'),folderInput=form.querySelector('[name=folder]');
    const button=form.querySelector('[type=submit]'),dispute=doc.querySelector('[data-file-dispute]');
    const close=doc.querySelector('[data-dispute-close]'),open=doc.querySelector('[data-dispute-open]'),disputeStatus=doc.querySelector('[data-dispute-status]');
    let plan=null,runId=null,artifactId=null,busy=false,planRevision=0;
    function displayHold() {
      if(!dispute)return;
      dispute.hidden=!artifactId;
      close.hidden=!recalled(win,'agent-bounties.dispute.'+artifactId);
    }
    filesInput.addEventListener('change',()=>{folderInput.value='';});
    folderInput.addEventListener('change',()=>{filesInput.value='';});
    planInput.addEventListener('change',async()=>{
      const revision=++planRevision;
      plan=null;
      preview.replaceChildren();
      try {
        const file=planInput.files[0];
        if(!file||file.size>262144)throw new Error('Choose a checking plan smaller than 256 KiB.');
        const raw=await file.text();
        const validated=await request(win,'/v1/verification/plans/validate',raw);
        const candidate=JSON.parse(raw);
        if(!safeNumbers(candidate))throw new Error('This plan contains numbers this browser cannot keep exact. Use the CLI or Python SDK with the original plan.');
        if(!validated.valid)throw new Error(validated.error.replaceAll('_',' '));
        if(revision!==planRevision)return;
        plan=candidate;
        showPlan(doc,preview,plan);
        status.textContent='Read each rule and review question. For a funded bounty, use its accepted plan unchanged.';
      } catch(error) { if(revision===planRevision)status.textContent=error.message; }
    });
    async function refresh() {
      if(!runId)return;
      const current=runId,run=await request(win,`/v1/verification/runs/${current}`);
      if(current!==runId)return;
      artifactId=run.artifact_id;
      displayHold();
      status.textContent=run.status==='completed'?'Checks finished. Read the results below.':run.status==='unavailable'?'A check could not run. This does not mean the work failed.':'Checks are queued or running. You can check again shortly.';
      if(run.report)showReport(doc,results,run.report);
    }
    form.addEventListener('submit',async event=>{
      event.preventDefault();
      if(busy)return;
      busy=true;button.disabled=true;
      try {
        const chosenPlan=plan;
        if(!chosenPlan)throw new Error('Choose the agreed checking plan first.');
        const selected=[...(folderInput.files.length?folderInput.files:filesInput.files)];
        status.textContent='Saving and checking your exact files…';
        const {files,key}=await prepareFiles(win,selected);
        const artifact=await request(win,'/v1/verification/artifacts',{files},key);
        artifactId=artifact.id;
        displayHold();
        const run=await request(win,'/v1/verification/runs',{artifact_id:artifact.id,plan:chosenPlan});
        runId=run.id;
        remember(win,'agent-bounties.check-run',runId);
        await refresh();
      } catch(error) { status.textContent=error.message; }
      finally { busy=false;button.disabled=false; }
    });
    async function changeHold(resolve) {
      if(!artifactId||open.disabled)return;
      const current=artifactId,reason=doc.querySelector('[data-dispute-reason]').value.trim();
      if(!reason){disputeStatus.textContent='Add a short reason first.';return;}
      open.disabled=true;close.disabled=true;
      try {
        const key='agent-bounties.dispute.'+current,id=recalled(win,key);
        if(resolve&&!id)throw new Error('No open dispute is saved in this browser.');
        const record=await request(win,resolve?`/v1/verification/disputes/${id}/resolve`:`/v1/verification/artifacts/${current}/disputes`,{reason});
        remember(win,key,resolve?null:record.id);
        if(current===artifactId) {
          disputeStatus.textContent=resolve?'Your dispute is closed. Other open disputes still protect these files.':'Your files will stay while your dispute is open.';
          displayHold();
        }
      } catch(error) {disputeStatus.textContent=error.message;}
      finally {open.disabled=false;close.disabled=false;}
    }
    open?.addEventListener('click',()=>changeHold(false));
    close?.addEventListener('click',()=>changeHold(true));
    doc.querySelector('[data-check-refresh]').addEventListener('click',()=>refresh().catch(e=>status.textContent=e.message));
    runId=recalled(win,'agent-bounties.check-run');
    if(runId&&/^[0-9a-f-]{36}$/i.test(runId))refresh().catch(e=>status.textContent=e.message);
  }
  return {labels,safeNumbers,showTimeline,showPlan,showReport,prepareFiles,request,start};
});
