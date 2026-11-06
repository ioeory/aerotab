import test from 'node:test';
import assert from 'node:assert/strict';
import {isDeepStrictEqual} from 'node:util';
import {replay} from './minimize.mjs';
import headless from '@xterm/headless';
const event=text=>({bytes:[...Buffer.from(text)]});
const minimal='\x1b[H中\x1b[H\x1b[31ma';

test('diagnosis: wide overwrite clears width but preserves DIFFERENT normal blank foreground',async()=>{
  const result=await replay([event(minimal)]);
  assert.deepEqual(result.native.cells[0][1],[' ',1,['default',0],['default',0],Array(7).fill(false)]);
  assert.deepEqual(result.xterm.cells[0][1],[' ',1,['palette',1],['default',0],Array(7).fill(false)]);
  assert.deepEqual(result.native.cells[0][0],result.xterm.cells[0][0]);
  // Diagnosis is not a passing equivalence claim; the original strict failing
  // seed-4660 comparison remains untouched in cross-core.test.mjs.
});

test('diagnosis: ordinary follow-up overwrite removes blank-color discrepancy',async()=>{
  const result=await replay([event(minimal)],[event('\x1b[32mX')]);
  assert.equal(isDeepStrictEqual(result.native,result.xterm),true);
});

test('diagnosis: later combining after cursor controls causes REAL visible glyph/cursor divergence',async()=>{
  const result=await replay([event(minimal)],[event('\x1b[2G\x1b[0m\u0301')]);
  assert.equal(result.native.cells[0][0][0],'a\u0301');
  assert.equal(result.xterm.cells[0][0][0],'a');
  assert.deepEqual(result.native.cursor,[1,0]);
  assert.deepEqual(result.xterm.cursor,[2,0]);
});

test('diagnosis: raw C1 and UTF8 C1 both differ; public parser has no execute interception',async()=>{
  for(const bytes of [[0x84],[0xc2,0x84]]) {
    const result=await replay([{bytes}]);
    assert.deepEqual(result.native.cursor,[0,0]);
    assert.deepEqual(result.xterm.cursor,[0,1]);
  }
  const terminal=new headless.Terminal({allowProposedApi:true});
  try {
    assert.equal(typeof terminal.parser.registerExecuteHandler,'undefined');
    for(const name of ['registerCsiHandler','registerDcsHandler','registerEscHandler','registerOscHandler'])assert.equal(typeof terminal.parser[name],'function');
  }finally{terminal.dispose();}
});
