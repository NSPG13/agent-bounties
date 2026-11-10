function private_scalar(value, h) {
 h=value; if(substr(h,1,2)=="0x") h=substr(h,3);
 h=tolower(h);
 return length(h)==64 && h ~ /^[0-9a-f]+$/ && h ~ /[1-9a-f]/ && ("x" h)<"xfffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141";
}
function private_ipv4(value, a, n, i) {
 n=split(value,a,"."); if(n!=4) return 0;
 for(i=1;i<=4;i++) if(a[i] !~ /^[0-9]+$/ || length(a[i])>3 || a[i]+0>255 || (length(a[i])>1 && substr(a[i],1,1)=="0")) return 0;
 return a[1]+0==10 || (a[1]+0==172 && a[2]+0>=16 && a[2]+0<=31) || (a[1]+0==192 && a[2]+0==168);
}
function hostname(value, a, n, i) {
 if(length(value)<1 || length(value)>253) return 0;
 n=split(value,a,".");
 for(i=1;i<=n;i++) if(length(a[i])<1 || length(a[i])>63 || a[i] !~ /^[A-Za-z0-9][A-Za-z0-9-]*$/ || a[i] ~ /-$/) return 0;
 return 1;
}
BEGIN {
 while ((getline line < policy)>0) {
  n=split(line,f,"\t");
  if(n!=3 || f[1] !~ /^(parent|worker)$/ || f[2] !~ /^[A-Z_][A-Z0-9_]*$/) config_bad++;
  if(f[1]==role) { if(f[2] in expected) config_bad++; expected[f[2]]=f[3]; expected_count++; }
 }
 close(policy); if(!expected_count) config_bad++;
 host_role=(role=="parent");
 split("RENDER_ENV_IS_DOCKER RENDER_INTERNAL_IP RENDER_INTERNAL_HOSTNAME", render_keys," ");
 for(i in render_keys) metadata[render_keys[i]]=1;
 split("KUBERNETES_SERVICE_PORT_HTTPS KUBERNETES_PORT KUBERNETES_PORT_443_TCP KUBERNETES_PORT_443_TCP_PROTO KUBERNETES_PORT_443_TCP_PORT KUBERNETES_PORT_443_TCP_ADDR KUBERNETES_SERVICE_HOST KUBERNETES_SERVICE_PORT",kube_keys," ");
 for(i in kube_keys) { metadata[kube_keys[i]]=1; kube[kube_keys[i]]=1; }
 split("PATH HOME USER LOGNAME LANG LC_ALL TZ HOSTNAME PORT PWD SHLVL _ RENDER_CPU_COUNT RENDER_DISCOVERY_SERVICE RENDER_EXTERNAL_HOSTNAME RENDER_EXTERNAL_URL RENDER_INSTANCE_ID RENDER_WEB_CONCURRENCY WEB_CONCURRENCY",basics," ");
 for(i in basics) allowed[basics[i]]=1;
 split("ENABLE_HOSTED_FINANCIAL_SIGNING ENABLE_BASE_TX_BROADCAST ENABLE_X402_HOSTED_RELAY ENABLE_BOND_SPONSORSHIP ENABLE_STRIPE_LIVE_EXECUTION ENABLE_STRIPE_PUBLIC_CHECKOUT ENABLE_MANAGED_SERVICE_ORDERS ENABLE_MANAGED_SERVICE_PUBLIC_CHECKOUT ENABLE_MANAGED_SERVICE_REFUND_EXECUTION ENABLE_MANAGED_RECOVERED_BOUNTY_REFUNDS ENABLE_SPONSORED_POSTING_CREATION",flags," ");
 for(i in flags) financial_flags[flags[i]]=1;
 RS="\0";
}
{
 n=index($0,"="); key=substr($0,1,n-1); value=substr($0,n+1);
 if(n<2 || key !~ /^[A-Za-z_][A-Za-z0-9_]*$/ || length($0)>131072) { bad++; next; }
 if(parent!="" && FILENAME==parent) {
  if(parent_seen[key]++) bad++;
  parent_values[key]=value; next;
 }
 if(seen[key]++) { bad++; next; }
 if(key in expected) {
  want=expected[key];
  if(want=="@present") { if(length(value)==0) bad++; else dependency_slots++; }
  else if(want=="@private_scalar") { if(!private_scalar(value)) bad++; else dependency_slots++; }
  else if(want=="@present32") { if(length(value)<32) bad++; else dependency_slots++; }
  else if(substr(want,1,6)=="@same:") {
   source_key=substr(want,7);
   if(!(source_key in parent_values) || length(value)==0 || value!=parent_values[source_key]) bad++;
   else inherited_slots++;
  }
  else if(value!=want) bad++;
 }
 else if(host_role && key in metadata) {
  metadata_count++; meta_values[key]=value;
  if(key=="RENDER_ENV_IS_DOCKER" && value!="true" && value!="1") bad++;
  if(key=="RENDER_INTERNAL_IP" && !private_ipv4(value)) bad++;
  if(key=="RENDER_INTERNAL_HOSTNAME" && !hostname(value)) bad++;
  if(key in kube) kube_seen++;
 }
 else if(host_role && key in allowed) {
  if(length(value)>4096 || value ~ /[\r\n]/) bad++;
  if(key=="PORT" && (value !~ /^[0-9]+$/ || value+0<1 || value+0>65535)) bad++;
 }
 else if(role=="parent" && key in financial_flags) { if(value!="false") bad++; }
 else unexpected++;
}
END {
 for(key in expected) if(!(key in seen)) bad++;
 if(kube_seen) {
  if(kube_seen!=8) bad++;
  host=meta_values["KUBERNETES_SERVICE_HOST"];
  if(!private_ipv4(host) || meta_values["KUBERNETES_PORT_443_TCP_ADDR"]!=host) bad++;
  if(meta_values["KUBERNETES_SERVICE_PORT_HTTPS"]!="443" || meta_values["KUBERNETES_SERVICE_PORT"]!="443" || meta_values["KUBERNETES_PORT_443_TCP_PORT"]!="443") bad++;
  if(meta_values["KUBERNETES_PORT_443_TCP_PROTO"]!="tcp") bad++;
  if(meta_values["KUBERNETES_PORT"]!="tcp://" host ":443" || meta_values["KUBERNETES_PORT_443_TCP"]!="tcp://" host ":443") bad++;
 }
 print "configuration_mismatches=" (bad+config_bad+0);
 print "unexpected_key_count=" (unexpected+0);
 if(bad || unexpected || config_bad) exit 23;
 print "validated_platform_metadata_count=" (metadata_count+0);
 print "present_dependency_slots=" (dependency_slots+0);
 print "matched_parent_dependency_slots=" (inherited_slots+0);
 print "runtime_configuration_exact=true";
}
