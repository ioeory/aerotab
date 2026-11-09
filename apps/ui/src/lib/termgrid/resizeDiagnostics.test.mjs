import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'vite';
const server = await createServer({ configFile:false, optimizeDeps:{noDiscovery:true,include:[]}, server:{middlewareMode:true} });
const { recordTerminalGeometry, terminalGeometrySnapshot } = await server.ssrLoadModule('/src/lib/termgrid/resizeDiagnostics.ts');
await server.close();
test('terminal geometry diagnostics are bounded, copied and contain no terminal or connection data', () => {
 for(let i=0;i<140;i++) recordTerminalGeometry({reason:'fit',backend:'webgl',revision:i,rect:[0,0,1920,1080],client:[1920,1080],scroll:[1920,1080],target:[200,60],applied:[180,50],dpr:1.25,canvas:[1800,1000],resets:2,secret:'do-not-copy',sessionId:'private-host'});
 const events=terminalGeometrySnapshot();assert.equal(events.length,128);
 assert.equal(events.at(-1).revision,139);assert.equal(JSON.stringify(events).includes('do-not-copy'),false);assert.equal(JSON.stringify(events).includes('private-host'),false);
 events[0].rect[0]=999;assert.equal(terminalGeometrySnapshot()[0].rect[0],0);
});
