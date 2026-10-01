(function(root,factory){const api=factory();if(typeof module==="object"&&module.exports)module.exports=api;if(root)root.AgentBountiesCreatorPush=api;})(typeof window!=="undefined"?window:globalThis,function(){
  "use strict";
  const STORAGE="ab.creator-push-device.v1";
  const CLEANUP="ab.creator-push-cleanup.v1";
  const SCOPE="/notifications/";
  const SCRIPT="/creator-push-sw.js";
  function publicKey(value,win){
    if(typeof value!=="string"||!/^[A-Za-z0-9_-]{86,88}$/.test(value))throw new Error("Push delivery is unavailable.");
    const raw=win.atob(value.replace(/-/g,"+").replace(/_/g,"/")+"=".repeat((4-value.length%4)%4));
    if(raw.length!==65||raw.charCodeAt(0)!==4)throw new Error("Push delivery is unavailable.");
    return Uint8Array.from(raw,c=>c.charCodeAt(0));
  }
  function create(win,endpoint,render){
    let account=null,version=0,configuration=null,busy=false,setupRunning=false,pendingSetup=0;
    const bounded=(promise,onLate)=>{
      let settled=false;
      if(onLate)pendingSetup++;
      return new Promise((resolve,reject)=>{
        const timeout=win.setTimeout(()=>{settled=true;reject(new Error("Browser push setup timed out. Pending setup will be cleaned up before another attempt."));},20000);
        Promise.resolve(promise).then(async value=>{
          if(settled){if(onLate)await onLate(value);return;}
          settled=true;win.clearTimeout(timeout);resolve(value);
        },error=>{if(!settled){settled=true;win.clearTimeout(timeout);reject(error);}})
          .catch(()=>{}).finally(()=>{if(onLate)pendingSetup--;});
      });
    };
    const cleanupQueue=()=>{try{const values=JSON.parse(win.localStorage.getItem(CLEANUP)||"[]");return Array.isArray(values)?values.filter(v=>typeof v?.account==="string"&&/^[0-9a-f-]{36}$/i.test(v?.id||"")):[];}catch{return [];}};
    const queueCleanup=(owner,id)=>{
      if(!/^[0-9a-f-]{36}$/i.test(id||""))return;
      const values=cleanupQueue();if(!values.some(v=>v.account===owner&&v.id===id))values.push({account:owner,id});
      win.localStorage.setItem(CLEANUP,JSON.stringify(values));
    };
    const supported=()=>Boolean(win.isSecureContext&&win.navigator?.serviceWorker&&win.PushManager&&win.Notification);
    const saved=()=>{try{return JSON.parse(win.localStorage.getItem(STORAGE)||"null");}catch{return null;}};
    const notify=(message)=>render({supported:supported(),busy,configured:configuration?.configured===true,
      enabled:!!account&&saved()?.account===account,message});
    const request=async(body)=>{
      const response=await win.fetch(endpoint,{method:body?"POST":"GET",credentials:"include",cache:"no-store",redirect:"error",headers:body?{"Content-Type":"application/json"}:undefined,body:body?JSON.stringify(body):undefined});
      if(!response.ok)throw new Error(response.status===401?"Sign in to manage notifications.":response.status===429?"Please wait before sending another test. You can send one per minute, up to five per hour.":response.status===409?"This device could not be linked. Turn notifications off, then enable them again.":"Notification settings are unavailable. Try again.");
      return response.json();
    };
    async function reconcileCleanup(owner){
      for(const device of cleanupQueue().filter(v=>v.account===owner)){
        if(account!==owner)return;
        await bounded(request({action:"unsubscribe",id:device.id}));
        win.localStorage.setItem(CLEANUP,JSON.stringify(cleanupQueue().filter(v=>v.account!==owner||v.id!==device.id)));
        const local=saved();if(local?.account===owner&&local.id===device.id)win.localStorage.removeItem(STORAGE);
      }
    }
    async function registration(){
      const value=await win.navigator.serviceWorker.getRegistration(SCOPE);
      const script=value?.active?.scriptURL||value?.waiting?.scriptURL||value?.installing?.scriptURL;
      return script&&new URL(script,win.location.origin).pathname===SCRIPT?value:null;
    }
    async function load(){
      const current=version;if(!account){configuration=null;notify("Sign in to manage browser notifications.");return;}
      if(!supported()){notify("This browser cannot receive push notifications here. Email alerts are also available.");return;}
      busy=true;notify("Checking browser notifications…");
      try{await reconcileCleanup(account);const result=await request();if(current!==version)return;configuration=result;
        const device=saved();const reg=await registration();const subscription=reg?await reg.pushManager.getSubscription():null;
        if(device?.account===account&&!subscription)win.localStorage.removeItem(STORAGE);
        notify(result.configured?"Notifications cover claims and submissions. Delivery may be delayed; check the current deadline on the site.":"Browser push delivery is not enabled yet.");
      }catch(error){if(current===version)notify(error.message);}finally{if(current===version){busy=false;notify();}}
    }
    async function enable(){
      if(busy||!account||!supported()||configuration?.configured!==true)return false;
      if(setupRunning||pendingSetup){notify("A previous browser setup is still finishing its cleanup. Please wait before trying again.");return false;}
      const current=version,owner=account;busy=true;setupRunning=true;notify("Waiting for browser permission…");let subscription,serverDeviceId;
      const currentAccount=()=>{if(current!==version)throw new Error("Account changed. Enable notifications again from your account.");};
      try{
        await reconcileCleanup(owner);currentAccount();
        // Called only by the visible Enable button; never prompt on page load.
        const permission=await win.Notification.requestPermission();if(permission!=="granted")throw new Error("Browser permission was not granted. You can keep using email alerts.");
        currentAccount();
        notify("Preparing this browser for notifications…");
        const reg=await bounded(win.navigator.serviceWorker.register(SCRIPT,{scope:SCOPE}));
        currentAccount();
        if(!reg.active)await new Promise((resolve,reject)=>{
          const worker=reg.installing||reg.waiting;if(!worker){reject(new Error("Notification worker did not start."));return;}
          const timeout=win.setTimeout(()=>reject(new Error("Notification worker did not start.")),10000);
          const check=()=>{if(worker.state==="activated"){win.clearTimeout(timeout);resolve();}else if(worker.state==="redundant"){win.clearTimeout(timeout);reject(new Error("Notification worker did not start."));}};
          worker.addEventListener("statechange",check);check();
        });
        currentAccount();
        notify("Connecting to this browser’s push service…");
        subscription=await bounded(reg.pushManager.getSubscription());
        currentAccount();
        if(subscription&&saved()?.account!==owner){if(!await bounded(subscription.unsubscribe()))throw new Error("The previous browser subscription could not be removed. Try again.");subscription=null;}
        currentAccount();
        subscription=subscription||await bounded(reg.pushManager.subscribe({userVisibleOnly:true,applicationServerKey:publicKey(configuration.public_key,win)}),late=>late.unsubscribe());
        currentAccount();
        notify("Saving this device’s notification preferences…");
        const result=await bounded(request({action:"subscribe",consent:true,subscription:subscription.toJSON()}),async late=>{
          queueCleanup(owner,late?.id);if(account===owner)await reconcileCleanup(owner);
        });
        serverDeviceId=result?.id;currentAccount();
        if(result.subscribed!==true||typeof result.id!=="string"||!/^[0-9a-f-]{36}$/i.test(result.id))throw new Error("Notification settings could not be confirmed.");
        win.localStorage.setItem(STORAGE,JSON.stringify({id:result.id,account:owner}));notify("Browser notifications are enabled for this device.");return true;
      }catch(error){
        if(subscription)await bounded(subscription.unsubscribe()).catch(()=>{});
        queueCleanup(owner,serverDeviceId);
        if(account===owner)await reconcileCleanup(owner).catch(()=>{});
        if(current===version)notify(error.message);return false;
      }
      finally{setupRunning=false;if(current===version){busy=false;notify();}}
    }
    async function disable(){
      if(!supported())return true;const device=saved();busy=true;notify("Turning off this device…");
      let localOff=false;
      try{
        const reg=await registration();const subscription=reg?await reg.pushManager.getSubscription():null;
        localOff=!subscription||await subscription.unsubscribe();
        if(!localOff)throw new Error("The browser could not unsubscribe. Try again.");
        if(reg?.getNotifications)for(const notice of await reg.getNotifications())notice.close();
        if(device?.account===account)await request({action:"unsubscribe",id:device.id});
        win.localStorage.removeItem(STORAGE);notify("Browser notifications are off for this device.");return true;
      }catch(error){if(localOff)win.localStorage.removeItem(STORAGE);notify(localOff?"This browser is unsubscribed. Server cleanup will follow; no new push can reach it.":error.message);return localOff;}
      finally{busy=false;notify();}
    }
    async function test(){
      const device=saved();
      if(busy||!account||device?.account!==account||!supported()||configuration?.configured!==true)return false;
      const current=version,requestId=win.crypto.randomUUID();busy=true;notify("Sending one test notification…");
      try{
        const reg=await registration();
        if(!reg||!await reg.pushManager.getSubscription())throw new Error("Enable notifications on this device first.");
        // Refresh the handler so an older installation understands test payloads.
        await reg.update();
        const updating=reg.installing||reg.waiting;
        if(updating&&updating.state!=="activated")await new Promise((resolve,reject)=>{
          const timeout=win.setTimeout(()=>reject(new Error("Notification update did not finish. Try again.")),10000);
          const check=()=>{if(updating.state==="activated"){win.clearTimeout(timeout);resolve();}else if(updating.state==="redundant"){win.clearTimeout(timeout);reject(new Error("Notification update did not finish. Try again."));}};
          updating.addEventListener("statechange",check);check();
        });
        if(current!==version)return false;
        const result=await request({action:"test",id:device.id,request_id:requestId,consent:true});
        if(current!==version)return false;
        if(result.request_id!==requestId||result.status!=="accepted")throw new Error("Delivery was not confirmed. No automatic resend was made. You can try a new test later.");
        notify("The push provider accepted the test. Waiting for this browser to receive it…");
        for(let attempt=0;attempt<20;attempt++){
          if(current!==version)return false;
          const notices=await reg.getNotifications({tag:`ab-test-${requestId}`});
          if(notices.some(notice=>notice.tag===`ab-test-${requestId}`)){
            notify("This browser received the test notification. No review or payment action is required.");return true;
          }
          await new Promise(resolve=>win.setTimeout(resolve,500));
        }
        notify("The push provider accepted the test, but this browser has not confirmed receipt. Check your browser notification settings.");return false;
      }catch(error){if(current===version)notify(error.message);return false;}
      finally{if(current===version){busy=false;notify();}}
    }
    return {load,enable,disable,test,setAccount(value){if(account!==value){account=value;version++;busy=false;configuration=null;}notify();}};
  }
  return {create,publicKey};
});
