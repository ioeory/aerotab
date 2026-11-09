import type { RpcClient } from './rpc';
export interface AiConfig { enabled:boolean;provider:'openai'|'custom'|'anthropic'|'ollama';apiBase:string;model:string;temperature:number;maxTokens:number;maxRounds:number;maxCalls:number;maxConsecutiveWaits:number;systemPrompt:string;toolsEnabled:boolean }
export interface AiContext {kind:'terminal'|'notes'|'profile'|'resource'|'prompt';id:string;label:string;text:string}
export type AiPermissionMode = 'observer'|'confirm'|'auto';
export interface AiTerminalBinding {sessionId:string;readAuthorized:boolean;mode?:AiPermissionMode;autoApproved?:boolean}
export interface AiTerminalTarget {id:string;title:string}
export interface AiProviderChoice {id:string;name:string;provider:string;apiBase:string;model:string;enabled:boolean;hasKey:boolean}
export interface AiActivity {type:'activity';call:AiToolCall;client?:string;tool?:string;approval?:AiEvent;state:'running'|'approval'|'approved'|'denied'|'completed'|'error'|'canceled';result?:unknown;target?:AiTerminalTarget|null}
export interface AiNotice {type:'notice';code?:'tool_budget'|'terminal_wait_limit'|string;message?:string}
export type AiPart = {type:'text';text:string}|AiActivity|AiNotice;
export interface AiMessage {role:'user'|'assistant';content:string;parts?:AiPart[]}
export interface AiToolCall {id:string;name:string;arguments:Record<string,unknown>}
export interface AiEvent {type:'delta'|'approval'|'tool'|'result'|'done'|'canceled'|'error'|'notice';code?:AiNotice['code'];text?:string;message?:string;approvalId?:string;call?:AiToolCall;client?:string;tool?:string;callId?:string;allowed?:boolean;result?:unknown}
export type McpClientConfig = {id:string;name:string}&({transport:'stdio';command:string;args:string[]}|{transport:'http';url:string});
export interface McpClientEntry {config:McpClientConfig;trusted:boolean;hasAuth:boolean}
export interface McpCatalog {tools:{name:string;description?:string;inputSchema:unknown}[];resources:{uri:string;name:string}[];resourceTemplates:{uriTemplate:string;name:string;description?:string}[];prompts:{name:string;description?:string;arguments?:{name:string;required?:boolean}[]}[]}
export const defaultAiConfig:AiConfig={enabled:false,provider:'openai',apiBase:'https://api.openai.com/v1',model:'gpt-4o-mini',temperature:0.7,maxTokens:1024,maxRounds:12,maxCalls:30,maxConsecutiveWaits:3,systemPrompt:'',toolsEnabled:false};
/** Profile context is deliberately an allowlist; never serialize auth/vault/key paths. */
export function profileContext(profile:{id:string;name:string;kind?:string;ssh?:{host?:string;port?:number;user?:string};tags?:string[]}):AiContext {return {kind:'profile',id:profile.id,label:profile.name,text:JSON.stringify({id:profile.id,name:profile.name,kind:profile.kind,host:profile.ssh?.host,port:profile.ssh?.port,username:profile.ssh?.user,tags:profile.tags})};}
export function boundedTerminalContext(id:string,label:string,text:string,lines:number):AiContext {return {kind:'terminal',id,label,text:text.split('\n').slice(-Math.max(1,Math.min(2000,lines))).join('\n').slice(-65536)};}
export function expandResourceTemplate(template:string,params:Record<string,unknown>):string {
 if(!params||typeof params!=='object'||Array.isArray(params))throw Error('Invalid resource template parameters');
 const uri=template.replace(/\{([^{}]+)\}/g,(_,name:string)=>{if(!/^[A-Za-z0-9_]+$/.test(name)||!Object.hasOwn(params,name)||typeof params[name]!=='string')throw Error('Provide each named template parameter as a string');return encodeURIComponent(params[name] as string);});
 if(/[{}]/.test(uri))throw Error('Unsupported resource template');return uri;
}
export function contextBytes(context:AiContext[]):number{return new TextEncoder().encode(JSON.stringify(context)).length;}
/** Keep the visible timeline local to its assistant turn; never put tool calls in history. */
export function appendAiEvent(message:AiMessage,event:AiEvent,target:AiTerminalTarget|null=null):void {
 if(!message.parts)message.parts=[];
 const parts=message.parts;
 if(event.type==='notice'){parts.push({type:'notice',code:event.code,message:event.message});return;}
 if(event.type==='delta') {const text=event.text??'';message.content+=text;const last=parts.at(-1);if(last?.type==='text')last.text+=text;else parts.push({type:'text',text});return;}
 if(event.type==='canceled'||event.type==='error'||event.type==='done') {for(const part of parts)if(part.type==='activity'&&['running','approval','approved'].includes(part.state))part.state=event.type==='error'?'error':event.type==='canceled'?'canceled':'completed';return;}
 const id=event.call?.id??event.callId;if(!id)return;
 let activity=parts.find((part):part is AiActivity=>part.type==='activity'&&part.call.id===id);
 if(!activity&&event.call){parts.push({type:'activity',call:event.call,state:'running',target});activity=parts.at(-1) as AiActivity;}
 if(!activity)return;
 if(event.type==='tool'){activity.client=event.client;activity.tool=event.tool;}
 if(event.type==='approval'){activity.approval=event;activity.state='approval';}
 if(event.type==='result'){activity.result=event.result;activity.state=event.allowed===false?'denied':(event.result&&typeof event.result==='object'&&('error' in event.result||('isError' in event.result&&event.result.isError)))?'error':'completed';}
}
export function shouldFollowAiScroll(element:{scrollHeight:number;scrollTop:number;clientHeight:number}):boolean{return element.scrollHeight-element.scrollTop-element.clientHeight<=64;}
export function readableAiError(message:string,t:(key:string)=>string):string{return message.includes('AI tool loop limit exceeded')?t('toolBudgetNotice'):message;}
export function readableAiNotice(notice:AiNotice,t:(key:string)=>string):string{return notice.code==='tool_budget'?t('toolBudgetNotice'):notice.code==='terminal_wait_limit'?t('terminalWaitNotice'):notice.message??'';}
export function aiToolLabel(activity:AiActivity,t:(key:string)=>string):string{const key:Record<string,string>={terminal_read:'toolRead',terminal_wait:'toolWait',terminal_input:'toolInput',terminal_interrupt:'toolInterrupt'};const label=key[activity.call.name];return label?t(label):activity.tool??activity.call.name;}
export function readableAiCommand(call:AiToolCall):string {const args=call.arguments;for(const key of ['text','command','script','input'])if(typeof args[key]==='string')return args[key] as string;return JSON.stringify(args,null,2);}
export async function pollAi(rpc:RpcClient,id:string,cursor:number):Promise<{events:AiEvent[];cursor:number;done:boolean}>{return rpc.call('ai.poll',{id,cursor});}
