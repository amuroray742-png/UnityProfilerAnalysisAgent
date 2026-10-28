import fs from 'node:fs';
import path from 'node:path';
import net from 'node:net';
import {spawn} from 'node:child_process';
import {createHash,randomUUID} from 'node:crypto';
import assert from 'node:assert/strict';

const root=process.cwd();
const app=path.resolve(process.argv[2]??'src-tauri/target/x86_64-pc-windows-msvc/release/unity-profiler-analysis-agent.exe');
const out=path.join(root,'.cache/recent-projects-desktop',randomUUID());
fs.mkdirSync(out,{recursive:true});
const formal=path.join(process.env.APPDATA,'com.ray.unity-profiler-analysis-agent/recent-projects.json');
const before=fs.existsSync(formal)?fs.readFileSync(formal):null;
const isolated=path.join(out,'recent-projects.json');
const report={application:app,sha256:createHash('sha256').update(fs.readFileSync(app)).digest('hex'),output:out,checks:[]};
const pause=ms=>new Promise(r=>setTimeout(r,ms));
async function wait(fn){const end=Date.now()+20000;while(!await fn()){if(Date.now()>end)throw Error('Desktop wait timed out');await pause(100);}}
function unchanged(){assert.deepEqual(fs.existsSync(formal)?fs.readFileSync(formal):null,before,'Formal recent list was changed');}
async function launch(test){
 const port=await new Promise(r=>{const s=net.createServer();s.listen(0,'127.0.0.1',()=>{const p=s.address().port;s.close(()=>r(p));});});
 const env={...process.env,WEBVIEW2_USER_DATA_FOLDER:path.join(out,test?'test-profile':'formal-profile'),WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:'--remote-debugging-port='+port};
 delete env.UPAA_RECENT_PROJECTS_PATH;
 if(test)env.UPAA_RECENT_PROJECTS_PATH=isolated;
 const child=spawn(app,[],{windowsHide:true,stdio:'ignore',env});
 let socket,seq=0,closed=false;const pending=new Map();
 try{
  let tabs;await wait(async()=>{try{tabs=await(await fetch('http://127.0.0.1:'+port+'/json/list')).json();return tabs.some(t=>t.url.includes('tauri.localhost'));}catch{return false;}});
  socket=new WebSocket(tabs.find(t=>t.type==='page'&&t.url.includes('tauri.localhost')).webSocketDebuggerUrl);
  await new Promise((r,j)=>{socket.onopen=r;socket.onerror=j;});
  socket.onmessage=({data})=>{const m=JSON.parse(data);if(!m.id)return;const p=pending.get(m.id);pending.delete(m.id);m.error?p.reject(Error(JSON.stringify(m.error))):p.resolve(m.result);};
  const cdp=(method,params={})=>new Promise((resolve,reject)=>{const id=++seq;pending.set(id,{resolve,reject});socket.send(JSON.stringify({id,method,params}));});
  const evaluate=async expression=>{const r=await cdp('Runtime.evaluate',{expression,awaitPromise:true,returnByValue:true});if(r.exceptionDetails)throw Error(JSON.stringify(r.exceptionDetails));return r.result.value;};
  const ipc=(command,action)=>evaluate('window.__TAURI_INTERNALS__.invoke('+JSON.stringify(command)+',{action:'+JSON.stringify(action)+'})');
  await wait(()=>evaluate('!!window.__TAURI_INTERNALS__'));
  return {ipc,evaluate,cdp,close:async()=>{if(closed)return;closed=true;socket.close();if(child.exitCode===null){const exited=new Promise(r=>child.once('exit',r));child.kill();await exited;}}};
 }catch(e){socket?.close();child.kill();throw e;}
}
let desktop;
try{
 desktop=await launch(true);
 const recent=()=>desktop.ipc('workflow_command',{op:'recent'});
 const close=()=>desktop.ipc('optimization_command',{op:'close'});
 assert.deepEqual(await recent(),[]);unchanged();
 const archive=path.join(out,'archive');
 const project=await desktop.ipc('workflow_command',{op:'create',root:path.join(root,'src-tauri/tests/fixtures/unity-project'),directory:archive,name:'Recent list acceptance'});
 await close();
 const aliases=[archive,archive.replaceAll('\\','/'),archive.toUpperCase(),archive+path.sep,archive+path.sep+'.',archive];
 for(const directory of aliases){
  const opened=await desktop.ipc('workflow_command',{op:'open',directory});
  assert.equal(opened.id,project.id);await close();
  const rows=await recent();assert.equal(rows.length,1);assert.equal(rows[0].id,project.id);unchanged();
 }
 report.checks.push('Six repeat opens including Windows path aliases retain one entry');
 const copy=path.join(out,'copy');
 fs.cpSync(archive,copy,{recursive:true});
 await desktop.ipc('workflow_command',{op:'open',directory:copy});await close();
 let rows=await recent();assert.equal(rows.length,2);assert.equal(rows[0].id,rows[1].id);
 // Seed legacy duplicate rows, then check production read-side repair on disk.
 fs.writeFileSync(isolated,JSON.stringify([rows[0],{...rows[0],directory:copy.toUpperCase()},rows[1],rows[1]]));
 assert.equal((await recent()).length,2);assert.equal(JSON.parse(fs.readFileSync(isolated)).length,2);
 await desktop.evaluate('location.reload()');
 await wait(()=>desktop.evaluate('document.querySelectorAll(".recent-project").length===2'));
 const labels=await desktop.evaluate('Array.from(document.querySelectorAll(".recent-project")).map(b=>b.innerText)');
 assert.ok(labels.every(t=>t.includes('存档：')));assert.notEqual(labels[0],labels[1]);
 await desktop.evaluate('document.querySelectorAll(".recent-project")[1].click()');
 await wait(async()=>{const p=await desktop.ipc('optimization_command',{op:'get'});return p&&p.directory===project.directory;});
 await close();unchanged();
 report.checks.push('Copied IDs have distinct archive labels and open the selected archive; legacy duplicates repaired');
 fs.unlinkSync(path.join(copy,'optimization.json'));
 assert.equal((await recent()).length,1);
 assert.equal(JSON.parse(fs.readFileSync(isolated)).length,1);unchanged();
 report.checks.push('Deleted archive is removed from recent history and cleanup is persisted');
 await desktop.close();desktop=await launch(true);
 assert.equal((await desktop.ipc('workflow_command',{op:'recent'})).length,1);unchanged();
 report.checks.push('Isolated list persists across application restart');
 await desktop.close();desktop=await launch(false);
 // The user may have deleted a real archive since the previous verification.
 // Isolation must preserve formal bytes until this normal production read; that
 // read may legitimately prune confirmed missing archives.
 const original=before?JSON.parse(before):[];
 const expected=original.filter(r=>{
  try{fs.statSync(path.join(r.directory,'optimization.json'));return true;}
  catch(e){if(e.code==='ENOENT')return false;throw e;}
 });
 unchanged();
 const production=await desktop.ipc('workflow_command',{op:'recent'});
 assert.deepEqual(production,expected);
 if(expected.length===original.length)unchanged();
 else assert.deepEqual(JSON.parse(fs.readFileSync(formal)),expected);
 report.productionCount=production.length;
 report.productionNames=production.map(r=>r.name);
 report.productionRemovedMissing=original.filter(r=>!expected.includes(r)).map(r=>({name:r.name,directory:r.directory}));
 report.checks.push('Test runs leave formal bytes unchanged; normal production read only removes confirmed missing archives, with no test entries leaked');
 report.passed=true;
}catch(e){report.passed=false;report.error=String(e);process.exitCode=1;}
finally{if(desktop)await desktop.close();fs.writeFileSync(path.join(out,'verification.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report,null,2));}
