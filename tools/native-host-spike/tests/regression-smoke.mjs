import { spawnSync } from 'node:child_process';
const backend=process.argv[2]??'wayland';
const mode=process.argv[3]??'--reload-regression';
const r=spawnSync('timeout',['110s','target/debug/webview-window-spike','--handoff-smoke',mode],{env:{...process.env,GDK_BACKEND:backend,WEBKIT_DISABLE_DMABUF_RENDERER:'1'},encoding:'utf8',maxBuffer:32*1024*1024});
const events=(r.stdout??'').split('\n').flatMap(l=>{try{return [JSON.parse(l)];}catch{return [];}});
const has=name=>events.some(e=>e.event===name);
let errors=[];
if(r.status!==0)errors.push(`exit ${r.status}`);
if(!has('single_core_two_views_pass'))errors.push('missing handoff pass');
if(has('unexpected_timeout'))errors.push('unexpected timeout');
if(mode==='--reload-regression'&&(!has('reload_old_generation_rejected')||!has('reload_regression_pass')))errors.push('missing genuine reload stale-generation evidence');
if(mode==='--input-regression'&&!has('input_fifo_regression_pass'))errors.push('missing real native/Web IPC FIFO evidence');
if(mode==='--delayed-startup'&&!has('startup_negative_control_pass'))errors.push('missing legacy negative control');
if(mode==='--deliberate-timeout'&&(!has('expected_timeout')||!has('expected_timeout_complete')))errors.push('missing timeout or cleanup event');
if(events.some(e=>e.event==='startup_ready'&&e.frameearly!==0))errors.push('early frame');
console.log(JSON.stringify({event:'regression_runner',backend,mode,errors,ready:events.filter(e=>e.event==='startup_ready').length,stale_generation:events.filter(e=>e.event==='reload_old_generation_rejected').length,expected_timeouts:events.filter(e=>e.event==='expected_timeout').length,unexpected_timeouts:events.filter(e=>e.event==='unexpected_timeout').length}));
if(errors.length){console.error(r.stderr);process.exit(1);}
