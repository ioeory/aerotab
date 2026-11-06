// Public state-only transport. No terminal parser, bytes or xterm dependency.
const u64=value=>typeof value==='string'&&/^(0|[1-9][0-9]*)$/.test(value)&&BigInt(value)<=18446744073709551615n;
const integer=(value,max)=>Number.isInteger(value)&&value>=0&&value<=max;
function fail(message){throw new Error(message);}
function color(c){if(!c||!['named','indexed','rgb'].includes(c.kind)||!integer(c.value,c.kind==='rgb'?0xffffff:c.kind==='indexed'?255:268))fail('malformed color');}
export class FrameTransport {
 #tickets=new WeakSet();
 constructor({session,view,epoch,ack}){
  if(!u64(session)||!u64(epoch)||!['native','webview'].includes(view)||typeof ack!=='function')fail('malformed attachment');
  Object.assign(this,{session,view,epoch,ack,state:null});this.drawGeneration=0;
 }
 apply(input){
  // Validate completely before changing state or emitting any ACK.
  const f=structuredClone(input);
  if(!f||f.version!==1||f.session!==this.session||f.view!==this.view||f.epoch!==this.epoch)fail('stale attachment');
  if(!u64(f.revision)||!u64(f.output_seq)||!integer(f.wire_seq,0xffffffff)||typeof f.full!=='boolean')fail('malformed frame');
  if(!integer(f.cols,512)||f.cols<2||!integer(f.lines,256)||f.lines<1||!integer(f.history,100000)||!integer(f.mode,0xffffffff)||typeof f.ended!=='boolean')fail('malformed geometry');
  if(!f.cursor||!integer(f.cursor.col,f.cols-1)||!Number.isInteger(f.cursor.line)||f.cursor.line< -100000||f.cursor.line>100000||typeof f.cursor.visible!=='boolean')fail('malformed cursor');
  if(this.state&&(BigInt(f.revision)<=BigInt(this.state.revision)||BigInt(f.output_seq)<BigInt(this.state.output_seq)))fail('stale revision');
  if(f.full){if(f.base_revision!==null)fail('malformed full base');}
  else if(!this.state||!u64(f.base_revision)||f.base_revision!==this.state.revision||f.cols!==this.state.cols||f.lines!==this.state.lines)fail('base mismatch: request full');
  if(!Array.isArray(f.rows)||f.rows.length>f.lines)fail('malformed rows');
  const seen=new Set();
  for(const row of f.rows){
   if(!integer(row.y,f.lines-1)||seen.has(row.y)||!Array.isArray(row.cells)||row.cells.length!==f.cols)fail('malformed row');seen.add(row.y);
   for(const c of row.cells){if(!c||typeof c.text!=='string'||c.text.length>1024||!integer(c.width,2)||!integer(c.flags,65535))fail('malformed cell');color(c.fg);color(c.bg);}
  }
  if(f.full&&seen.size!==f.lines)fail('incomplete full');
  const rows=f.full?Array(f.lines):this.state.rows.slice();for(const row of f.rows)rows[row.y]=row.cells;
  const state={...f,rows};
  // IDs are compared using exact BigInt; wrapping u32 wire_seq is diagnostic,
  // never used as monotonic ordering or as substitute for revision/epoch.
  this.state=state;this.drawGeneration++;
  this.ack(this.message('applied',null));return state;
 }
 message(phase,evidence){const s=this.state;return {version:1,session:this.session,view:this.view,epoch:this.epoch,revision:s.revision,output_seq:s.output_seq,wire_seq:s.wire_seq,phase,evidence,physical_present:false};}
 drawTicket(){if(!this.state)fail('no state');const ticket=Object.freeze({generation:this.drawGeneration,epoch:this.epoch,revision:this.state.revision});this.#tickets.add(ticket);return ticket;}
 drawComplete(ticket,evidence){
  if(!ticket||!this.#tickets.has(ticket)||ticket.generation!==this.drawGeneration||ticket.epoch!==this.epoch||ticket.revision!==this.state?.revision)fail('stale draw');
  if(!['canvas-raf','native-draw-callback'].includes(evidence))fail('unsupported evidence');
  this.#tickets.delete(ticket);
  this.ack(this.message('drawn',evidence));
 }
}
