const assert=require("node:assert/strict");
const fs=require("node:fs"),vm=require("node:vm");
const push=require("../site/creator-push.js");
const key=Buffer.concat([Buffer.from([4]),Buffer.alloc(64,1)]).toString("base64url");
const id="11111111-1111-4111-8111-111111111111";
function fixture(){
  let subscription=null,config={configured:true,public_key:key},permission="granted",device=new Map(),testStatus="accepted",received=true,testId=null;
  const calls=[],messages=[];
  const reg={active:{scriptURL:"https://agentbounties.app/creator-push-sw.js"},update:async()=>{calls.push(["update"]);},getNotifications:async()=>received&&testId?[{tag:`ab-test-${testId}`,close:()=>{}}]:[],pushManager:{
    getSubscription:async()=>subscription,
    subscribe:async(value)=>{calls.push(["subscribe",value]);subscription={toJSON:()=>({endpoint:"https://fcm.googleapis.com/wp/fixture",keys:{p256dh:key,auth:Buffer.alloc(16,2).toString("base64url")},expirationTime:null}),unsubscribe:async()=>{calls.push(["unsubscribe"]);subscription=null;return true;}};return subscription;}
  }};
  const win={isSecureContext:true,PushManager:function(){},crypto:{randomUUID:()=>id},location:{origin:"https://agentbounties.app"},atob:value=>Buffer.from(value,"base64").toString("binary"),setTimeout:fn=>setTimeout(fn,0),clearTimeout,
    localStorage:{getItem:k=>device.get(k)||null,setItem:(k,v)=>device.set(k,v),removeItem:k=>device.delete(k)},
    Notification:{requestPermission:async()=>{calls.push(["permission"]);return permission;}},
    navigator:{serviceWorker:{getRegistration:async()=>reg,register:async(path,options)=>{calls.push(["register",path,options]);return reg;}}},
    fetch:async(url,options)=>{calls.push(["fetch",url,options]);const body=options.body?JSON.parse(options.body):null;
      if(body?.action==="test")testId=body.request_id;
      return {ok:true,json:async()=>options.method==="GET"?config:body.action==="subscribe"?{subscribed:true,id}:body.action==="test"?{request_id:body.request_id,status:testStatus}:{subscribed:false}};}
  };
  const controller=push.create(win,"https://api.agentbounties.app/v1/site-auth/push-notifications",value=>messages.push(value));
  return {controller,win,calls,messages,reg,permission:value=>permission=value,config:value=>config=value,received:value=>received=value,testStatus:value=>testStatus=value};
}
(async()=>{
  const f=fixture();f.controller.setAccount("account-1");await f.controller.load();
  assert.equal(f.calls.filter(v=>v[0]==="permission").length,0,"load never prompts");
  assert.equal(await f.controller.enable(),true);
  assert.equal(f.calls.filter(v=>v[0]==="permission").length,1);
  const posted=f.calls.find(v=>v[0]==="fetch"&&v[2].method==="POST");assert.equal(posted[2].credentials,"include");assert.equal(posted[2].redirect,"error");
  assert.equal(JSON.parse(posted[2].body).consent,true);assert.equal(f.messages.at(-1).enabled,true);
  assert.equal(f.calls.filter(v=>v[0]==="fetch"&&v[2].body?.includes('"test"')).length,0,"enabling never sends a test");
  assert.equal(await f.controller.test(),true);
  assert(f.messages.some(v=>v.message?.includes("This browser received the test")));
  const testRequest=JSON.parse(f.calls.find(v=>v[0]==="fetch"&&v[2].body?.includes('"test"'))[2].body);
  assert.deepEqual(testRequest,{action:"test",id,request_id:id,consent:true});
  f.received(false);assert.equal(await f.controller.test(),false);
  assert(f.messages.some(v=>v.message?.includes("has not confirmed receipt")));
  f.testStatus("unknown");assert.equal(await f.controller.test(),false);
  assert(f.messages.some(v=>v.message?.includes("No automatic resend")));
  assert.equal(await f.controller.disable(),true);assert.equal(f.messages.at(-1).enabled,false);assert.equal(f.calls.filter(v=>v[0]==="unsubscribe").length,1);
  const requestsAfterDisable=f.calls.length;assert.equal(await f.controller.test(),false);assert.equal(f.calls.length,requestsAfterDisable);
  const denied=fixture();denied.permission("denied");denied.controller.setAccount("account-1");await denied.controller.load();assert.equal(await denied.controller.enable(),false);assert.equal(denied.calls.filter(v=>v[0]==="subscribe").length,0);
  const off=fixture();off.config({configured:false});off.controller.setAccount("account-1");await off.controller.load();assert.equal(await off.controller.enable(),false);assert.equal(off.calls.filter(v=>v[0]==="permission").length,0);
  const stalled=fixture();stalled.controller.setAccount("account-1");await stalled.controller.load();
  let finishSubscribe;stalled.reg.pushManager.subscribe=()=>new Promise(resolve=>finishSubscribe=resolve);
  assert.equal(await stalled.controller.enable(),false);
  assert.equal(stalled.messages.at(-1).busy,false);
  assert(stalled.messages.some(value=>value.message?.includes("setup timed out")));
  assert.equal(await stalled.controller.enable(),false,"retry waits for the pending native subscription to be cleaned up");
  let lateUnsubscribed=0;
  finishSubscribe({toJSON:()=>({}),unsubscribe:async()=>{lateUnsubscribed++;return true;}});await new Promise(resolve=>setImmediate(resolve));
  assert.equal(lateUnsubscribed,1,"a native subscription arriving after timeout is explicitly unsubscribed");
  assert.equal(stalled.calls.filter(v=>v[0]==="fetch"&&v[2].method==="POST").length,0,"late native subscription cannot save after timeout");
  const latePost=fixture();latePost.controller.setAccount("account-1");await latePost.controller.load();
  const postFetch=latePost.win.fetch;let finishPost,finishCleanup;
  latePost.win.fetch=async(url,options)=>{
    const body=options.body&&JSON.parse(options.body);
    if(body?.action==="subscribe")return new Promise(resolve=>finishPost=resolve);
    if(body?.action==="unsubscribe"){latePost.calls.push(["cleanup",body]);return new Promise(resolve=>finishCleanup=resolve);}
    return postFetch(url,options);
  };
  assert.equal(await latePost.controller.enable(),false);
  assert.equal(latePost.calls.filter(v=>v[0]==="unsubscribe").length,1,"POST timeout revokes the native subscription immediately");
  assert.equal(await latePost.controller.enable(),false,"an unresolved save cannot race another setup");
  finishPost({ok:true,json:async()=>({subscribed:true,id})});await new Promise(resolve=>setImmediate(resolve));
  assert.deepEqual(latePost.calls.find(v=>v[0]==="cleanup")[1],{action:"unsubscribe",id});
  assert.equal(await latePost.controller.enable(),false,"new setup waits for late server cleanup too");
  await new Promise(resolve=>setTimeout(resolve,10));
  assert.equal(await latePost.controller.enable(),false,"a timed-out cleanup is still a barrier until its original request settles");
  assert.equal(latePost.calls.filter(v=>v[0]==="cleanup").length,1,"retry cannot race an older unsubscribe request");
  finishCleanup({ok:true,json:async()=>({subscribed:false})});await new Promise(resolve=>setImmediate(resolve));
  assert.deepEqual(JSON.parse(latePost.win.localStorage.getItem("ab.creator-push-cleanup.v1")),[{account:"account-1",id}],"uncertain cleanup keeps a reconciliation record");
  assert.equal(latePost.messages.at(-1).enabled,false);
  latePost.win.fetch=postFetch;assert.equal(await latePost.controller.enable(),true,"setup recovers after confirmed cleanup");
  assert.deepEqual(JSON.parse(latePost.win.localStorage.getItem("ab.creator-push-cleanup.v1")),[]);
  const switched=fixture();switched.controller.setAccount("account-1");await switched.controller.load();
  const switchedFetch=switched.win.fetch;let finishOldAccount;
  switched.win.fetch=async(url,options)=>options.body&&JSON.parse(options.body).action==="subscribe"?new Promise(resolve=>finishOldAccount=resolve):switchedFetch(url,options);
  assert.equal(await switched.controller.enable(),false);
  switched.controller.setAccount("account-2");await switched.controller.load();
  finishOldAccount({ok:true,json:async()=>({subscribed:true,id})});await new Promise(resolve=>setImmediate(resolve));
  assert.equal(switched.calls.filter(v=>v[0]==="fetch"&&v[2].body?.includes('"unsubscribe"')).length,0,"old account cleanup is not sent under a new account session");
  assert.deepEqual(JSON.parse(switched.win.localStorage.getItem("ab.creator-push-cleanup.v1")),[{account:"account-1",id}]);
  const resumed=push.create(switched.win,"https://api.agentbounties.app/v1/site-auth/push-notifications",()=>{});
  resumed.setAccount("account-1");await resumed.load();
  assert.equal(switched.calls.filter(v=>v[0]==="fetch"&&v[2].body?.includes('"unsubscribe"')).length,1,"cleanup survives controller reload and resumes only for the owning account");
  assert.deepEqual(JSON.parse(switched.win.localStorage.getItem("ab.creator-push-cleanup.v1")),[]);
  assert.throws(()=>push.publicKey("https://evil.example",f.win));
  const events={},notifications=[],opened=[];
  const self={location:{origin:"https://agentbounties.app"},addEventListener:(name,fn)=>events[name]=fn,skipWaiting:()=>{},registration:{showNotification:async(...value)=>notifications.push(value)},clients:{openWindow:async value=>opened.push(value)}};
  vm.runInNewContext(fs.readFileSync(require.resolve("../site/creator-push-sw.js"),"utf8"),{self,URL});
  const data={schema:"agent-bounties/creator-push-v1",id,kind:"submission",title:"untrusted override",deadline:"2026-10-01 12:00:00 UTC",url:"/participate.html?bountyContract=0x"+"11".repeat(20)+"&network=base-mainnet"};
  let pending;events.push({data:{json:()=>data},waitUntil:p=>pending=p});await pending;
  assert.equal(notifications.length,1);assert.equal(notifications[0][0],"A solution is ready for your review");assert.equal(notifications[0][1].renotify,false);assert.match(notifications[0][1].body,/2026-10-01/);
  assert.equal(notifications[0][1].tag,`ab-${id}`);
  events.push({data:{json:()=>({...data,url:"https://evil.example/participate.html"})},waitUntil:()=>{throw Error("foreign notification");}});
  events.notificationclick({notification:{close:()=>{},data:{url:"https://evil.example/"}},waitUntil:()=>{throw Error("foreign navigation");}});assert.equal(opened.length,0);
  events.notificationclick({notification:{close:()=>{},data:notifications[0][1].data},waitUntil:p=>pending=p});await pending;assert.match(opened[0],/^https:\/\/agentbounties.app\/participate.html\?/);
  const creator={...data,kind:"creator_open_submission",url:"/creator-open.html?network=base-mainnet&bounty=0x"+"11".repeat(20)+"&entry=3"};
  events.push({data:{json:()=>creator},waitUntil:p=>pending=p});await pending;assert.equal(notifications.length,2);
  events.notificationclick({notification:{close:()=>{},data:notifications[1][1].data},waitUntil:p=>pending=p});await pending;assert.match(opened[1],/creator-open.html\?network=base-mainnet&bounty=0x[0-9a-f]{40}&entry=3$/);
  for(const invalid of [{...creator,kind:"claim"},{...creator,url:creator.url.replace("entry=3","entry=-1")},{...creator,url:creator.url.replace("base-mainnet","base-sepolia")}])events.push({data:{json:()=>invalid},waitUntil:()=>{throw Error("invalid creator notification");}});
  for(const stage of ["remaining_24h","remaining_6h","remaining_1h","overdue"]){
    for(const source of [data,creator]){
      events.push({data:{json:()=>({...source,schema:"agent-bounties/creator-push-v2",stage,expires_at:Math.floor(Date.now()/1000)+60})},waitUntil:p=>pending=p});await pending;
      const latest=notifications.at(-1);
      assert.equal(latest[0],stage==="overdue"?"A review deadline was missed":"Your review deadline is approaching");
      assert.equal(latest[1].tag,`ab-${id}-${stage}`,"each deadline stage gets a distinct alert even when the initial notice remains visible");
      assert.equal(latest[1].renotify,false,"duplicate delivery of the same stage does not re-alert");
      if(stage==="overdue"){assert.match(latest[1].body,/deadline passed/);assert.doesNotMatch(latest[1].body,/Review by/);}
    }
  }
  for(const invalid of [{...data,stage:"invented"},{...data,stage:"overdue",deadline:null},{...data,stage:"remaining_1h",kind:"claim"}])events.push({data:{json:()=>invalid},waitUntil:()=>{throw Error("invalid deadline notification");}});
  const reminder={...data,schema:"agent-bounties/creator-push-v2",stage:"remaining_1h",expires_at:Math.floor(Date.now()/1000)+60};
  const tray=new Map();
  for(const notice of notifications)tray.set(notice[1].tag,notice);
  assert.equal(tray.size,5,"initial plus four distinct deadline stages survive replacement in the notification tray");
  for(let duplicate=0;duplicate<2;duplicate++){events.push({data:{json:()=>reminder},waitUntil:p=>pending=p});await pending;const notice=notifications.at(-1);tray.set(notice[1].tag,notice);}
  assert.equal(tray.size,5,"duplicate same-stage delivery reuses its tag");
  for(const invalid of [{...reminder,expires_at:Math.floor(Date.now()/1000)-1},{...reminder,expires_at:null},{...reminder,schema:"agent-bounties/creator-push-v1"},{...reminder,stage:"initial"}])events.push({data:{json:()=>invalid},waitUntil:()=>{throw Error("expired or mismatched deadline notification");}});
  const test={schema:"agent-bounties/push-test-v1",id,expires_at:Math.floor(Date.now()/1000)+60,title:"Review required",url:"https://evil.example"};
  events.push({data:{json:()=>test},waitUntil:p=>pending=p});await pending;
  assert.equal(notifications.at(-1)[0],"Agent Bounties notification test");
  assert.match(notifications.at(-1)[1].body,/No review or payment action is required/);
  events.notificationclick({notification:{close:()=>{},data:notifications.at(-1)[1].data},waitUntil:p=>pending=p});await pending;
  assert.equal(opened.at(-1),"https://agentbounties.app/#account");
  for(const invalid of [{...test,id:"bad"},{...test,expires_at:null},{...test,expires_at:0},{...test,expires_at:Math.floor(Date.now()/1000)+3600}])events.push({data:{json:()=>invalid},waitUntil:()=>{throw Error("invalid delivery test");}});
  events.notificationclick({notification:{close:()=>{},data:{test:true,url:data.url}},waitUntil:()=>{throw Error("test navigated to a bounty");}});
  const calendar=require("../site/review-deadline.js"),deadline=Math.floor(Date.now()/1000)+3600;
  const ics=calendar.calendar({protocol:"creator-open-v1",network:"base-sepolia",bounty_contract:"0x"+"11".repeat(20),round:3,verification_expires_at:deadline}).replace(/\r\n /g,"");
  assert.match(ics,/creator-open.html\?network=base-sepolia&bounty=0x[0-9a-f]{40}&entry=3/);assert.match(ics,/UID:creator-open-v1-base-sepolia-/);
  console.log("creator_push_browser=ok (permission, configuration, opt-out, payload and navigation boundaries)");
})().catch(error=>{console.error(error);process.exit(1);});
