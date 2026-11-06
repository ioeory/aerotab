// Bounded deterministic ddmin of the EXISTING seed-4660 trace; no new fuzzing.
import {readFileSync,writeFileSync} from 'node:fs';
import {spawn} from 'node:child_process';
import {createInterface} from 'node:readline';
import {fileURLToPath} from 'node:url';
import headless from '@xterm/headless';
import {installNativeUnicode} from './unicode-provider.mjs';
import {Utf8Bridge} from './utf8-bridge.mjs';
import {snapshot} from './snapshot.mjs';
import {isDeepStrictEqual} from 'node:util';

export async function replay(events, followup = []) {
  const child=spawn(fileURLToPath(new URL('../target/debug/cross-core',import.meta.url)),[],{stdio:['pipe','pipe','inherit']});
  const pending=[];
  createInterface({input:child.stdout}).on('line',line=>pending.shift()(JSON.parse(line)));
  const request=value=>new Promise(resolve=>{pending.push(resolve);child.stdin.write(JSON.stringify(value)+'\n');});
  const term=new headless.Terminal({cols:80,rows:24,scrollback:20000,allowProposedApi:true,logLevel:'off'});
  installNativeUnicode(term);
  const bridge=new Utf8Bridge(term);
  try {
    for(const event of [...events,...followup]) {
      const result=await request({op:'append',...event});
      if(result.error)throw new Error(result.error);
      await request({op:'next'});
      const record=await request({op:'inflight'});
      if(record.bytes)await bridge.write(Uint8Array.from(record.bytes));
      else if(record.cols)term.resize(record.cols,record.rows);
      else await bridge.eof();
      await request({op:'ack',seq:record.seq,pendingUtf8:bridge.pendingBytes});
    }
    return {native:await request({op:'snapshot'}),xterm:snapshot(term)};
  } finally {term.dispose();child.stdin.end();}
}
function target(result) {
  // Preserve ORIGINAL foreground discrepancy on a normal blank cell (width=1),
  // not an assumed wide spacer; other syntax/UTF8 failures are not acceptable.
  for(let y=0;y<result.native.cells.length;y++)for(let x=0;x<result.native.cells[y].length;x++) {
    const n=result.native.cells[y][x], t=result.xterm.cells[y]?.[x];
    if(n?.[0]===' '&&t?.[0]===' '&&n[1]===1&&t[1]===1&&n[2][0]==='default'&&t[2][0]==='palette'&&t[2][1]===1)return true;
  }
  return false;
}
if(process.argv.includes('--run')) {
  let events=JSON.parse(readFileSync(new URL('../artifacts/mixed-seed-4660-events.json',import.meta.url)));
  const original=events.length;
  let calls=0;
  const accepts=async candidate=>{if(++calls>600)throw new Error('bounded ddmin exhausted');return target(await replay(candidate));};
  if(!await accepts(events))throw new Error('original spacer foreground predicate did not reproduce');
  // First retain EVERY resize while minimizing original byte/chunk events.
  for(const preserveResize of [true,false]) {
    let n=2;
    while(events.length>=2) {
      const size=Math.ceil(events.length/n);
      let reduced=false;
      for(let start=0;start<events.length;start+=size) {
        const candidate=events.filter((event,i)=>i<start||i>=start+size||(preserveResize&&event.cols));
        if(candidate.length===events.length)continue;
        if(await accepts(candidate)){events=candidate;n=Math.max(2,n-1);reduced=true;break;}
      }
      if(!reduced){if(n>=events.length)break;n=Math.min(events.length,n*2);}
    }
    const name=preserveResize?'spacer-resize-preserved':'spacer-minimal';
    writeFileSync(new URL(`../generated/${name}.json`,import.meta.url),JSON.stringify(events,null,2)+'\n');
    console.log(JSON.stringify({stage:name,original,remaining:events.length,calls,events}));
  }
  const result=await replay(events);
  writeFileSync(new URL('../artifacts/spacer-minimal-snapshots.json',import.meta.url),JSON.stringify(result,null,2));
  console.log('exactSnapshotsEqual',isDeepStrictEqual(result.native,result.xterm));
}
