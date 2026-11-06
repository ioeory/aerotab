import test from 'node:test';
import assert from 'node:assert/strict';
import { restoreRetainedSourceStatus } from './nativeViewTransaction.ts';
test('failed staging restores retained actual source status and allows subsequent switch',()=>{
 let status='pending';let generation=1;const source={};
 const options={source,generation:1,current:()=>({source,generation,ready:true,closed:false}),report:()=>status='native'};
 assert.equal(restoreRetainedSourceStatus(options),true);assert.equal(status,'native');
 assert.notEqual(status,'pending');status='pending';generation++;assert.equal(restoreRetainedSourceStatus(options),false);assert.equal(status,'pending');
});
test('closed, invalid source, unready and superseded callbacks never overwrite new status',()=>{
 const source={};for(const patch of [{closed:true},{ready:false},{source:{}},{generation:2}]){
 let reports=0;assert.equal(restoreRetainedSourceStatus({source,generation:1,current:()=>({source,generation:1,ready:true,closed:false,...patch}),report:()=>reports++}),false);assert.equal(reports,0);
 }
});
