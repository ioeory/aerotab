export const PANEL_MIN_WIDTH=240, PANEL_MIN_HEIGHT=160;
export function spanLimits(pair:number,pixels:number,minimum=PANEL_MIN_HEIGHT){const min=Math.min(pair*50,pair*100*minimum/Math.max(1,pixels));return{min,max:pair*100-min};}
export function updateWorkbenchViewport(layout:WorkbenchLayout,view:Viewport){if(view.width<1||view.height<1)return layout;view={width:Math.max(view.width,PANEL_MIN_WIDTH),height:Math.max(view.height,PANEL_MIN_HEIGHT)};return{...layout,panels:layout.panels.map(p=>({...p,rect:clampRect(p.rect,view)}))};}
export type PanelType = 'sftp' | 'notes' | 'system' | 'ai';
export type Side = 'left' | 'right' | 'top' | 'bottom' | 'float';
export interface Rect { x: number; y: number; width: number; height: number }
export interface Viewport { width: number; height: number }
export interface PanelPlacement { id: string; side: Side; size: number; span?: number; rect: Rect }
export interface WorkbenchLayout { version: 1; panels: PanelPlacement[] }
export class PanelRegistry {
  private entries = new Map<string, { id: string; type: PanelType }>();
  register(panel: { id: string; type: PanelType }) { if (!panel.id || this.entries.has(panel.id)) throw new Error('duplicate panel ID'); this.entries.set(panel.id, panel); return () => this.entries.delete(panel.id); }
  ids() { return [...this.entries.keys()]; }
  get(id: string) { return this.entries.get(id); }
}
const finite = (v: unknown, fallback: number) => typeof v === 'number' && Number.isFinite(v) ? v : fallback;
export function clampRect(rect: Rect, view: Viewport): Rect {
  const width = Math.min(Math.max(PANEL_MIN_WIDTH, finite(rect.width, 360)), Math.max(0, view.width));
  const height = Math.min(Math.max(PANEL_MIN_HEIGHT, finite(rect.height, 280)), Math.max(0, view.height));
  return { x: Math.max(0, Math.min(finite(rect.x, 32), view.width - width)), y: Math.max(0, Math.min(finite(rect.y, 32), view.height - height)), width, height };
}
export function defaultLayout(ids: string[]): WorkbenchLayout { return { version: 1, panels: ids.map(id => ({ id, side: 'right', size: 320, rect: { x: 40, y: 40, width: 420, height: 360 } })) }; }
export function restoreLayout(raw: unknown, ids: string[]): WorkbenchLayout {
  const fallback = defaultLayout(ids); if (!raw || typeof raw !== 'object' || (raw as WorkbenchLayout).version !== 1 || !Array.isArray((raw as WorkbenchLayout).panels)) return fallback;
  const seen = new Set<string>(); const panels: PanelPlacement[] = [];
  for (const item of (raw as WorkbenchLayout).panels) { if (!item || typeof item.id!=='string' || !/^[\w:.-]{1,160}$/.test(item.id) || seen.has(item.id)) continue; seen.add(item.id); const base = fallback.panels.find(p => p.id === item.id)??defaultLayout([item.id]).panels[0]!;
    panels.push({ id: item.id, side: ['left','right','top','bottom','float'].includes(item.side) ? item.side : 'right', size: Math.max(120, Math.min(900, finite(item.size, 320))), span: Math.max(.1,Math.min(20,finite(item.span,1))), rect: clampRect(item.rect && typeof item.rect === 'object' ? item.rect : base.rect, {width:10000,height:10000}) }); }
  return {version:1,panels:[...panels,...fallback.panels.filter(p=>!seen.has(p.id))]};
}
export function placePanel(layout: WorkbenchLayout, id: string, side: Side, index = Infinity): WorkbenchLayout {
  const panel = layout.panels.find(p => p.id === id); if (!panel) return layout;
  const panels = layout.panels.filter(p => p.id !== id); const peers = panels.filter(p => p.side === side); const before = peers[Math.max(0,index)];
  panels.splice(before ? panels.indexOf(before) : panels.length,0,{...panel,side}); return {...layout,panels};
}
export function reconcilePanels(layout: WorkbenchLayout, ids: string[]): WorkbenchLayout {
  return {...layout,panels:[...layout.panels,...defaultLayout(ids.filter(id=>!layout.panels.some(p=>p.id===id))).panels]};
}
export interface PanelDropTarget {side:Side;index:number;preview:Rect}
export function panelDropTarget(layout:WorkbenchLayout,boxes:Record<string,Rect>,view:Viewport,visible:string[],dragged:string,x:number,y:number):PanelDropTarget|null {
 const candidates=layout.panels.filter(p=>p.id!==dragged&&p.side!=='float'&&visible.includes(p.id));
 const under=candidates.find(p=>{const r=boxes[p.id];return r&&x>=r.x&&x<=r.x+r.width&&y>=r.y&&y<=r.y+r.height;});
 const side=under?.side??(x<48?'left':x>view.width-48?'right':y<48?'top':y>view.height-48?'bottom':null);if(!side)return null;
 const peers=candidates.filter(p=>p.side===side);const vertical=side==='left'||side==='right';const coordinate=vertical?y:x;
 let index=peers.findIndex(p=>{const r=boxes[p.id];return r&&coordinate<(vertical?r.y+r.height/2:r.x+r.width/2);});if(index<0)index=peers.length;
 const peer=peers[Math.min(index,peers.length-1)];const r=peer?boxes[peer.id]:undefined;const preview=r?(vertical?{x:r.x,y:index===peers.length?r.y+r.height-6:r.y,width:r.width,height:6}:{x:index===peers.length?r.x+r.width-6:r.x,y:r.y,width:6,height:r.height}):{x:side==='right'?view.width-80:0,y:side==='bottom'?view.height-60:0,width:vertical?80:view.width,height:vertical?view.height:60};
 return{side,index,preview};
}
export function resizeRect(rect: Rect, edge: string, dx: number, dy: number, view: Viewport): Rect {
  const r = clampRect(rect,view); let {x,y,width,height}=r;
  if(edge.includes('w')) { x=Math.max(0,Math.min(x+dx,x+width-Math.min(PANEL_MIN_WIDTH,width)));width=r.x+r.width-x; }
  if(edge.includes('e')) width=Math.max(Math.min(PANEL_MIN_WIDTH,view.width-x),Math.min(width+dx,view.width-x));
  if(edge.includes('n')) {y=Math.max(0,Math.min(y+dy,y+height-Math.min(PANEL_MIN_HEIGHT,height)));height=r.y+r.height-y;}
  if(edge.includes('s')) height=Math.max(Math.min(PANEL_MIN_HEIGHT,view.height-y),Math.min(height+dy,view.height-y));
  return clampRect({x,y,width,height},view);
}
export function measureLayout(layout: WorkbenchLayout, view: Viewport, visible: string[]) {
  const sideCount=(side:Side)=>layout.panels.filter(p=>p.side===side&&visible.includes(p.id)).length;
  const extent={width:Math.max(view.width,Math.max(sideCount('top'),sideCount('bottom'))*PANEL_MIN_WIDTH+(sideCount('left')?PANEL_MIN_WIDTH:0)+(sideCount('right')?PANEL_MIN_WIDTH:0),sideCount('left')||sideCount('right')?720:240),height:Math.max(view.height,(Math.max(sideCount('left'),sideCount('right'))+(sideCount('top')?1:0)+(sideCount('bottom')?1:0))*PANEL_MIN_HEIGHT,(sideCount('top')+sideCount('bottom')+1)*PANEL_MIN_HEIGHT)};
  const main: Rect={x:0,y:0,width:extent.width,height:extent.height};const panels:Record<string,Rect>={};
  for(const side of ['top','bottom','left','right'] as const) {
    const group=layout.panels.filter(p=>p.side===side&&visible.includes(p.id));if(!group.length)continue;
    const vertical=side==='left'||side==='right'; const extent=vertical?main.width:main.height;
    const crossMinimum=vertical?PANEL_MIN_WIDTH:PANEL_MIN_HEIGHT;
    const reserve=vertical?PANEL_MIN_WIDTH*(1+(side==='left'&&sideCount('right')?1:0)):PANEL_MIN_HEIGHT*(Math.max(1,sideCount('left'),sideCount('right'))+(side==='top'&&sideCount('bottom')?1:0));
    const size=Math.max(crossMinimum,Math.min(Math.max(crossMinimum,...group.map(p=>p.size)),Math.max(crossMinimum,extent-reserve),extent*.42));
    const total=group.reduce((sum,p)=>sum+(p.span??1),0);let offset=0;
    const axis=vertical?main.height:main.width;const minimum=vertical?PANEL_MIN_HEIGHT:PANEL_MIN_WIDTH;const remainder=Math.max(0,axis-minimum*group.length);
    group.forEach(p=>{const shares=group.map(p=>(p.span??1)/total);const constrained=shares.some(share=>share*axis<minimum);
    const fraction=constrained?(minimum+remainder*(p.span??1)/total)/axis:(p.span??1)/total;panels[p.id]=vertical?{x:side==='left'?main.x:main.x+main.width-size,y:main.y+main.height*offset,width:size,height:main.height*fraction}:{x:main.x+main.width*offset,y:side==='top'?main.y:main.y+main.height-size,width:main.width*fraction,height:size};offset+=fraction;});
    if(vertical){if(side==='left')main.x+=size;main.width-=size;}else{if(side==='top')main.y+=size;main.height-=size;}
  }
  for(const p of layout.panels)if(p.side==='float'&&visible.includes(p.id))panels[p.id]=clampRect(p.rect,extent);
  return {main,panels,extent};
}
