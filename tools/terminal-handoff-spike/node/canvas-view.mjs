import {FrameTransport} from './frame-transport.mjs';
// Real DOM Canvas view of canonical cell state. Not xterm, not a parser. Host
// injects frame objects via receive() and forwards ACK JSON via onAck. A real
// browser rAF callback is draw scheduling evidence, NOT physical presentation.
const ansi=['#000000','#cd0000','#00cd00','#cdcd00','#0000ee','#cd00cd','#00cdcd','#e5e5e5','#7f7f7f','#ff0000','#00ff00','#ffff00','#5c5cff','#ff00ff','#00ffff','#ffffff'];
function css(c){if(c.kind==='rgb')return `#${c.value.toString(16).padStart(6,'0')}`;const n=c.value;if(n<16)return ansi[n];if(n>=16&&n<232){const v=n-16;const l=i=>i?55+40*i:0;return `rgb(${l(Math.floor(v/36))},${l(Math.floor(v/6)%6)},${l(v%6)})`;}if(n<256)return `rgb(${8+(n-232)*10},${8+(n-232)*10},${8+(n-232)*10})`;return n===257?'#101820':'#eeeeee';}
export function mountCanvasView(canvas,{session,epoch,onAck}){
 if(!(canvas instanceof HTMLCanvasElement))throw new Error('real DOM canvas required');
 const transport=new FrameTransport({session,epoch,view:'webview',ack:onAck});
 let disposed=false;
 return {transport,receive(frame){
  if(disposed)throw new Error('view disposed');transport.apply(frame);
  const ticket=transport.drawTicket();
  requestAnimationFrame(()=>{
   if(disposed||ticket.revision!==transport.state.revision)return;
   const s=transport.state;canvas.width=s.cols*12;canvas.height=s.lines*24;
   const ctx=canvas.getContext('2d');if(!ctx)throw new Error('Canvas2D unavailable');
   ctx.font='18px monospace';ctx.textBaseline='top';
   s.rows.forEach((row,y)=>row.forEach((cell,x)=>{
    let fg=css(cell.fg),bg=css(cell.bg);if(cell.flags&1)[fg,bg]=[bg,fg];
    ctx.fillStyle=bg;ctx.fillRect(x*12,y*24,Math.max(1,cell.width)*12,24);
   }));
   // Separate background/glyph passes so a wide trailing cell never erases the
   // second half of the preceding glyph.
   s.rows.forEach((row,y)=>row.forEach((cell,x)=>{
    let fg=css(cell.fg);if(cell.flags&1)fg=css(cell.bg);
    if(cell.width&&!(cell.flags&256)){ctx.fillStyle=fg;ctx.fillText(cell.text,x*12,y*24);}
   }));
   if(s.cursor.visible){ctx.strokeStyle='#ffffff';ctx.strokeRect(s.cursor.col*12,s.cursor.line*24,11,23);}
   transport.drawComplete(ticket,'canvas-raf');
  });
 },dispose(){disposed=true;}};
}
