# Strict compact JSON from the pinned reader binary; no arbitrary output.
function flat(raw, kind, parts, fields, n, i, key, value, seen) {
 n=split(raw,parts,","); if(n!=3) return 0;
 for(i=1;i<=n;i++) {
  if(split(parts[i],fields,":")!=2) return 0;
  key=fields[1]; value=fields[2];
  if(seen[key]++) return 0;
  if(kind=="binding") {
   if(key=="\"service_id\"") { if(value!="\"" sid "\"") return 0; }
   else if(key=="\"source_revision\"") { if(value!="\"" revision "\"") return 0; }
   else if(key=="\"capability\"") { if(value!="\"" capability "\"") return 0; }
   else return 0;
  } else {
   if(key=="\"binding\"") { if(value!="true") return 0; }
   else if(key=="\"schema\"") { if(value!="\"agentizable/recovery-funding-build-v1\"") return 0; }
   else if(key=="\"unbound_inert_only\"") { if(value!="false") return 0; }
   else return 0;
  }
 }
 return 1;
}
{
 if(NR!=1 || length($0)>1024 || substr($0,1,1)!="{" || substr($0,length($0),1)!="}") { bad=1; next; }
 raw=substr($0,2,length($0)-2);
 if(!match(raw,/"binding":\{[^{}]*\}/)) { bad=1; next; }
 binding=substr(raw,RSTART+11,RLENGTH-12);
 outer=substr(raw,1,RSTART-1) "\"binding\":true" substr(raw,RSTART+RLENGTH);
 if(!flat(binding,"binding") || !flat(outer,"outer")) bad=1;
}
END { if(NR!=1 || bad) exit 24; print "baked_funding_identity_exact=true"; }
