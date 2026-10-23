import { useEffect, useId, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { AlertTriangle, Check, CirclePause, CirclePlay, FileWarning, GitBranch, KeyRound, LoaderCircle, MessageSquareText, Plus, RotateCcw, Save, Send, ShieldCheck, Square, TerminalSquare, Trash2 } from 'lucide-react';
import { allowedPlanActions, buildGateInvocation, buildQuestionAnswers, createSoftwareTemplate, formatDuration, humanRequestIdentity, humanRequestKind, parseDependencies, runtimeSeconds, structuredGateActions, taskRoles, validateAutopilotInput, type GateAction } from './autopilot-logic.js';
import { activityKindLabel } from './activity-logic.js';
import './autopilot.css';

export type TaskRole = 'planner'|'coder'|'mobile-dev'|'tester'|'reviewer'|'human';
export type TaskStatus = 'pending'|'ready'|'running'|'needsInput'|'passed'|'failed'|'cancelled'|'interrupted';
export type PlanStatus = 'draft'|'approved'|'running'|'paused'|'needsInput'|'cancelling'|'completed'|'failed'|'cancelled'|'interrupted';
export type PlanLimits = {maxAttempts:number;maxRuntimeMinutes:number;maxTokens:number;maxParallel:number};
export type HumanRequest = {requestId:unknown;method:string;threadId?:string|null;turnId?:string|null;params:unknown};
type UserQuestion = {id:string;header?:string;question?:string;options?:Array<{label:string;description?:string}>};
export type PlanTask = {id:string;title:string;description:string;role:TaskRole;dependencies:string[];model?:string;effort?:string;status?:TaskStatus;attempts?:number;threadId?:string;turnId?:string;summary?:string;inputTokens?:number;outputTokens?:number;startedAt?:number;finishedAt?:number;humanRequests?:HumanRequest[]};
export type PlanInput = {planId?:string;title:string;objective:string;workspace:string;limits:PlanLimits;tasks:PlanTask[]};
export type AutopilotPlan = PlanInput & {revision:number;approvedRevision?:number;status:PlanStatus;createdAt:number;updatedAt:number;totalInputTokens:number;totalOutputTokens:number};
type AutopilotEvent = {id:number;at:number;kind:string;taskId?:string;message:string};
type AutopilotSnapshot = {plan:AutopilotPlan|null;executionAvailable:boolean;connectionMessage:string;events:AutopilotEvent[]};

const emptyLimits:PlanLimits={maxAttempts:2,maxRuntimeMinutes:120,maxTokens:100000,maxParallel:1};
const blankInput=():PlanInput=>({title:'',objective:'',workspace:'',limits:{...emptyLimits},tasks:[]});
const roleLabel:Record<TaskRole,string>={planner:'Planner',coder:'Coder','mobile-dev':'Mobile Dev',tester:'Tester',reviewer:'Reviewer',human:'Human review'};
const statusLabel:Record<string,string>={draft:'Draft',approved:'Approved',running:'Running',paused:'Paused',needsInput:'Needs input',cancelling:'Cancelling…',completed:'Completed',failed:'Failed',cancelled:'Cancelled',interrupted:'Interrupted',pending:'Pending',ready:'Ready',passed:'Passed'};
const asInput=(plan:AutopilotPlan):PlanInput=>({planId:plan.planId,title:plan.title,objective:plan.objective,workspace:plan.workspace,limits:{...plan.limits},tasks:plan.tasks.map(task=>({...task,dependencies:[...task.dependencies]}))});
const formatTokens=(value:number)=>Intl.NumberFormat('en-US',{notation:value>=10000?'compact':'standard',maximumFractionDigits:1}).format(value||0);
const formatEventTime=(at:number)=>at?new Date(at*1000).toLocaleTimeString('vi-VN',{hour:'2-digit',minute:'2-digit',second:'2-digit'}):'—';
const asRecord=(value:unknown):Record<string,unknown>=>value&&typeof value==='object'&&!Array.isArray(value)?value as Record<string,unknown>:{};
const redact=(value:unknown,key=''):unknown=>{
  if(/token|secret|password|authorization|api.?key/i.test(key))return '[redacted]';
  if(Array.isArray(value))return value.map(item=>redact(item));
  if(value&&typeof value==='object')return Object.fromEntries(Object.entries(value as Record<string,unknown>).map(([childKey,child])=>[childKey,redact(child,childKey)]));
  return value;
};
const prettyParams=(params:unknown)=>JSON.stringify(redact(params),null,2);
const scopeText=(value:unknown,fallback:string)=>value===undefined||value===null?fallback:typeof value==='string'?value:JSON.stringify(redact(value),null,2);
const requestQuestions=(params:unknown):UserQuestion[]=>{
  const direct=asRecord(params).questions;
  return Array.isArray(direct)?direct.filter(item=>item&&typeof item==='object').map(item=>{
    const value=asRecord(item); const options=Array.isArray(value.options)?value.options.map(option=>{const entry=asRecord(option);return{label:String(entry.label||''),description:entry.description?String(entry.description):undefined}}).filter(option=>option.label):undefined;
    return{id:String(value.id||''),header:value.header?String(value.header):undefined,question:value.question?String(value.question):undefined,options};
  }).filter(question=>question.id):[];
};

function DependencyEditor({task,disabled,onCommit}:{task:PlanTask;disabled:boolean;onCommit:(dependencies:string[])=>void}){
  const serialized=task.dependencies.join(', ');
  const [raw,setRaw]=useState(serialized);
  useEffect(()=>setRaw(serialized),[serialized]);
  return <label className="dependency-input"><span>Depends on</span><input value={raw} disabled={disabled} onChange={event=>setRaw(event.target.value)} onBlur={()=>{const dependencies=parseDependencies(raw);setRaw(dependencies.join(', '));onCommit(dependencies)}} placeholder="task-id, another-task"/></label>;
}

function HumanRequestCard({task,request,busy,onRespond,onError}:{task:PlanTask;request:HumanRequest;busy:boolean;onRespond:(task:PlanTask,request:HumanRequest,decision:'accept'|'decline'|'cancel'|'submit',input?:unknown)=>void;onError:(message:string)=>void}){
  const kind=humanRequestKind(request.method);
  const params=asRecord(request.params);
  const questions=requestQuestions(request.params);
  const [answers,setAnswers]=useState<Record<string,string>>({});
  const [formValue,setFormValue]=useState('{}');
  const fieldPrefix=useId();
  const icon=kind==='command'?<TerminalSquare size={16}/>:kind==='file'?<FileWarning size={16}/>:kind==='permissions'?<KeyRound size={16}/>:<MessageSquareText size={16}/>;
  const label={command:'Command approval',file:'File change approval',permissions:'Permission approval',questions:'Questions from agent',elicitation:'MCP form request',unsupported:'Unsupported request'}[kind];
  const submitQuestions=()=>{
    const input=buildQuestionAnswers(questions,answers);
    if(!questions.length){onError('Codex sent a question request without a supported questions array. The request remains blocked.');return}
    if(Object.keys(input).length!==questions.length){onError('Answer every question before submitting.');return}
    onRespond(task,request,'submit',input);
  };
  const submitForm=()=>{try{onRespond(task,request,'submit',JSON.parse(formValue))}catch{onError('MCP form content must be valid JSON.')}};
  return <div className={`protocol-request ${kind}`}><div className="protocol-request-head"><span>{icon}</span><div><b>{label}</b><small>{request.method}</small></div><em>Turn only</em></div><div className="request-scope"><span>Task</span><b>{task.id}</b><span>Thread / turn</span><b>{request.threadId||task.threadId||'—'} / {request.turnId||task.turnId||'—'}</b><span>Request ID</span><b>{scopeText(request.requestId,'—')}</b></div>
    {kind==='command'&&<div className="scope-highlight"><span>Command requested</span><code>{scopeText(params.command??params.cmd,'Command was not supplied')}</code>{Boolean(params.cwd)&&<small>Working directory: {String(params.cwd)}</small>}</div>}
    {kind==='file'&&<div className="scope-highlight"><span>File changes requested</span><code>{scopeText(params.changes??params.path??params.filePath??params,'File scope was not supplied')}</code></div>}
    {kind==='permissions'&&<div className="scope-highlight"><span>Permissions requested for this turn</span><code>{JSON.stringify(redact(params.permissions),null,2)}</code></div>}
    {kind==='questions'&&<div className="request-questions">{questions.map(question=><label key={question.id}><span>{question.header||question.id}</span><b>{question.question||'Answer required'}</b><input list={question.options?.length?`${fieldPrefix}-${question.id}`:undefined} value={answers[question.id]||''} onChange={event=>setAnswers(values=>({...values,[question.id]:event.target.value}))} placeholder="Type your answer"/>{question.options?.length&&<datalist id={`${fieldPrefix}-${question.id}`}>{question.options.map(option=><option value={option.label} key={option.label}>{option.description}</option>)}</datalist>}</label>)}</div>}
    {kind==='elicitation'&&<label className="elicitation-input"><span>Form content (JSON)</span><textarea value={formValue} onChange={event=>setFormValue(event.target.value)}/></label>}
    <details className="request-raw"><summary>Exact request scope</summary><pre>{prettyParams(request.params)}</pre></details>
    {kind==='unsupported'?<div className="unsupported-request"><AlertTriangle size={15}/><span>This server request type is unsupported and remains blocked. It cannot be approved automatically. Cancel the run safely or update the controller.</span></div>:<div className="request-actions">{['command','file','permissions'].includes(kind)&&<><button disabled={busy} className="approve-request" onClick={()=>onRespond(task,request,'accept')}><Check size={14}/> {kind==='permissions'?'Grant requested permissions for this turn':`Approve this ${kind} for this turn`}</button><button disabled={busy} onClick={()=>onRespond(task,request,'decline')}><Square size={13}/> Decline this {kind} request</button></>}{kind==='questions'&&<button disabled={busy} onClick={submitQuestions}><Send size={14}/> Submit typed answers</button>}{kind==='elicitation'&&<><button disabled={busy} onClick={submitForm}><Send size={14}/> Submit this MCP form</button><button disabled={busy} onClick={()=>onRespond(task,request,'decline')}>Decline this MCP request</button><button disabled={busy} onClick={()=>onRespond(task,request,'cancel')}>Cancel this MCP request</button></>}</div>}
  </div>;
}

export function AutopilotPage(){
  const [snapshot,setSnapshot]=useState<AutopilotSnapshot|null>(null);
  const [draft,setDraft]=useState<PlanInput>(blankInput);
  const [dirty,setDirty]=useState(false);
  const [loading,setLoading]=useState(true);
  const [busy,setBusy]=useState('');
  const [error,setError]=useState('');
  const [gateResponses,setGateResponses]=useState<Record<string,string>>({});
  const requestGeneration=useRef(0);
  const lastLoadedRevision=useRef<number|null>(null);
  const dirtyRef=useRef(false);
  const busyRef=useRef('');

  const refresh=async(background=false)=>{
    const generation=++requestGeneration.current;
    if(!background)setLoading(true);
    try{
      const next=await invoke<AutopilotSnapshot>('get_autopilot_snapshot');
      if(generation!==requestGeneration.current)return;
      setSnapshot(next); setError('');
      if(next.plan&&!dirtyRef.current&&lastLoadedRevision.current!==next.plan.revision){setDraft(asInput(next.plan));lastLoadedRevision.current=next.plan.revision}
    }catch(reason){if(generation===requestGeneration.current)setError(String(reason))}
    finally{if(generation===requestGeneration.current&&!background)setLoading(false)}
  };
  useEffect(()=>{let alive=true;void refresh();const timer=setInterval(()=>{if(alive&&!busyRef.current)void refresh(true)},1000);return()=>{alive=false;clearInterval(timer);requestGeneration.current++}},[]);

  const plan=snapshot?.plan??null;
  const validation=useMemo(()=>validateAutopilotInput(draft),[draft]);
  const actions=allowedPlanActions(plan,Boolean(busy));
  const gateTasks=(plan?.tasks??[]).filter(task=>task.status==='needsInput');
  const protocolRequests=gateTasks.flatMap(task=>(task.humanRequests||[]).map(request=>({task,request,key:humanRequestIdentity(plan?.revision||0,task.id,request)})));
  const structuredGateTasks=gateTasks.filter(task=>!(task.humanRequests||[]).length);
  const cancelling=plan?.status==='cancelling'||busy==='Cancelling';
  const responsesDisabled=Boolean(busy)||cancelling;
  const displayPlanStatus=cancelling?'cancelling':plan?.status||'draft';
  const gateResponseKey=(task:PlanTask)=>`${plan?.revision||0}\u0000${task.id}\u0000${task.threadId||''}\u0000${task.turnId||''}`;
  const now=Date.now()/1000;
  const updateDraft=(change:(value:PlanInput)=>PlanInput)=>{dirtyRef.current=true;setDraft(value=>change(value));setDirty(true);setError('')};
  const updateTask=(index:number,change:Partial<PlanTask>)=>updateDraft(value=>({...value,tasks:value.tasks.map((task,i)=>i===index?{...task,...change}:task)}));

  const mutate=async(command:string,args:Record<string,unknown>,label:string)=>{
    if(busyRef.current)return;
    busyRef.current=label;setBusy(label);setError('');requestGeneration.current++;
    try{
      const next=await invoke<AutopilotSnapshot>(command,args);
      setSnapshot(next);
      if(next.plan){setDraft(asInput(next.plan));lastLoadedRevision.current=next.plan.revision}
      dirtyRef.current=false;setDirty(false);
    }catch(reason){setError(String(reason))}
    finally{busyRef.current='';setBusy('')}
  };
  const save=()=>{if(validation.length){setError(validation[0]);return}void mutate('save_autopilot_plan',{input:draft},'Saving')};
  const useTemplate=()=>updateDraft(value=>({...value,title:value.title||'Software delivery plan',tasks:createSoftwareTemplate(value.objective)}));
  const simplePlanAction=(command:string,label:string,extra:Record<string,unknown>={})=>{
    if(!plan||plan.status==='cancelling'||busyRef.current)return;
    void mutate(command,{planId:plan.planId,...extra},label);
  };
  const respondGate=(task:PlanTask,action:GateAction)=>{
    if(!plan||plan.status==='cancelling'||busyRef.current)return;
    const response=gateResponses[gateResponseKey(task)]?.trim()||'';
    if(!response){setError('Enter direction or review notes before responding.');return}
    const invocation=buildGateInvocation(plan.planId||'',task.id,response,action);
    void mutate(invocation.command,invocation.args,'Responding');
  };
  const respondRequest=(task:PlanTask,request:HumanRequest,decision:'accept'|'decline'|'cancel'|'submit',input?:unknown)=>{
    if(!plan||plan.status==='cancelling'||busyRef.current)return;
    void mutate('respond_autopilot_request',{planId:plan.planId,planRevision:plan.revision,taskId:task.id,requestId:request.requestId,threadId:request.threadId||task.threadId||'',turnId:request.turnId||task.turnId||'',decision,input:input??null},'Responding');
  };

  if(loading&&!snapshot)return <div className="autopilot-state"><LoaderCircle size={18}/> Loading persisted Autopilot state…</div>;
  return <div className="autopilot-page">
    <div className="autopilot-heading"><div><div className="eyebrow"><span className="eyebrow-line"/> SOFTWARE AUTOPILOT V2</div><h1>Dependency driven delivery<span className="h1-dot">.</span></h1><p>Define the work, approve its revision, then let Codex execute the serial task queue through independent test and review gates.</p></div><div className={`autopilot-connection ${snapshot?.executionAvailable?'online':'offline'}`}><i/><div><b>{snapshot?.executionAvailable?'Controller ready':'Controller unavailable'}</b><span>{snapshot?.connectionMessage||'No response from the local controller'}</span></div></div></div>

    {error&&<div className="autopilot-error" role="alert"><AlertTriangle size={17}/><span>{error}</span><button onClick={()=>setError('')}>Dismiss</button></div>}
    <section className="autopilot-summary">
      <div><span>PLAN STATUS</span><strong className={`plan-status ${displayPlanStatus}`}>{statusLabel[displayPlanStatus]}</strong><small>{cancelling?'Waiting for the active process to confirm shutdown':plan?`Revision ${plan.revision}${plan.approvedRevision===plan.revision?' · approved':''}`:'No saved plan'}</small></div>
      <div><span>TASK PROGRESS</span><strong>{plan?.tasks.filter(task=>task.status==='passed').length||0}<em> / {plan?.tasks.length||draft.tasks.length}</em></strong><small>{plan?.tasks.filter(task=>task.status==='running').length||0} currently running</small></div>
      <div><span>MANAGED TOKENS</span><strong>{formatTokens((plan?.totalInputTokens||0)+(plan?.totalOutputTokens||0))}</strong><small>{formatTokens(plan?.totalInputTokens||0)} in · {formatTokens(plan?.totalOutputTokens||0)} out</small></div>
      <div><span>EXECUTION MODE</span><strong>Serial</strong><small>One active task · shared workspace safety</small></div>
    </section>

    <div className="autopilot-grid">
      <section className="panel plan-editor"><div className="panel-head"><div><span className="section-index">01</span><h2>Plan and dependency map</h2></div><span className="panel-tag">{dirty?'UNSAVED':'PERSISTED'}</span></div>
        <div className="plan-form"><label><span>Plan title</span><input value={draft.title} disabled={!actions.save} onChange={event=>updateDraft(value=>({...value,title:event.target.value}))} placeholder="Example: Add offline project search"/></label><label><span>Workspace path</span><input value={draft.workspace} disabled={!actions.save} onChange={event=>updateDraft(value=>({...value,workspace:event.target.value}))} placeholder="C:\\path\\to\\repository"/></label><label className="wide"><span>Requirement and acceptance direction</span><textarea value={draft.objective} disabled={!actions.save} onChange={event=>updateDraft(value=>({...value,objective:event.target.value}))} placeholder="Describe the outcome, constraints, acceptance checks, and anything requiring human direction."/></label></div>
        <div className="task-toolbar"><div><b>Tasks</b><span>Dependencies are task IDs separated by commas.</span></div><div><button className="secondary-button" disabled={!actions.save} onClick={useTemplate}><GitBranch size={14}/> Apply Software Team template</button><button className="secondary-button" disabled={!actions.save} onClick={()=>updateDraft(value=>({...value,tasks:[...value.tasks,{id:`task-${value.tasks.length+1}`,title:'',description:'',role:'coder',dependencies:[]}]}))}><Plus size={14}/> Add task</button></div></div>
        <div className="task-editor-list">{draft.tasks.length?draft.tasks.map((task,index)=><div className="task-editor-row" key={index}><div className="task-number">{index+1}</div><div className="task-fields"><div className="task-fields-top"><input aria-label={`Task ${index+1} ID`} value={task.id} disabled={!actions.save} onChange={event=>updateTask(index,{id:event.target.value.trim().replace(/\s+/g,'-')})} placeholder="task-id"/><select aria-label={`Task ${index+1} role`} value={task.role} disabled={!actions.save} onChange={event=>updateTask(index,{role:event.target.value as TaskRole})}>{taskRoles.map(role=><option value={role} key={role}>{roleLabel[role as TaskRole]}</option>)}</select><input className="task-title-input" aria-label={`Task ${index+1} title`} value={task.title} disabled={!actions.save} onChange={event=>updateTask(index,{title:event.target.value})} placeholder="Task title"/><button className="icon-task-button" aria-label={`Remove ${task.id}`} disabled={!actions.save} onClick={()=>updateDraft(value=>({...value,tasks:value.tasks.filter((_,i)=>i!==index)}))}><Trash2 size={15}/></button></div><textarea aria-label={`Task ${index+1} instructions`} value={task.description} disabled={!actions.save} onChange={event=>updateTask(index,{description:event.target.value})} placeholder="Exact instructions and expected evidence"/><DependencyEditor task={task} disabled={!actions.save} onCommit={dependencies=>updateTask(index,{dependencies})}/></div></div>):<div className="empty-plan">No task map yet. Apply the Software Team template or add tasks manually.</div>}</div>
        <div className="limits"><label><span>Attempts / task</span><input type="number" min="1" max="5" value={draft.limits.maxAttempts} disabled={!actions.save} onChange={event=>updateDraft(value=>({...value,limits:{...value.limits,maxAttempts:Number(event.target.value)}}))}/></label><label><span>Runtime minutes</span><input type="number" min="1" value={draft.limits.maxRuntimeMinutes} disabled={!actions.save} onChange={event=>updateDraft(value=>({...value,limits:{...value.limits,maxRuntimeMinutes:Number(event.target.value)}}))}/></label><label><span>Token ceiling</span><input type="number" min="1" value={draft.limits.maxTokens} disabled={!actions.save} onChange={event=>updateDraft(value=>({...value,limits:{...value.limits,maxTokens:Number(event.target.value)}}))}/></label><label><span>Parallel tasks</span><input type="number" value="1" disabled/><small>Serial is enforced in V2.</small></label></div>
        {validation.length>0&&<details className="validation"><summary>{validation.length} plan issue{validation.length>1?'s':''}</summary>{validation.map(item=><div key={item}>{item}</div>)}</details>}
        <div className="plan-actions"><button className="secondary-button" disabled={!actions.save||validation.length>0||!dirty} onClick={save}><Save size={15}/> Save draft</button><button className="approve-button" disabled={!actions.approve||dirty} onClick={()=>simplePlanAction('approve_autopilot_plan','Approving',{revision:plan?.revision})}><ShieldCheck size={15}/> Approve revision</button><button className="start-button" disabled={!actions.start||dirty||!snapshot?.executionAvailable} onClick={()=>simplePlanAction('start_autopilot','Starting',{revision:plan?.revision})}><CirclePlay size={16}/> Start Autopilot</button>{busy&&<span className="busy-label"><LoaderCircle size={14}/> {busy}…</span>}</div>
      </section>

      <aside className="autopilot-side">
        <section className="panel run-controls"><div className="panel-head"><div><span className="section-index">02</span><h2>Run controls</h2></div></div><p>Pause stops new dispatch after the active task finishes. Cancel requests controller shutdown; the displayed status changes only after backend confirmation. Human gates always wait for an explicit response.</p><div><button disabled={!actions.pause} onClick={()=>simplePlanAction('pause_autopilot','Pausing')}><CirclePause size={15}/> Pause after task</button><button disabled={!actions.resume} onClick={()=>simplePlanAction('resume_autopilot','Resuming')}><CirclePlay size={15}/> Resume</button><button className="danger" disabled={!actions.cancel} onClick={()=>simplePlanAction('cancel_autopilot','Cancelling')}><Square size={14}/> Request cancel</button></div></section>
        <section className="panel human-queue"><div className="panel-head"><div><span className="section-index">03</span><h2>Human direction</h2></div><span className="queue-count">{structuredGateTasks.length+protocolRequests.length}</span></div>{cancelling&&<div className="cancelling-note"><LoaderCircle size={14}/><span>Cancellation requested. Responses are locked until the active Codex process confirms it has stopped.</span></div>}{protocolRequests.map(({task,request,key})=><HumanRequestCard key={key} task={task} request={request} busy={responsesDisabled} onRespond={respondRequest} onError={setError}/>)}{structuredGateTasks.map(task=>{const responseKey=gateResponseKey(task);return <div className="gate-card" key={responseKey}><b>{task.title}</b><span className="gate-meta">{roleLabel[task.role]} · task {task.id} · thread {task.threadId||'not provided'}</span><pre>{task.summary||task.description}</pre>{(snapshot?.events||[]).filter(event=>event.taskId===task.id).slice(-3).map(event=><div className="gate-evidence" key={event.id}><b>{event.kind}</b><span>{event.message}</span></div>)}<textarea disabled={responsesDisabled} value={gateResponses[responseKey]||''} onChange={event=>setGateResponses(values=>({...values,[responseKey]:event.target.value}))} placeholder={task.role==='human'?'Final review decision and evidence':'Direction or corrected instructions for this agent'}/><div>{structuredGateActions(task.role).map(item=><button key={item.action} disabled={responsesDisabled} onClick={()=>respondGate(task,item.action)}>{item.action==='approve'?<Check size={14}/>:item.action==='reject'?<Square size={13}/>:item.action==='continue'?<Send size={14}/>:<RotateCcw size={14}/>} {item.label}</button>)}</div></div>})}{!structuredGateTasks.length&&!protocolRequests.length&&<div className="empty-queue"><ShieldCheck size={20}/><span>No task is waiting for human direction.</span></div>}</section>
      </aside>
    </div>

    <section className="panel managed-work"><div className="panel-head"><div><span className="section-index">04</span><h2>Managed task evidence</h2></div><span className="panel-tag">CONTROLLER STATE</span></div><div className="managed-task-grid">{plan?.tasks.length?plan.tasks.map(task=><article className={`managed-task ${task.status||'pending'}`} key={task.id}><div className="managed-task-head"><span className="managed-role">{roleLabel[task.role]}</span><span className={`managed-status ${task.status||'pending'}`}>{statusLabel[task.status||'pending']||task.status}</span></div><h3>{task.title}</h3><p>{task.summary||task.description}</p><dl><div><dt>Depends on</dt><dd>{task.dependencies.join(', ')||'Start'}</dd></div><div><dt>Attempts</dt><dd>{task.attempts||0} / {plan.limits.maxAttempts}</dd></div><div><dt>Runtime</dt><dd>{formatDuration(runtimeSeconds(task,now))}</dd></div><div><dt>Tokens</dt><dd>{formatTokens((task.inputTokens||0)+(task.outputTokens||0))}</dd></div><div><dt>Thread</dt><dd title={task.threadId}>{task.threadId||'Not dispatched'}</dd></div><div><dt>Model</dt><dd>{task.model||'Role setting'}</dd></div></dl></article>):<div className="empty-plan">Save a dependency map to see controller managed task state here.</div>}</div></section>

    <section className="panel managed-events"><div className="panel-head"><div><span className="section-index">05</span><h2>Managed event log</h2></div><span className="panel-tag">AUTO REFRESH 1S</span></div><div className="managed-event-list">{snapshot?.events.length?snapshot.events.slice().reverse().map(event=><div className="managed-event" key={event.id}><time>{formatEventTime(event.at)}</time><span>{event.taskId||'controller'}</span><b title={event.kind}>{activityKindLabel(event.kind)}</b><p>{event.message}</p></div>):<div className="empty-plan">No managed execution events yet. Monitor events are intentionally kept separate.</div>}</div></section>
  </div>
}
