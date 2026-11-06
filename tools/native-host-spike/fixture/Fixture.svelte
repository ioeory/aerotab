<script>
  import { onMount, tick } from 'svelte';
  let split = $state(50);
  let hidden = $state(false);
  let modal = $state(false);
  let focus = $state(-1);
  let ack = $state(0);
  let stage = $state('initial');
  let sequence = 0;
  let views=$state(['native','native']);
  let canvases=[];
  const transports=[null,null],inputEpoch=[0,0],focusGeneration=[0,0];let selecting=null;let inputError=$state('');
  function sendInput(pane,kind,data={}){window.webkit.messageHandlers.layout.postMessage(JSON.stringify({kind:'input',pane,document:documentSession,generation,focus_generation:focusGeneration[pane],epoch:inputEpoch[pane],action:kind,...data}));}
  function cellEvent(e,pane,action){const r=e.currentTarget.getBoundingClientRect();sendInput(pane,action,{col:Math.floor((e.clientX-r.left-12)/11),row:Math.floor((e.clientY-r.top-12)/23)});}
  function keyEvent(e,pane){if(e.isComposing)return;e.preventDefault();sendInput(pane,'key',{key:e.key,shift:e.shiftKey,alt:e.altKey,control:e.ctrlKey,super_key:e.metaKey});}
  function requestView(pane){window.webkit.messageHandlers.layout.postMessage(JSON.stringify({kind:'switch',pane,target:views[pane]==='native'?'web':'native'}));}
  const documentSession=crypto.randomUUID();let generation=0;const tickets=[null,null];
  let first;
  let second;
  async function report() {
    await tick();
    if (!first || !window.webkit?.messageHandlers?.layout) return;
    const panes = [first, second].map(el => {
      const r = el.getBoundingClientRect();
      return {x:r.x,y:r.y,width:r.width,height:r.height};
    });
    window.webkit.messageHandlers.layout.postMessage(JSON.stringify({seq:++sequence, panes, hidden, modal, focus, dpr:devicePixelRatio, stage}));
  }
  onMount(() => {
    const waitingInput=[null,null];
    window.inputError=message=>{inputError=message;};
    window.inputContext=(pane,epoch,fg,g)=>{if(g!==generation)return;inputEpoch[pane]=epoch;focusGeneration[pane]=fg;if(waitingInput[pane]!==null){sendInput(pane,'commit',{text:waitingInput[pane]});waitingInput[pane]=null;}};
    window.fixtureInput=(pane,text)=>{waitingInput[pane]=text;canvases[pane].focus();sendInput(pane,'focus');};
    window.gridCommit=(pane,target)=>{views[pane]=target;};
    window.gridFrame=(f)=>{
      if(f.document!==documentSession||f.startup_generation!==generation)return;
      if (!Number.isSafeInteger(f.epoch)||f.cells.length!==f.cols*f.rows) throw Error('invalid frame');
      const tf=f.transport;let transport=transports[f.pane];if(!transport||transport.epoch!==tf.epoch){transport=new window.FrameTransport({session:tf.session,view:'webview',epoch:tf.epoch,ack:a=>{if(a.phase==='drawn')window.webkit.messageHandlers.layout.postMessage(JSON.stringify({kind:'gridAck',pane:f.pane,document:documentSession,generation,ack:a}));}});transports[f.pane]=transport;}
      if(!transport.state||BigInt(tf.revision)>BigInt(transport.state.revision))transport.apply(tf);
      else if(tf.revision!==transport.state.revision)return;
      const ticket=transport.drawTicket();inputEpoch[f.pane]=f.epoch;
      const canvas=canvases[f.pane];canvas.width=f.cols*11+24;canvas.height=f.rows*23+24;
      const ctx=canvas.getContext('2d');ctx.font='17px monospace';ctx.textBaseline='top';ctx.fillStyle='rgb(24,28,36)';ctx.fillRect(0,0,canvas.width,canvas.height);
      let ink=0;for(let i=0;i<f.cells.length;i++){const c=f.cells[i];if(c.width===0)continue;const x=12+i%f.cols*11,y=12+Math.floor(i/f.cols)*23;ctx.fillStyle=`rgb(${c.bg})`;ctx.fillRect(x,y,c.width*11,23);ctx.save();ctx.beginPath();ctx.rect(x,y,c.width*11,23);ctx.clip();ctx.fillStyle=`rgb(${c.fg})`;ctx.font=`${c.flags&4?'italic ':''}${c.flags&2?'bold ':''}17px monospace`;ctx.globalAlpha=c.flags&128?0.6:1;ctx.fillText(c.text,x,y);if(c.flags&8)ctx.fillRect(x,y+20,c.width*11,1);if(c.flags&512)ctx.fillRect(x,y+11,c.width*11,1);ctx.restore();}
      if(f.cursor_visible){ctx.fillStyle='rgb(230,230,230)';const x=12+f.cursor[0]*11,y=12+f.cursor[1]*23;ctx.fillRect(x,f.cursor_shape==='Underline'?y+21:y,f.cursor_shape==='Beam'?2:11,f.cursor_shape==='Block'?23:f.cursor_shape==='Beam'?23:2);}
      const pixels=ctx.getImageData(0,0,canvas.width,canvas.height).data;for(let i=0;i<pixels.length;i+=4)if(pixels[i]>70||pixels[i+1]>70||pixels[i+2]>80)ink++;
      tickets[f.pane]=ticket;
      requestAnimationFrame(()=>{if(tickets[f.pane]!==ticket||transports[f.pane]!==transport)return;transport.drawComplete(ticket,'canvas-raf');});
    };
    window.hostAck = (seq) => { ack = seq; window.webkit.messageHandlers.layout.postMessage(JSON.stringify({kind:'ack',seq})); };
    window.fixtureStep = async (next) => {
      stage = next;
      if (next === 'split') split = 32;
      if (next === 'hide') hidden = true;
      if (next === 'show') hidden = false;
      if (next === 'modal') modal = true;
      if (next === 'close') modal = false;
      if (next === 'focus0') focus = 0;
      if (next === 'focus1') focus = 1;
      await report();
      return true;
    };
    window.legacyStartupNegativeControl=(delay,timeout)=>{const control={legacy_early_frames:0,legacy_timeouts:0};let installed=false;const legacyDispatch=()=>{if(!installed)control.legacy_early_frames++;};legacyDispatch();setTimeout(()=>{if(!installed)control.legacy_timeouts++;},timeout);setTimeout(()=>{installed=true;window.webkit.messageHandlers.layout.postMessage(JSON.stringify({kind:'negativeControl',...control}));},delay);};
    window.fixtureChallenge=(session,g)=>{if(session!==documentSession)return;generation=g;window.webkit.messageHandlers.layout.postMessage(JSON.stringify({kind:'fixtureReady',document:documentSession,generation}));};
    window.webkit.messageHandlers.layout.postMessage(JSON.stringify({kind:'fixtureHello',document:documentSession}));
    const observer = new ResizeObserver(report);
    observer.observe(document.body);
    report();
    return () => observer.disconnect();
  });
</script>

<header><b>Actual Tauri WebviewWindow + isolated Svelte 5</b><span>stage {stage} · ack {ack} · DPR {typeof devicePixelRatio === 'number' ? devicePixelRatio : 1}</span></header>
<aside>
  <p>Native GLArea regions are sibling GTK overlay widgets, not WebView canvases.</p>
  <label>Split <input type="range" min="20" max="80" bind:value={split} oninput={report}></label>
  <button onclick={() => { hidden = !hidden; report(); }}>Tab hide/show</button>
  <button onclick={() => { modal = true; report(); }}>Web modal</button>
  <button onclick={() => { focus = 0; report(); }}>Focus native pane 1</button>
  <button onclick={() => { focus = 1; report(); }}>Focus native pane 2</button>
  <button onclick={()=>requestView(0)}>Pane 1: {views[0]} → switch</button>
  <button onclick={()=>requestView(1)}>Pane 2: {views[1]} → switch</button>
  <input placeholder="WebView-owned keyboard input">
  <p>Composition proof only. Hardware GPU and real IME composition NOT proven.</p>
</aside>
<main class:inactive={hidden} style:grid-template-columns={`${split}fr ${100-split}fr`}>
  <section bind:this={first}><canvas tabindex="0" role="application" aria-label="Terminal pane 1" onfocus={()=>sendInput(0,'focus')} onkeydown={e=>keyEvent(e,0)} oncompositionend={e=>sendInput(0,'commit',{text:e.data})} onpaste={e=>{e.preventDefault();sendInput(0,'paste',{text:e.clipboardData.getData('text/plain')});}} onwheel={e=>{e.preventDefault();sendInput(0,'scroll',{lines:e.deltaY<0?3:-3});}} onmousedown={e=>{selecting=0;cellEvent(e,0,'selectStart');}} onmouseup={e=>{if(selecting===0)cellEvent(e,0,'selectEnd');selecting=null;}} bind:this={canvases[0]} style:visibility={views[0]==='web'?'visible':'hidden'}></canvas></section>
  <section bind:this={second}><canvas tabindex="0" role="application" aria-label="Terminal pane 2" onfocus={()=>sendInput(1,'focus')} onkeydown={e=>keyEvent(e,1)} oncompositionend={e=>sendInput(1,'commit',{text:e.data})} onpaste={e=>{e.preventDefault();sendInput(1,'paste',{text:e.clipboardData.getData('text/plain')});}} onwheel={e=>{e.preventDefault();sendInput(1,'scroll',{lines:e.deltaY<0?3:-3});}} onmousedown={e=>{selecting=1;cellEvent(e,1,'selectStart');}} onmouseup={e=>{if(selecting===1)cellEvent(e,1,'selectEnd');selecting=null;}} bind:this={canvases[1]} style:visibility={views[1]==='web'?'visible':'hidden'}></canvas></section>
</main>
{#if modal}<div class="modal"><article><h1>Svelte modal</h1><p>Both native views hidden and input blocked.</p><input placeholder="Modal WebView input"><button onclick={() => { modal = false; report(); }}>Close</button></article></div>{/if}
{#if inputError}<div role="alert" style="position:fixed;bottom:35px;left:0;right:0;z-index:100;background:#642020;padding:8px">{inputError}</div>{/if}
<footer>Svelte rects → WebKit message → native GTK allocation → sequence ACK. No top-level terminal window.</footer>
<style>
:global(*){box-sizing:border-box}:global(body){margin:0;background:#101b2a;color:#e2edf7;font:15px sans-serif;width:100vw;height:100vh;overflow:hidden}header{height:70px;padding:18px;background:#24384f;display:flex;flex-direction:column;gap:8px}header span{font-size:12px;color:#b5d8f5}aside{position:absolute;left:0;top:70px;bottom:34px;width:250px;padding:18px;background:#19293d}button,input{display:block;width:100%;margin:12px 0;padding:9px}main{position:absolute;left:266px;right:16px;top:90px;bottom:54px;display:grid;gap:14px}section{background:#080f19;border:2px solid #56809f;position:relative;min-width:0}section span{position:absolute;top:8px;left:8px}.inactive{visibility:hidden}footer{position:absolute;bottom:0;height:34px;padding:8px;font-size:12px}.modal{position:fixed;inset:0;background:#203044;display:grid;place-items:center;z-index:10}article{padding:35px;background:#344e6b;max-width:500px}
</style>
