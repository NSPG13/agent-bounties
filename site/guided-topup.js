/* A saved operation, one next action, and no automatic purchase confirmations. */
(() => {
  "use strict";
  const funding=window.AgentBountiesWalletFunding, $=selector=>document.querySelector(selector);
  const state={operation:null,client:null,wallet:null,required:null,readiness:null,attempt:null,options:null,checkout:null,busy:false,action:"refresh",metamask:null,country:""};
  const mobile=()=>/Android|iPhone|iPad/i.test(navigator.userAgent);
  const text=(selector,value)=>{const node=$(selector);if(node)node.textContent=value;};
  const show=(selector,visible)=>{const node=$(selector);if(node)node.hidden=!visible;};
  const units=value=>window.AgentBountiesFundingReadiness.formatUnits(BigInt(value));
  function remembered(key){try{return localStorage.getItem(key);}catch(_){return null;}}
  function remember(key,value){try{localStorage.setItem(key,value);}catch(_){}}
  function announce(message){text("[data-guided-message]",message);}
  function primary(label,action,enabled=true){state.action=action;state.actionEnabled=enabled;const button=$("[data-guided-next]");button.textContent=label;button.disabled=!enabled||state.busy;}
  function diagnostic(code){
    // Only fixed event names; never order IDs, addresses, URLs, card or auth data.
    const names={quote_ready:"onramp_viewed",purchase_pending:`onramp_${state.attempt?.provider}_started`,recovered:"onramp_returned"};
    state.diagnostic={step:state.action,provider:state.attempt?.provider||$("[data-guided-provider]")?.value||null,outcome:code};
    if(names[code])window.agentBountiesAnalytics?.track(names[code]);
  }
  function input(provider=$("[data-guided-provider]").value){return {wallet:state.wallet,required_usdc_units:String(state.required),country:$("[data-guided-country]").value.trim().toUpperCase(),subdivision:$("[data-guided-state]").value.trim().toUpperCase(),payment_currency:$("[data-guided-currency]").value,payment_method:$("[data-guided-method]").value,provider,metamask_address:state.metamask,analytics_disabled:new URLSearchParams(window.location.search).get("analytics")==="off"};}
  function snapshot(){return {operation_id:state.operation,wallet:state.wallet,network:"base-mainnet",readiness:state.readiness,attempt:state.attempt,options:state.options,step:funding.step(state.readiness,state.attempt),diagnostic:state.diagnostic||null,next_action:$("[data-guided-next]")?.textContent||"Open saved bounty review",bounty_funded:false};}
  function guide(){
    const provider=$("[data-guided-provider]").value, list=$("[data-guided-guide-steps]");list.replaceChildren();
    const steps=provider==="coinbase"?[
      ["Sign in to Coinbase","Use your existing email or social account. Recover it through Coinbase if needed.","Sign in"],
      ["Check the prepared purchase","The checkout receives this wallet, USDC and Base. Check the quote, fees and destination.","USDC · Base · 0x1234…abcd"],
      ["Confirm with Coinbase","Complete any identity or bank check yourself. Return here after the provider confirms delivery.","Confirm"]
    ]:provider==="moonpay"?[
      ["Sign in or create your MoonPay account","Enter credentials directly in MoonPay; identity verification may be required.","Continue"],
      ["Review the quote","Check USDC on Base, this exact wallet, the purchase total and fees. A provider minimum may leave extra USDC in your wallet.","USDC · Base · Review order"],
      ["Complete payment and return","Approve any bank authentication yourself. If declined, check the same order with support before retrying.","Continue"]
    ]:[
      [mobile()?"Open MetaMask on this phone":"Use a browser connected to MetaMask",mobile()?"Use the native wallet handoff. Return to the saved link when finished.":"The in-app browser may not expose your extension. Open the saved handoff in your normal wallet browser, then connect the same address.","Connect wallet"],
      ["Choose the purchase","Open Buy, select your country and payment method, then USDC on Base. Check that the connected address matches this wallet.","Buy · USDC · Base"],
      ["Review the Transak quote","Continue only after MetaMask finishes authenticating. A disabled button means the checkout has not started.","Continue with Transak"]
    ];
    for(const [title,copy,label] of steps){const li=document.createElement("li"),strong=document.createElement("strong"),p=document.createElement("p"),example=document.createElement("span"),b=document.createElement("b");strong.textContent=title;p.textContent=copy;example.className="topup-example";example.setAttribute("aria-label","Illustrative interface");b.textContent=label;example.append(b);li.append(strong,p,example);list.append(li);}
  }
  function feeSummary(quote){
    if(Array.isArray(quote.fees))return quote.fees.map(fee=>`${String(fee.type||"Fee").replace("FEE_TYPE_","").toLowerCase()}: ${fee.amount} ${fee.currency||quote.payment_currency||""}`).join(" · ");
    if(quote.fees&&typeof quote.fees==="object")return Object.entries(quote.fees).filter(([,amount])=>amount!=null).map(([name,amount])=>`${name}: ${amount} ${quote.payment_currency||""}`).join(" · ");
    return "The provider shows fees before payment.";
  }
  async function detectMetaMask(){
    const candidates=Array.isArray(window.ethereum?.providers)?window.ethereum.providers:[window.ethereum];
    const provider=candidates.find(p=>p?.isMetaMask&&typeof p.request==="function");
    if(!provider)return null;
    const accounts=await provider.request({method:"eth_accounts"}).catch(()=>[]);
    return accounts[0]||null;
  }
  function render(){
    const current=funding.step(state.readiness,state.attempt);
    for(const node of document.querySelectorAll("[data-topup-progress]")){if(Number(node.dataset.topupProgress)===current)node.setAttribute("aria-current","step");else node.removeAttribute("aria-current");}
    show("[data-guided-wallet]",Boolean(state.wallet));text("[data-guided-address]",state.wallet||"");
    text("[data-guided-required]",state.required?`${units(state.required)} USDC`:"Saved bounty budget");
    text("[data-guided-shortfall]",state.readiness?.usdc_shortfall_units!=null?`${units(state.readiness.usdc_shortfall_units)} USDC`:"Recheck balance");
    text("[data-guided-gas]",state.readiness?.gas_sponsorship?.message||"Gas sponsorship is checked for the exact bounty on return. Buying USDC does not authorize funding.");
    const pending=funding.unresolved(state.attempt), quoted=state.attempt?.status==="prepared"&&Boolean(state.checkout);
    show("[data-guided-wallet]",Boolean(state.wallet)&&!quoted);
    show("[data-guided-choices]",!pending&&current!==3);show("[data-guided-quote]",quoted);show("[data-guided-recovery]",pending&&!quoted);
    show("[data-guided-handoff]",!pending&&current!==3&&state.options?.recommended==="handoff");
    text("[data-guided-handoff-copy]",mobile()?"Open this saved operation inside your wallet app. Use the same wallet; native handoff keeps pairing on this phone.":"This browser has no matching MetaMask connection. Copy the saved handoff into your wallet browser, or connect a phone wallet by scanning its live pairing code.");
    if(pending && state.attempt.operation_id !== state.operation){
      show("[data-guided-recovery]",false);text("[data-guided-title]","Resume the saved purchase");announce("Another saved bounty has an unresolved purchase for this wallet. Check that purchase before starting another.");primary("Open that saved operation","other_operation");
    } else if(current===3){text("[data-guided-title]","Your wallet is ready");announce("The required USDC is visible in this wallet. Your bounty still needs your funding confirmation.");primary("Return to funding review","return");}
    else if(quoted){
      const quote=state.attempt.quote||{};text("[data-guided-title]","Review before opening checkout");announce("Check the purchase total and destination below. You confirm payment with the provider.");
      text("[data-guided-pay]",quote.payment_total!=null?`${quote.payment_total} ${quote.payment_currency}`:"Shown by MetaMask before payment");
      text("[data-guided-receive]",quote.received_usdc!=null?`${quote.received_usdc} USDC`:"Review the live provider quote");text("[data-guided-excess]",quote.excess_usdc!=null?`${quote.excess_usdc} USDC`:"Shown with the quote");
      text("[data-guided-fees]",feeSummary(quote));text("[data-guided-expiry]",`Quote prepared for Base and ${state.wallet}. Open before ${new Date(state.attempt.expires_at).toLocaleTimeString()}; final provider terms apply.`);
      text("[data-guided-signin]",state.options?.providers?.find(p=>p.id===state.attempt.provider)?.requirements||"The provider may require its own account and identity verification.");primary(`Open ${state.attempt.provider==="coinbase"?"Coinbase":state.attempt.provider==="moonpay"?"MoonPay":"MetaMask"} checkout`,"open");
    }else if(pending){text("[data-guided-title]","Continue your existing purchase");announce(["prepared","preparing"].includes(state.attempt.status)?"An unopened quote is saved. It can be replaced after confirming it was never opened.":"A purchase may be pending. Check its order before starting another, including with a different provider.");
      const support=$("[data-guided-support]");support.href=state.attempt.provider==="moonpay"?"https://support.moonpay.com/":state.attempt.provider==="coinbase"?"https://help.coinbase.com/en/coinbase/trading-and-funding/coinbase-pay/using-onramp":"https://support.metamask.io/";primary(["prepared","preparing"].includes(state.attempt.status)?"Replace unopened quote":"Check purchase and wallet",["prepared","preparing"].includes(state.attempt.status)?"cancel_unopened":"refresh");
    }else {text("[data-guided-title]","Add the missing USDC");announce(state.options?.recommended==="handoff"?"No compatible purchase route is ready in this browser. Keep the same wallet and saved operation when you switch devices.":"We’ll prefill the purchase. You review its fees before paying.");primary(state.options?.recommended==="handoff"?"Return to wallet review":state.options?"Get provider quote":"Check purchase options",state.options?.recommended==="handoff"?"return":state.options?"prepare":"options");}
    const recovery=state.attempt?.quote?.recovery_code;
    if(recovery){announce(funding.recovery(recovery));show("[data-guided-recovery]",true);const support=$("[data-guided-support]");try{const url=new URL(state.attempt.quote.support_url);if(["buy.moonpay.com","support.moonpay.com","help.coinbase.com"].includes(url.hostname)&&url.protocol==="https:"&&!url.username&&!url.password&&!url.port)support.href=url.href;}catch(_){}support.textContent=`Check this provider order${state.attempt.quote.order_reference?` · ${state.attempt.quote.order_reference}`:""} ↗`;}
    $("[data-guided-card]").setAttribute("aria-busy",String(state.busy));guide();
  }
  async function refresh(autoReturn=false){
    const wasShort=!state.readiness||(state.readiness.usdc_shortfall_units!=null&&state.readiness.usdc_shortfall_units!=="0");
    const [readiness,status]=await Promise.all([state.client.readiness(state.wallet,state.required),state.client.status()]);
    state.readiness=readiness;if(readiness.required_usdc_units)state.required=BigInt(readiness.required_usdc_units);state.attempt=status.attempt||null;
    if(state.attempt?.status!=="prepared")state.checkout=null;
    render();
    if(autoReturn&&wasShort&&readiness.usdc_shortfall_units==="0"){diagnostic("recovered");window.location.assign(funding.returnUrl(window,state.operation));}
    return snapshot();
  }
  async function options(overrides={}){
    if(overrides.country)$("[data-guided-country]").value=overrides.country;
    if(overrides.subdivision)$("[data-guided-state]").value=overrides.subdivision;
    const selected=input();if(!/^[A-Z]{2}$/.test(selected.country))throw new Error("Enter your two-letter country code so the provider can check availability.");
    state.metamask=await detectMetaMask();state.options=await state.client.options(input());
    const selector=$("[data-guided-provider]");for(const option of selector.options){option.disabled=!state.options.providers?.find(p=>p.id===option.value)?.available;}
    if(state.options.enabled!==true)throw new Error(state.options.next_action||"Purchase options are unavailable. Keep this saved review.");
    if(state.options.recommended!=="handoff")selector.value=state.options.recommended;
    paymentChoices();render();return snapshot();
  }
  function paymentChoices(){
    const provider=state.options?.providers?.find(p=>p.id===$("[data-guided-provider]").value);
    const currencies=provider?.options?.payment_currencies||[{id:"USD"}],methods=provider?.options?.payment_methods||[{id:"CARD"}];
    for(const [selector,values] of [["[data-guided-currency]",currencies],["[data-guided-method]",methods]]){const select=$(selector),previous=select.value;select.replaceChildren();for(const value of values){const option=document.createElement("option");option.value=value.id;option.textContent=value.id;select.append(option);}if([...select.options].some(option=>option.value===previous))select.value=previous;}
    text("[data-guided-requirements]",provider?.requirements||"Choose a compatible provider after checking availability.");guide();
  }
  async function prepare(overrides={}){
    if(funding.unresolved(state.attempt))throw new Error("Resolve the saved purchase before requesting another quote.");
    if(overrides.provider)$("[data-guided-provider]").value=overrides.provider;
    for(const [key,selector] of [["payment_currency","[data-guided-currency]"],["payment_method","[data-guided-method]"]])if(overrides[key])$(selector).value=overrides[key];
    const before=state.wallet;state.metamask=await detectMetaMask();
    if($("[data-guided-provider]").value==="metamask"&&!funding.sameWallet(state.metamask,before))throw new Error(funding.recovery("wallet_mismatch"));
    const result=await state.client.prepare(input());state.attempt=result.attempt||null;state.checkout=result.checkout_url?funding.checkoutUrl(result.checkout_url,state.attempt.provider):null;
    render();if(result.status==="unavailable")announce(result.next_action||funding.recovery(result.code));else diagnostic("quote_ready");return snapshot();
  }
  async function openCheckout(){
    if(!state.checkout||state.attempt?.status!=="prepared")throw new Error("Recheck the saved quote before continuing.");
    if(Date.parse(state.attempt.expires_at)<=Date.now()){state.checkout=null;render();throw new Error(funding.recovery("quote_expired"));}
    const popup=window.open("about:blank","agent-bounties-wallet-topup");if(!popup)throw new Error(funding.recovery("navigation_blocked"));
    const url=state.checkout;
    try{await state.client.action({attempt_id:state.attempt.id,action:"open"});state.attempt.status="pending";state.checkout=null;popup.opener=null;popup.location.replace(url);diagnostic("purchase_pending");render();}
    catch(error){popup.close();await refresh().catch(()=>{});throw error;}
  }
  async function run(action){if(state.busy)return;state.busy=true;$("[data-guided-next]").disabled=true;try{await action();}catch(error){announce(error.message);diagnostic("failure");if(error.status===401)primary("Sign in to restore this bounty","signin");}finally{state.busy=false;$("[data-guided-card]").setAttribute("aria-busy","false");$("[data-guided-next]").disabled=state.actionEnabled===false;}}
  function registerTools(){
    const context=document.modelContext;if(!context?.registerTool)return;
    const lifecycle=new AbortController();window.addEventListener("pagehide",()=>lifecycle.abort(),{once:true});
    const tools=[
      ["agent_bounties_get_topup_status","Read the saved purchase and wallet state; never infer bounty funding from a provider receipt.",{},async()=>refresh()],
      ["agent_bounties_get_topup_options","Check compatible purchase providers for the saved wallet. This does not open checkout.",{country:{type:"string",pattern:"^[A-Z]{2}$"},subdivision:{type:"string"}},options],
      ["agent_bounties_prepare_topup","Stage a quote for the saved wallet and operation. The person reviews fees and opens checkout; this cannot buy crypto.",{provider:{type:"string",enum:["coinbase","moonpay","metamask"]},payment_currency:{type:"string"},payment_method:{type:"string"}},prepare]
    ];
    for(const [name,description,properties,execute] of tools){void Promise.resolve(context.registerTool({name,title:name.replaceAll("_"," "),description,inputSchema:{type:"object",properties,additionalProperties:false},annotations:{readOnlyHint:name.endsWith("status"),untrustedContentHint:true},async execute(input={}){if(!state.client||!state.wallet)throw new Error("Restore the saved account wallet first.");if(Object.keys(input).some(k=>!Object.hasOwn(properties,k)))throw new Error("Unsupported top-up field.");return execute(input);}},{signal:lifecycle.signal})).catch(()=>{});}
  }
  async function boot(){
    const requested=new URLSearchParams(window.location.search), key=`agent-bounties:guided-topup:${requested.get("operation_id")}`;
    const recovering=requested.get("guided")==="1"||remembered(key)==="1";
    let enabled=false;
    try{const api=window.AgentBountiesWorkflow.apiBase(window.location);const response=await fetch(`${api}/v1/wallet-funding/capabilities`,{cache:"no-store",credentials:"omit"});enabled=response.ok&&(await response.json()).guided_topup===true;}catch(_){}
    if(!enabled&&!recovering)return false;
    remember(key,"1");
    show("[data-legacy-topup]",false);show("[data-guided-topup]",true);
    const params=new URLSearchParams(window.location.search);state.operation=params.get("operation_id");
    try{
      state.client=funding.create(window,state.operation);const back=funding.returnUrl(window,state.operation);$("[data-guided-return]").href=back;
      const api=window.AgentBountiesWorkflow.apiBase(window.location);
      const accountResponse=await fetch(`${api}/v1/site-auth/account`,{credentials:"include",cache:"no-store"});if(!accountResponse.ok)throw Object.assign(new Error("Sign in here to restore this saved bounty."),{status:401});
      const account=await accountResponse.json();state.wallet=params.get("wallet")||account.wallets?.[0]?.address;
      if(!funding.ADDRESS.test(state.wallet||""))throw new Error("Choose your account wallet in the saved bounty review.");
      state.required=window.AgentBountiesFundingReadiness.parseUsdc(params.get("amount")||"");
      state.country=remembered("agent-bounties:topup-country")||"";$("[data-guided-country]").value=state.country;show("[data-guided-state-label]",state.country==="US");
      registerTools();await refresh(true);
      // Carry an earlier unresolved MoonPay order into the provider-neutral guard.
      const legacy=JSON.parse(remembered("agent-bounties:onramp-attempts:v1")||"{}");
      if(!state.attempt&&Object.keys(legacy).some(key=>funding.sameWallet(key.split(":")[0],state.wallet))){
        const imported=await state.client.prepare({...input("moonpay"),country:state.country||"ZZ",legacy_pending:true});state.attempt=imported.attempt;render();
      }
    }catch(error){announce(error.message);primary(error.status===401?"Sign in to restore this bounty":"Return to saved review",error.status===401?"signin":"return");}
    $("[data-guided-next]").addEventListener("click",event=>{if(!event.isTrusted)return;void run(async()=>{
      if(state.action==="other_operation")window.location.assign(funding.returnUrl(window,state.attempt.operation_id));
      else if(state.action==="return")window.location.assign(funding.returnUrl(window,state.operation));
      else if(state.action==="signin"){window.AgentBountiesPostingAuth.begin(window,funding.returnUrl(window,state.operation));}
      else if(state.action==="options")await options();else if(state.action==="prepare")await prepare();else if(state.action==="open")await openCheckout();
      else if(state.action==="cancel_unopened"){await state.client.action({attempt_id:state.attempt.id,action:"cancel_unopened"});state.checkout=null;await refresh();}
      else await refresh();
    });});
    $("[data-guided-country]").addEventListener("input",()=>{state.options=null;state.country=$("[data-guided-country]").value.trim().toUpperCase();show("[data-guided-state-label]",state.country==="US");primary("Check purchase options","options");if(/^[A-Z]{2}$/.test(state.country))remember("agent-bounties:topup-country",state.country);});
    $("[data-guided-state]").addEventListener("input",()=>{state.options=null;primary("Check purchase options","options");});$("[data-guided-provider]").addEventListener("change",paymentChoices);
    $("[data-guided-clear]").addEventListener("click",event=>{if(!event.isTrusted||!$("[data-guided-resolved]").checked)return;void run(async()=>{await state.client.action({attempt_id:state.attempt.id,action:"confirm_resolved",provider_resolution_confirmed:true});$("[data-guided-resolved]").checked=false;await refresh();});});
    $("[data-guided-copy]").addEventListener("click",()=>void run(async()=>{await navigator.clipboard.writeText(window.location.href);announce("Saved handoff copied. Open it in your wallet browser; your bounty operation and wallet are preserved.");}));
    $("[data-guided-phone]").addEventListener("click",()=>void run(async()=>{const phone=window.AgentBountiesPhoneWallet;if(!phone?.state().available)throw new Error("Phone pairing is unavailable here. Copy the saved handoff into your wallet browser.");const accounts=await phone.provider.request({method:"eth_requestAccounts"});if(!funding.sameWallet(accounts?.[0],state.wallet))throw new Error(funding.recovery("wallet_mismatch"));announce("The same wallet is connected. Return to the saved bounty review when its USDC arrives.");}));
    window.addEventListener("focus",()=>{if(state.client&&state.wallet&&!state.busy)void run(()=>refresh(true));});
    const timer=setInterval(()=>{if(!document.hidden&&state.client&&state.wallet&&!state.busy&&funding.unresolved(state.attempt))void run(()=>refresh(true));},15000);window.addEventListener("pagehide",()=>clearInterval(timer),{once:true});
    window.AgentBountiesGuidedTopup={snapshot,options,prepare,refresh};return true;
  }
  window.AgentBountiesGuidedFundingBoot=boot();
})();
