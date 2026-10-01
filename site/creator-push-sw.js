/* This worker only handles explicitly requested push. It never caches pages,
   intercepts requests, signs wallet messages, or performs a bounty action. */
"use strict";
self.addEventListener("install",()=>self.skipWaiting());
function destination(value){
  try{
    const url=new URL(value,self.location.origin);
    if(url.origin!==self.location.origin||url.username||url.password)return null;
    if(url.pathname==="/creator-open.html"){
      const contract=url.searchParams.get("bounty"),entry=url.searchParams.get("entry");
      if(!/^0x[0-9a-f]{40}$/i.test(contract||"")||url.searchParams.get("network")!=="base-mainnet"||!/^\d+$/.test(entry||"")||!Number.isSafeInteger(Number(entry))||Number(entry)<=0)return null;
      return `${self.location.origin}/creator-open.html?network=base-mainnet&bounty=${contract}&entry=${Number(entry)}`;
    }
    if(url.pathname!=="/participate.html")return null;
    const contract=url.searchParams.get("bountyContract");
    if(!/^0x[0-9a-f]{40}$/i.test(contract||"")||url.searchParams.get("network")!=="base-mainnet")return null;
    return `${self.location.origin}/participate.html?bountyContract=${contract}&network=base-mainnet&role=verifier`;
  }catch{return null;}
}
self.addEventListener("push",event=>{
  let data;try{data=event.data?.json();}catch{return;}
  if(data?.schema==="agent-bounties/push-test-v1"){
    if(!/^[0-9a-f-]{36}$/i.test(data.id||"")||!Number.isSafeInteger(data.expires_at)||data.expires_at*1000<=Date.now()||data.expires_at*1000>Date.now()+300000)return;
    event.waitUntil(self.registration.showNotification("Agent Bounties notification test",{
      body:"This device received a test notification. No review or payment action is required.",
      tag:`ab-test-${data.id}`,renotify:false,data:{test:true,url:`${self.location.origin}/#account`}
    }));return;
  }
  const url=destination(data?.url);
  if(!["agent-bounties/creator-push-v1","agent-bounties/creator-push-v2"].includes(data?.schema)||!url||!/^[0-9a-f-]{36}$/i.test(data.id||"")||!["claim","submission","creator_open_submission"].includes(data.kind))return;
  if((data.kind==="creator_open_submission")!==(new URL(url).pathname==="/creator-open.html"))return;
  const stage=data.stage??"initial";
  if(!["initial","remaining_24h","remaining_6h","remaining_1h","overdue"].includes(stage)||(data.kind==="claim"&&stage!=="initial"))return;
  if((data.schema==="agent-bounties/creator-push-v1")!==(stage==="initial"))return;
  if(stage!=="initial"&&(!Number.isSafeInteger(data.expires_at)||data.expires_at*1000<=Date.now()))return;
  const title=stage==="overdue"?"A review deadline was missed":stage!=="initial"?"Your review deadline is approaching":data.kind==="claim"?"Work has started on your bounty":"A solution is ready for your review";
  const deadline=typeof data.deadline==="string"&&/^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2} UTC$/.test(data.deadline)?data.deadline:null;
  if(stage!=="initial"&&!deadline)return;
  const body=stage==="overdue"?`Review deadline passed: ${deadline}. Open the site for current status and recovery steps.`:deadline?`${data.kind==="claim"?"Claim expires":"Review by"} ${deadline}. Open the site for current status.`:"Open Agent Bounties to check the latest work and deadline.";
  event.waitUntil(self.registration.showNotification(title,{body,tag:`ab-${data.id}`,renotify:false,data:{url}}));
});
self.addEventListener("notificationclick",event=>{
  event.notification.close();
  const data=event.notification.data;
  const url=data?.test===true?(data.url===`${self.location.origin}/#account`?data.url:null):destination(data?.url);
  if(url)event.waitUntil(self.clients.openWindow(url));
});
