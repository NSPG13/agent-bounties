const assert=require("node:assert/strict");
const fs=require("node:fs"),vm=require("node:vm");
const push=require("../site/creator-push.js");
const key=Buffer.concat([Buffer.from([4]),Buffer.alloc(64,1)]).toString("base64url");
const id="11111111-1111-4111-8111-111111111111";
function fixture(){
  let subscription=null,config={configured:true,public_key:key},permission="granted",device=new Map();
  const calls=[],messages=[];
  const reg={active:{scriptURL:"https://agentbounties.app/creator-push-sw.js"},getNotifications:async()=>[],pushManager:{
    getSubscription:async()=>subscription,
    subscribe:async(value)=>{calls.push(["subscribe",value]);subscription={toJSON:()=>({endpoint:"https://fcm.googleapis.com/wp/fixture",keys:{p256dh:key,auth:Buffer.alloc(16,2).toString("base64url")},expirationTime:null}),unsubscribe:async()=>{calls.push(["unsubscribe"]);subscription=null;return true;}};return subscription;}
  }};
  const win={isSecureContext:true,PushManager:function(){},location:{origin:"https://agentbounties.app"},atob:value=>Buffer.from(value,"base64").toString("binary"),setTimeout,clearTimeout,
    localStorage:{getItem:k=>device.get(k)||null,setItem:(k,v)=>device.set(k,v),removeItem:k=>device.delete(k)},
    Notification:{requestPermission:async()=>{calls.push(["permission"]);return permission;}},
    navigator:{serviceWorker:{getRegistration:async()=>reg,register:async(path,options)=>{calls.push(["register",path,options]);return reg;}}},
    fetch:async(url,options)=>{calls.push(["fetch",url,options]);return {ok:true,json:async()=>options.method==="GET"?config:JSON.parse(options.body).action==="subscribe"?{subscribed:true,id}:{subscribed:false}};}
  };
  const controller=push.create(win,"https://api.agentbounties.app/v1/site-auth/push-notifications",value=>messages.push(value));
  return {controller,win,calls,messages,permission:value=>permission=value,config:value=>config=value};
}
(async()=>{
  const f=fixture();f.controller.setAccount("account-1");await f.controller.load();
  assert.equal(f.calls.filter(v=>v[0]==="permission").length,0,"load never prompts");
  assert.equal(await f.controller.enable(),true);
  assert.equal(f.calls.filter(v=>v[0]==="permission").length,1);
  const posted=f.calls.find(v=>v[0]==="fetch"&&v[2].method==="POST");assert.equal(posted[2].credentials,"include");assert.equal(posted[2].redirect,"error");
  assert.equal(JSON.parse(posted[2].body).consent,true);assert.equal(f.messages.at(-1).enabled,true);
  assert.equal(await f.controller.disable(),true);assert.equal(f.messages.at(-1).enabled,false);assert.equal(f.calls.filter(v=>v[0]==="unsubscribe").length,1);
  const denied=fixture();denied.permission("denied");denied.controller.setAccount("account-1");await denied.controller.load();assert.equal(await denied.controller.enable(),false);assert.equal(denied.calls.filter(v=>v[0]==="subscribe").length,0);
  const off=fixture();off.config({configured:false});off.controller.setAccount("account-1");await off.controller.load();assert.equal(await off.controller.enable(),false);assert.equal(off.calls.filter(v=>v[0]==="permission").length,0);
  assert.throws(()=>push.publicKey("https://evil.example",f.win));
  const events={},notifications=[],opened=[];
  const self={location:{origin:"https://agentbounties.app"},addEventListener:(name,fn)=>events[name]=fn,skipWaiting:()=>{},registration:{showNotification:async(...value)=>notifications.push(value)},clients:{openWindow:async value=>opened.push(value)}};
  vm.runInNewContext(fs.readFileSync(require.resolve("../site/creator-push-sw.js"),"utf8"),{self,URL});
  const data={schema:"agent-bounties/creator-push-v1",id,kind:"submission",title:"untrusted override",deadline:"2026-10-01 12:00:00 UTC",url:"/participate.html?bountyContract=0x"+"11".repeat(20)+"&network=base-mainnet"};
  let pending;events.push({data:{json:()=>data},waitUntil:p=>pending=p});await pending;
  assert.equal(notifications.length,1);assert.equal(notifications[0][0],"A solution is ready for your review");assert.equal(notifications[0][1].renotify,false);assert.match(notifications[0][1].body,/2026-10-01/);
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
      if(stage==="overdue"){assert.match(latest[1].body,/deadline passed/);assert.doesNotMatch(latest[1].body,/Review by/);}
    }
  }
  for(const invalid of [{...data,stage:"invented"},{...data,stage:"overdue",deadline:null},{...data,stage:"remaining_1h",kind:"claim"}])events.push({data:{json:()=>invalid},waitUntil:()=>{throw Error("invalid deadline notification");}});
  const reminder={...data,schema:"agent-bounties/creator-push-v2",stage:"remaining_1h",expires_at:Math.floor(Date.now()/1000)+60};
  for(const invalid of [{...reminder,expires_at:Math.floor(Date.now()/1000)-1},{...reminder,expires_at:null},{...reminder,schema:"agent-bounties/creator-push-v1"},{...reminder,stage:"initial"}])events.push({data:{json:()=>invalid},waitUntil:()=>{throw Error("expired or mismatched deadline notification");}});
  const calendar=require("../site/review-deadline.js"),deadline=Math.floor(Date.now()/1000)+3600;
  const ics=calendar.calendar({protocol:"creator-open-v1",network:"base-sepolia",bounty_contract:"0x"+"11".repeat(20),round:3,verification_expires_at:deadline}).replace(/\r\n /g,"");
  assert.match(ics,/creator-open.html\?network=base-sepolia&bounty=0x[0-9a-f]{40}&entry=3/);assert.match(ics,/UID:creator-open-v1-base-sepolia-/);
  console.log("creator_push_browser=ok (permission, configuration, opt-out, payload and navigation boundaries)");
})().catch(error=>{console.error(error);process.exit(1);});
