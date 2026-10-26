#!/usr/bin/env node
import fs from 'node:fs';
import {buildDashboard, contract} from './generate-client-channel-dashboard.mjs';

// 按原面板ID逐项取证；API接受、有限数值、缺触发和结构面板分别记账。
const dashboard=buildDashboard();
const flatten=panels=>panels.flatMap(panel=>[panel,...flatten(panel.panels??[])]);
const panels=flatten(dashboard.panels);
const end=Number(process.env.HELIX_AUDIT_END_MS??Date.now());
const start=end-Number(process.env.HELIX_AUDIT_HOURS??3)*3600000;
const selected={environment:'pre',service:process.env.HELIX_AUDIT_SERVICE??'',...Object.fromEntries(contract.scopeFields.map(field=>[field,process.env[`HELIX_AUDIT_${field.toUpperCase()}`]??'']))};
const urls={prometheus:process.env.HELIX_PRE_PROMETHEUS_URL??'http://192.168.6.66:30090',loki:process.env.HELIX_DIAGNOSTIC_LOKI_URL??'http://192.168.6.66:31000'};

// 与Grafana字符串/regex格式相同地转义，保留精确身份和空服务语义。
function interpolate(expr) {
  return expr.replace(/\$\{([^}:]+):(doublequote|regex)\}/g,(_,key,format)=>{
    const value=selected[key]??'';
    return format==='doublequote'?JSON.stringify(value):JSON.stringify(value.replace(/[.*+?^${}()|[\]\\]/g,'\\$&')).slice(1,-1);
  }).replaceAll('$__rate_interval','5m').replaceAll('$__interval','1m').replaceAll('$__range',`${Math.round((end-start)/1000)}s`);
}

// range查有限样本而非只看最新时刻；保留实际时间，旧数据不冒充当前活跃。
async function inspect(panel,target) {
  const source=target.datasource.type;
  const instant=target.instant||target.queryType==='instant';
  const endpoint=source==='prometheus'?(instant?'/api/v1/query':'/api/v1/query_range'):(instant?'/loki/api/v1/query':'/loki/api/v1/query_range');
  const url=new URL(endpoint,urls[source]);
  url.searchParams.set('query',interpolate(target.expr));
  if(instant)url.searchParams.set('time',String(end/1000));
  else {
    url.searchParams.set('start',source==='loki'?String(BigInt(start)*1000000n):String(start/1000));
    url.searchParams.set('end',source==='loki'?String(BigInt(end)*1000000n):String(end/1000));
    url.searchParams.set('step','60');
    if(source==='loki')url.searchParams.set('limit','3');
  }
  try {
    const response=await fetch(url,{signal:AbortSignal.timeout(20000)});
    const raw=await response.text();
    if(!response.ok)throw Error(`HTTP ${response.status}: ${raw.slice(0,200)}`);
    const body=JSON.parse(raw);
    if(body.status!=='success')throw Error(JSON.stringify(body).slice(0,200));
    const result=body.data.result??[];
    const log=body.data.resultType==='streams';
    const samples=result.flatMap(item=>item.values??(item.value?[item.value]:[]));
    const finite=log?samples:samples.filter(([,value])=>Number.isFinite(Number(value)));
    const times=finite.map(([time])=>log?Number(BigInt(time)/1000000n):Number(time)*1000);
    return {refId:target.refId,source,api:'PASS',sampleStatus:finite.length?'REAL_SAMPLE':samples.length?'NON_FINITE':'NO_SAMPLE',series:result.length,samples:samples.length,finiteSamples:finite.length,firstSample:times.length?new Date(Math.min(...times)).toISOString():null,lastSample:times.length?new Date(Math.max(...times)).toISOString():null,recentSample:times.some(time=>time>=end-120000),logSampleLimit:log?3:undefined,proves:finite.length?(log?'真实诊断记录存在，未证明整个列表完整':'所选时段存在有限数值，未证明当前业务旅程完成'):'仅API接受查询，未取得有限样本'};
  }catch(error){return {refId:target.refId,source,api:'FAIL',sampleStatus:'QUERY_FAILED',error:String(error.message)}}
}
const results=[];
for(const panel of panels) {
  const queries=[];
  for(const target of panel.targets??[])queries.push(await inspect(panel,target));
  const state=queries.length===0?'STRUCTURAL':queries.some(q=>q.api==='FAIL')?'QUERY_FAILED':queries.every(q=>q.sampleStatus==='REAL_SAMPLE')?'REAL_SAMPLE':queries.some(q=>q.sampleStatus==='REAL_SAMPLE')?'PARTIAL_SAMPLE':'NO_SAMPLE';
  results.push({panelId:panel.id,title:panel.title,type:panel.type,baseline:panel.id<=111,state,queries});
}
const counts=Object.fromEntries([...new Set(results.map(r=>r.state))].map(state=>[state,results.filter(r=>r.state===state).length]));
const evidence={observedAt:new Date().toISOString(),window:{start:new Date(start).toISOString(),end:new Date(end).toISOString()},selected,baseline:contract.baselinePanels,current:{panels:panels.length,targets:results.flatMap(r=>r.queries).length},counts,evidenceLevel:'QUERY_AND_SAMPLE_AUDIT',realBusinessJourneyVerified:false,results};
const output=process.env.HELIX_AUDIT_OUTPUT??'/tmp/helix-client-channel-panel-audit.json';
fs.writeFileSync(output,JSON.stringify(evidence,null,2)+'\n');
console.log(JSON.stringify({output,counts,current:evidence.current}));
if(counts.QUERY_FAILED)process.exitCode=1;
