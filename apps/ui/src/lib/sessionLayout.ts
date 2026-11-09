import type { PaneNode, Tab } from './tabs.svelte';
import type { SessionMeta } from './types';
export type SafeRestore = {kind:'local'} | {kind:'ssh-profile';id:string};
export type SavedNode = {type:'leaf';paneIndex:number} | {type:'split';direction:'row'|'col';ratios:number[];children:SavedNode[]};
export interface SavedTab {title:string;customTitle?:string|null;panes:SafeRestore[];layout:SavedNode;activePaneIndex:number;maximizedPaneIndex?:number|null}
const ratios=(values:number[])=>{const total=values.reduce((a,b)=>a+b,0);return values.map(v=>v/total);};
export function safeRestore(value:unknown):SafeRestore|null {if(!value||typeof value!=='object')return null;const r=value as Record<string,unknown>;if(r.kind==='local')return{kind:'local'};if(r.kind==='ssh-profile'&&typeof r.id==='string'&&r.id.length>0&&r.id.length<512)return{kind:'ssh-profile',id:r.id};return null;}
function validateNode(value:unknown,count:number,seen:Set<number>,depth=0):SavedNode|null {
 if(depth>24||!value||typeof value!=='object')return null;const n=value as Record<string,unknown>;
 if(n.type==='leaf'){const i=n.paneIndex;if(!Number.isInteger(i)||(i as number)<0||(i as number)>=count||seen.has(i as number))return null;seen.add(i as number);return{type:'leaf',paneIndex:i as number};}
 if(n.type!=='split'||!['row','col'].includes(n.direction as string)||!Array.isArray(n.children)||n.children.length<2||n.children.length>64||!Array.isArray(n.ratios)||n.ratios.length!==n.children.length||n.ratios.some(v=>typeof v!=='number'||!Number.isFinite(v)||v<=0))return null;
 const children=n.children.map(c=>validateNode(c,count,seen,depth+1));if(children.some(c=>!c))return null;return{type:'split',direction:n.direction as 'row'|'col',ratios:ratios(n.ratios),children:children as SavedNode[]};
}
export function parseSessionLayout(raw:unknown):SavedTab[]{
 if(Array.isArray(raw))return raw.slice(0,100).flatMap(e=>{const restore=safeRestore(e?.restore);return restore?[{title:typeof e.title==='string'?e.title:'',panes:[restore],layout:{type:'leaf' as const,paneIndex:0},activePaneIndex:0}]:[];});
 if(!raw||typeof raw!=='object'||(raw as {version:number}).version!==1||!Array.isArray((raw as {tabs:unknown[]}).tabs))return[];
 return (raw as {tabs:unknown[]}).tabs.slice(0,100).flatMap(item=>{if(!item||typeof item!=='object')return[];const t=item as Record<string,unknown>;if(!Array.isArray(t.panes)||!t.panes.length||t.panes.length>64)return[];const panes=t.panes.map(safeRestore);if(panes.some(p=>!p))return[];const seen=new Set<number>();const layout=validateNode(t.layout,panes.length,seen);if(!layout||seen.size!==panes.length)return[];const index=(v:unknown)=>Number.isInteger(v)&&(v as number)>=0&&(v as number)<panes.length?v as number:0;return[{title:typeof t.title==='string'?t.title.slice(0,512):'',customTitle:typeof t.customTitle==='string'?t.customTitle.slice(0,512):null,panes:panes as SafeRestore[],layout,activePaneIndex:index(t.activePaneIndex),maximizedPaneIndex:t.maximizedPaneIndex==null?null:index(t.maximizedPaneIndex)}];});
}
export function instantiateLayout(node:SavedNode,opened:Array<SessionMeta|null>,newId:()=>string):PaneNode|null {
 if(node.type==='leaf'){const pane=opened[node.paneIndex];return pane?{type:'leaf',id:pane.id,pane}:null;}
 const children:PaneNode[]=[],weights:number[]=[];node.children.forEach((child,i)=>{const built=instantiateLayout(child,opened,newId);if(built){children.push(built);weights.push(node.ratios[i]??1);}});
 if(!children.length)return null;if(children.length===1)return children[0]??null;return{type:'split',id:newId(),direction:node.direction,children,ratios:ratios(weights)};
}
export function snapshotSessionLayout(tabs:Tab[],restoreMap:Map<string,unknown>){return{version:1 as const,tabs:tabs.filter(t=>t.kind!=='transfer').flatMap(tab=>{
 const panes:SafeRestore[]=[],indices=new Map<string,number>();for(const pane of tab.panes){const recipe=pane.profileId?safeRestore({kind:'ssh-profile',id:pane.profileId}):safeRestore(restoreMap.get(pane.id));if(recipe){indices.set(pane.id,panes.length);panes.push(recipe);}}
 function node(n:PaneNode):SavedNode|null{if(n.type==='leaf'){const i=indices.get(n.pane.id);return i===undefined?null:{type:'leaf',paneIndex:i};}const children:SavedNode[]=[],weights:number[]=[];n.children.forEach((c,i)=>{const saved=node(c);if(saved){children.push(saved);const weight=n.ratios[i]??1;weights.push(Number.isFinite(weight)&&weight>0?weight:1);}});if(!children.length)return null;if(children.length===1)return children[0]??null;return{type:'split',direction:n.direction,children,ratios:ratios(weights)};}
 const layout=node(tab.layout);return layout?[{title:tab.title,customTitle:tab.customTitle??null,panes,layout,activePaneIndex:indices.get(tab.activePaneId)??0,maximizedPaneIndex:tab.maximizedPaneId?indices.get(tab.maximizedPaneId)??null:null}]:[];
})};}
