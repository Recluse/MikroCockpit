'use strict';
const $=id=>document.getElementById(id);
let token=location.hash.slice(1)||sessionStorage.getItem('mikrocockpit-token');
if(location.hash){sessionStorage.setItem('mikrocockpit-token',token);history.replaceState(null,'',location.pathname);}
let state=null,prefs=null,lastRender='',pending=false;
const durations=new Map();
const captions={devices:'Devices',routes:'Internet VPN',wireguard:'WireGuard',bypass:'Exceptions'};
const trayCaptions={direct:'Direct access',exits:'VPN exits',routes:'Internet VPN',wireguard:'WireGuard',bypass:'Exceptions'};
const el=(tag,text,cls)=>{const n=document.createElement(tag);if(text!==undefined)n.textContent=text;if(cls)n.className=cls;return n;};
const flag=(r,k)=>r?.[k]===true||r?.[k]==='true'||r?.[k]==='yes';
const disabled=()=>pending||state?.busy||!!state?.error;
async function api(path,body){
 const response=await fetch(path,{method:body?'POST':'GET',headers:{'X-MikroCockpit-Token':token||'','Content-Type':'application/json'},body:body?JSON.stringify(body):undefined,cache:'no-store'});
 const text=await response.text();let data;try{data=JSON.parse(text);}catch{data={error:text};}
 if(!response.ok)throw new Error(data.error||`HTTP ${response.status}`);return data;
}
function notice(text,error=false){$('notice').textContent=text;$('notice').classList.toggle('error',error);}
async function action(body){
 if(pending||state?.busy||(state?.error&&body.kind!=='refresh'))return;
 pending=true;notice('Sending request…');renderControls();
 try{await api('/api/action',body);notice('Request accepted. Confirm the action if a MikroCockpit window appears.');}
 catch(e){notice(e.message,true);}
 finally{pending=false;await refresh();}
}
function button(caption,body,locked=false,cls='secondary'){
 const b=el('button',caption,cls);b.dataset.action='true';b.dataset.locked=String(locked);b.disabled=locked||disabled();b.addEventListener('click',()=>action(typeof body==='function'?body():body));return b;
}
function ttl(r){return r?.timeout?String(r.timeout):'permanent';}
function normalizeIp(s){return String(s).replace(/\/32$/,'');}
function hasAll(snapshot){return state.config.lan.length>0&&state.config.lan.every(n=>snapshot.direct.some(r=>r.address===n));}
function applyTheme(){const theme=prefs?.theme||'system';document.body.classList.toggle('light',theme==='light'||theme==='system'&&matchMedia('(prefers-color-scheme:light)').matches);for(const id of Object.keys(captions))$(id).hidden=(prefs?.hidden||[]).includes(id);}
function renderControls(){
 const blocked=disabled();for(const b of document.querySelectorAll('[data-action]'))b.disabled=blocked||b.dataset.locked==='true';for(const id of ['all-direct','all-vpn'])$(id).disabled=blocked||!state?.policy_ready;
 $('switch-exit').disabled=blocked||!state?.snapshot?.proxies?.[state?.config?.group]?.all?.length;
 $('refresh').disabled=pending||!!state?.busy;
 $('bypass-form').querySelector('button').disabled=blocked||!state?.snapshot;
}
function render(){
 document.documentElement.lang=state.locale?.language||'en';
 const s=state.snapshot;if(!s){notice(state.error||state.message||'Connecting…',!!state.error);renderControls();return;}
 prefs=state.preferences;applyTheme();
 const c=state.config,group=s.proxies[c.group]||{},selected=group.now||'',actual=s.proxies[selected]?.now||selected;
 const defaults=s.routes.filter(r=>r['dst-address']==='0.0.0.0/0'&&flag(r,'active')&&!flag(r,'disabled'));
 const path=defaults.map(r=>r.gateway).join(' + ');
 const route=defaults[0],routeName=route?.comment||route?.gateway||'',wireguard=s.wg.some(w=>w.name===route?.gateway);
 $('exit-name').textContent=hasAll(s)?'Network using direct access':wireguard?routeName:actual||routeName||'No active exit';
 $('path').textContent=`${c.table} → ${path||'no active default route'}`;
 $('addresses').textContent=`WAN IP: ${(s.wan_addresses||[]).join(', ')||'unavailable'} · WG local IP: ${(s.wg_addresses||[]).join(', ')||'unavailable'} · VPN public IP: ${s.public_ip||'unavailable'} · ${s.public_ip_source||s.public_ip_error||'Checking router egress…'}`;
 $('host').textContent=`${c.name} · ${c.host}`;
 $('protection').textContent=state.policy_ready?'Protection policy verified':'MikroCockpit policy is not installed yet';
 $('cpu').textContent=`${s.resource['cpu-load']??'—'}%`;$('memory').textContent=`${Math.round(Number(s.resource['free-memory']||0)/1048576)} MB`;
 $('version').textContent=`RouterOS ${s.resource.version||'—'}`;$('count').textContent=s.leases.length;$('direct-count').textContent=s.direct.length;
 $('table-name').textContent=c.table;
 const options=(group.all||[]).filter(n=>!['DIRECT','REJECT','PASS'].includes(n));
 if(document.activeElement!==$('exit')){$('exit').replaceChildren(...options.map(n=>{const o=el('option',n);o.value=n;o.selected=n===selected;return o;}));}
 const all=hasAll(s);const body=$('device-rows');body.replaceChildren();
 for(const lease of s.leases){
   const ip=lease.address;if(!ip)continue;
   const direct=s.direct.find(r=>normalizeIp(r.address)===ip);
   const row=el('tr');row.append(el('td',lease['host-name']||lease.comment||lease['mac-address']||'Device'),el('td',ip));
   const mode=el('td');mode.append(el('span',all?'Direct · entire LAN':direct?`Direct · ${ttl(direct)}`:'Router policy',direct||all?'badge warn':'badge'));row.append(mode);
   const controls=el('td');const duration=el('select');duration.setAttribute('aria-label',`Direct access duration for ${ip}`);
   for(const [v,t] of [['15','15 min'],['60','1 hour'],['permanent','Permanent']]){const o=el('option',t);o.value=v;o.selected=(durations.get(ip)||'15')===v;duration.append(o);}
   duration.addEventListener('change',()=>durations.set(ip,duration.value));controls.append(duration);
   controls.append(button('Direct',()=>({kind:'direct',address:ip,minutes:duration.value==='permanent'?null:Number(duration.value)}),!state.policy_ready));
   if(direct)controls.append(button('Restore', {kind:'end_direct',address:ip},!state.policy_ready));row.append(controls);body.append(row);
 }
 $('route-rows').replaceChildren();for(const r of s.routes.filter(r=>r['dst-address']==='0.0.0.0/0')){
   const row=el('div',undefined,'row');const info=el('div');const primary=flag(r,'active')&&String(r.distance)==='1';info.append(el('strong',`${primary?'●':'○'} ${r.comment||r.gateway||'—'}`),el('p',`${r.gateway||'—'} · distance ${r.distance??'—'} · ${primary?'primary':flag(r,'active')?'fallback':'inactive'}`,'muted'));row.append(info);
   row.append(button('Make primary',{kind:'primary',id:r['.id']},!state.policy_ready||flag(r,'dynamic')||flag(r,'disabled')||r['dst-address']!=='0.0.0.0/0'));$('route-rows').append(row);
 }
 $('route-rows').append(el('p',state.policy_ready?'Kill switch verified · no direct WAN fallback':'Kill switch not verified · switching disabled','muted small'));
 $('wg-rows').replaceChildren();for(const w of s.wg){const row=el('div',undefined,'row');row.append(el('span',w.name));const protectedTunnel=c.protected.includes(w.name)||!c.wireguard.includes(w.name);row.append(button(protectedTunnel?'Read only':flag(w,'disabled')?'Enable':'Disable',{kind:'wireguard',name:w.name,enabled:flag(w,'disabled')},protectedTunnel));$('wg-rows').append(row);}
 $('bypass-rows').replaceChildren();for(const r of s.bypass){const row=el('div',undefined,'row');row.append(el('span',r.address),button('Remove',{kind:'remove_bypass',id:r['.id'],address:r.address}));$('bypass-rows').append(row);}
 if(!s.bypass.length)$('bypass-rows').append(el('p','No exceptions added by this app yet.','muted small'));
 $('updated').textContent=`Snapshot: ${new Date(state.updated*1000).toLocaleTimeString(state.locale?.language || 'en')}`;
 notice(state.error||state.message||'Status updated',!!state.error||String(state.message).startsWith('Action error:'));
 renderControls();
}
async function refresh(){
 try{const next=await api('/api/state');const signature=JSON.stringify(next);state=next;if(signature!==lastRender){lastRender=signature;render();}else{renderControls();}}
 catch(e){state={...state,error:e.message};notice(`Cannot reach the app: ${e.message}. Reopen the dashboard from the tray.`,true);renderControls();}
}
$('refresh').onclick=()=>action({kind:'refresh'});
$('switch-exit').onclick=()=>action({kind:'exit',name:$('exit').value});
$('all-direct').onclick=()=>action({kind:'direct',address:'all',minutes:$('global-duration').value==='permanent'?null:Number($('global-duration').value)});
$('all-vpn').onclick=()=>action({kind:'end_direct',address:'all'});
$('bypass-form').onsubmit=e=>{e.preventDefault();action({kind:'add_bypass',address:$('cidr').value});};
$('customize').onclick=()=>{
 $('theme').value=prefs?.theme||'system';$('sections').replaceChildren();
 for(const [id,caption]of Object.entries(captions)){const label=el('label');const input=el('input');input.type='checkbox';input.dataset.section=id;input.checked=!(prefs?.hidden||[]).includes(id);label.append(input,document.createTextNode(caption));$('sections').append(label);}
 $('sections').append(el('hr'),el('h3','Tray sections · order'));
 const order=prefs?.tray_order||Object.keys(trayCaptions);const keys=[...order,...Object.keys(trayCaptions).filter(k=>!order.includes(k))];
 const tray=el('div');tray.id='tray-order';
 for(const key of keys){const row=el('div',undefined,'inline');row.dataset.tray=key;const label=el('label');const input=el('input');input.type='checkbox';input.checked=order.includes(key);label.append(input,document.createTextNode(trayCaptions[key]));const up=el('button','↑','secondary');up.setAttribute('aria-label',`Move ${trayCaptions[key]} up`);up.onclick=()=>{if(row.previousElementSibling)tray.insertBefore(row,row.previousElementSibling);};row.append(label,up);tray.append(row);}$('sections').append(tray);
 const color=el('select');color.id='icon-color';color.setAttribute('aria-label','Menu icon color');for(const value of ['system','monochrome','blue','purple','green','orange','pink']){const o=el('option',value);o.value=value;o.selected=value===(prefs?.icon_color||'system');color.append(o);}$('sections').append(el('label','Menu icon color'),color);
 const title=el('select');title.id='tray-title';title.setAttribute('aria-label','Menu bar title');for(const [v,t]of [['exit','VPN exit'],['cpu','Router CPU'],['name','Profile name'],['icon','Icon only'],['custom','Custom status']]){const o=el('option',t);o.value=v;o.selected=v===(prefs?.tray_title||'exit');title.append(o);}$('sections').append(el('label','Menu bar title'),title);
 $('sections').append(el('h3','Status text — right of icon'));
 for(const [key,caption] of Object.entries({internet:'Internet reachability (this Mac)',connection:'Interface link up/down',speed:'Download / upload (bits/s)',exit:'Selected VPN/proxy exit',enabled:'Enabled tunnels and proxy',interfaces:'Interface indicators',cpu:'CPU load',uptime:'Router uptime',wan_ip:'WAN IP',wg_ip:'WireGuard local IP',public_ip:'VPN public IP'})){
  const label=el('label'),input=el('input');input.type='checkbox';input.dataset.status=key;input.checked=(prefs?.status_fields||['internet','speed','exit']).includes(key);label.append(input,document.createTextNode(caption));$('sections').append(label);
 }
 $('sections').append(el('h3','Interfaces (empty = profile WAN)'));
 for(const i of state?.snapshot?.interfaces||[]){const label=el('label'),input=el('input');input.type='checkbox';input.dataset.interface=i.name;input.checked=(prefs?.status_interfaces||[]).includes(i.name);label.append(input,document.createTextNode(i.name));$('sections').append(label);}
 $('preferences').showModal();
};
$('cancel-preferences').onclick=()=>$('preferences').close();
$('save-preferences').onclick=async()=>{
 const p={theme:$('theme').value,icon_color:$('icon-color').value,hidden:[...document.querySelectorAll('[data-section]')].filter(i=>!i.checked).map(i=>i.dataset.section),tray_order:[...document.querySelectorAll('[data-tray]')].filter(r=>r.querySelector('input').checked).map(r=>r.dataset.tray),tray_title:$('tray-title').value,status_fields:[...document.querySelectorAll('[data-status]')].filter(i=>i.checked).map(i=>i.dataset.status),status_interfaces:[...document.querySelectorAll('[data-interface]')].filter(i=>i.checked).map(i=>i.dataset.interface)};
 try{await api('/api/preferences',p);prefs=p;applyTheme();$('preferences').close();await refresh();}catch(e){notice(e.message,true);}
};
matchMedia('(prefers-color-scheme:light)').addEventListener('change',applyTheme);
refresh();setInterval(()=>{if(!$('preferences').open)refresh();},4000);
