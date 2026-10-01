(function(root,factory){const api=factory();if(typeof module==="object"&&module.exports)module.exports=api;if(root)root.AgentBountiesCreatorPush=api;})(typeof window!=="undefined"?window:globalThis,function(){
  "use strict";
  const STORAGE="ab.creator-push-device.v1";
  const SCOPE="/notifications/";
  const SCRIPT="/creator-push-sw.js";
  function publicKey(value,win){
    if(typeof value!=="string"||!/^[A-Za-z0-9_-]{86,88}$/.test(value))throw new Error("Push delivery is unavailable.");
    const raw=win.atob(value.replace(/-/g,"+").replace(/_/g,"/")+"=".repeat((4-value.length%4)%4));
    if(raw.length!==65||raw.charCodeAt(0)!==4)throw new Error("Push delivery is unavailable.");
    return Uint8Array.from(raw,c=>c.charCodeAt(0));
  }
  function create(win,endpoint,render){
    let account=null,version=0,configuration=null,busy=false;
    const supported=()=>Boolean(win.isSecureContext&&win.navigator?.serviceWorker&&win.PushManager&&win.Notification);
    const saved=()=>{try{return JSON.parse(win.localStorage.getItem(STORAGE)||"null");}catch{return null;}};
    const notify=(message)=>render({supported:supported(),busy,configured:configuration?.configured===true,
      enabled:!!account&&saved()?.account===account,message});
    const request=async(body)=>{
      const response=await win.fetch(endpoint,{method:body?"POST":"GET",credentials:"include",cache:"no-store",redirect:"error",headers:body?{"Content-Type":"application/json"}:undefined,body:body?JSON.stringify(body):undefined});
      if(!response.ok)throw new Error(response.status===401?"Sign in to manage notifications.":response.status===409?"This device could not be linked. Turn notifications off, then enable them again.":"Notification settings are unavailable. Try again.");
      return response.json();
    };
    async function registration(){
      const value=await win.navigator.serviceWorker.getRegistration(SCOPE);
      const script=value?.active?.scriptURL||value?.waiting?.scriptURL||value?.installing?.scriptURL;
      return script&&new URL(script,win.location.origin).pathname===SCRIPT?value:null;
    }
    async function load(){
      const current=version;if(!account){configuration=null;notify("Sign in to manage browser notifications.");return;}
      if(!supported()){notify("This browser cannot receive push notifications here. Email alerts are also available.");return;}
      busy=true;notify("Checking browser notifications…");
      try{const result=await request();if(current!==version)return;configuration=result;
        const device=saved();const reg=await registration();const subscription=reg?await reg.pushManager.getSubscription():null;
        if(device?.account===account&&!subscription)win.localStorage.removeItem(STORAGE);
        notify(result.configured?"Notifications cover claims and submissions. Delivery may be delayed; check the current deadline on the site.":"Browser push delivery is not enabled yet.");
      }catch(error){if(current===version)notify(error.message);}finally{if(current===version){busy=false;notify();}}
    }
    async function enable(){
      if(busy||!account||!supported()||configuration?.configured!==true)return false;
      const current=version,owner=account;busy=true;notify("Waiting for browser permission…");let subscription;
      try{
        // Called only by the visible Enable button; never prompt on page load.
        const permission=await win.Notification.requestPermission();if(permission!=="granted")throw new Error("Browser permission was not granted. You can keep using email alerts.");
        if(current!==version)throw new Error("Account changed. Enable notifications again from your account.");
        const reg=await win.navigator.serviceWorker.register(SCRIPT,{scope:SCOPE});
        if(!reg.active)await new Promise((resolve,reject)=>{
          const worker=reg.installing||reg.waiting;if(!worker){reject(new Error("Notification worker did not start."));return;}
          const timeout=win.setTimeout(()=>reject(new Error("Notification worker did not start.")),10000);
          const check=()=>{if(worker.state==="activated"){win.clearTimeout(timeout);resolve();}else if(worker.state==="redundant"){win.clearTimeout(timeout);reject(new Error("Notification worker did not start."));}};
          worker.addEventListener("statechange",check);check();
        });
        subscription=await reg.pushManager.getSubscription();
        if(subscription&&saved()?.account!==owner){await subscription.unsubscribe();subscription=null;}
        subscription=subscription||await reg.pushManager.subscribe({userVisibleOnly:true,applicationServerKey:publicKey(configuration.public_key,win)});
        if(current!==version)throw new Error("Account changed. Enable notifications again from your account.");
        const result=await request({action:"subscribe",consent:true,subscription:subscription.toJSON()});
        if(current!==version)throw new Error("Account changed. Enable notifications again from your account.");
        if(result.subscribed!==true||typeof result.id!=="string"||!/^[0-9a-f-]{36}$/i.test(result.id))throw new Error("Notification settings could not be confirmed.");
        win.localStorage.setItem(STORAGE,JSON.stringify({id:result.id,account:owner}));notify("Browser notifications are enabled for this device.");return true;
      }catch(error){if(subscription)await subscription.unsubscribe().catch(()=>{});notify(error.message);return false;}
      finally{if(current===version){busy=false;notify();}}
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
    return {load,enable,disable,setAccount(value){if(account!==value){account=value;version++;busy=false;configuration=null;}notify();}};
  }
  return {create,publicKey};
});
