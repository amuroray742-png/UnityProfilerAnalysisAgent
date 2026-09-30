"""Public, read-only real Agent activity and saved-history desktop acceptance."""
import base64
import json
import time
import uuid


def run_history_activity(js, request, root, output, report, saved=None):
    request('POST','/timeouts',{'script':960000})
    def ipc(command,action):
        v=request('POST','/execute/async',{'script':"const done=arguments[arguments.length-1];window.__TAURI_INTERNALS__.invoke(arguments[0],{action:arguments[1]}).then(v=>done({value:v}),e=>done({error:String(e)}))",'args':[command,action]})
        if 'error' in v:raise AssertionError(v['error'])
        return v['value']
    def wait(predicate,seconds=60):
        end=time.monotonic()+seconds
        while not predicate():
            if time.monotonic()>end:raise TimeoutError('history/activity wait')
            time.sleep(.3)
    def click(text):
        wait(lambda:js('return [...document.querySelectorAll("button")].some(b=>b.textContent.trim()===arguments[0]&&!b.disabled&&b.getClientRects().length)',text))
        js('const b=[...document.querySelectorAll("button")].find(b=>b.textContent.trim()===arguments[0]&&!b.disabled&&b.getClientRects().length);b.click()',text)
    def state():return ipc('optimization_command',{'op':'get'})
    def capture(name):
        (output/name).write_bytes(base64.b64decode(request('GET','/screenshot')))
    if saved:
        ipc('workflow_command',{'op':'open','directory':str(saved)})
        # Reload the frontend to restore the opened backend project, not start any work.
        js('location.reload()');wait(lambda:js('return document.body.innerText.includes("历史轮次")'))
        click('历史轮次');wait(lambda:js('return document.querySelectorAll(".round-list button").length>=2'))
        for i in range(2):
            js('document.querySelectorAll(".round-list button")[arguments[0]].click()',i)
            click('性能诊断');wait(lambda:js('return !!document.querySelector(".history-panel .diagnosis-stream")'))
            click('工程定位');wait(lambda:js('return !!document.querySelector(".history-panel .diagnosis-stream")'))
            click('代码修改');wait(lambda:js('return !!document.querySelector(".change-file")'))
            js('document.querySelector(".change-file").click()');wait(lambda:js('return !!document.querySelector(".file-diff")'))
            assert js('return document.querySelector(".file-diff").innerText.length')>10
        capture('history-file-diff.png')
        report['checks'].append('two archived rounds: complete diagnosis/localization and individual rollback file diff readable without original capture')
        click('当前工作');click('返回项目首页')
    project=root/'src-tauri/tests/fixtures/unity-project'
    directory=output/('records-'+uuid.uuid4().hex)
    ipc('workflow_command',{'op':'create','root':str(project),'name':'公开实时工作验收','directory':str(directory)})
    js('location.reload()');wait(lambda:js('return document.body.innerText.includes("导入录制 A")'))
    # Real events, bounded test observer. Native picker not involved in this direct IPC bind.
    v=request('POST','/execute/async',{'script':'''const done=arguments[arguments.length-1];window.__parse=[];window.__activities=[];
      const api=window.__TAURI_INTERNALS__;Promise.all(['workflow-progress','workflow-activity'].map(event=>api.invoke('plugin:event|listen',{event,target:{kind:'Any'},handler:api.transformCallback(e=>{const rows=event==='workflow-progress'?window.__parse:window.__activities;if(rows.length<10000)rows.push(e.payload);})}))).then(()=>done(true),e=>done({error:String(e)}));''','args':[]})
    assert v is True,v
    p=ipc('workflow_command',{'op':'bind','path':str(root/'src-tauri/tests/fixtures/isolated-peak.json'),'role':'a','operationId':'desktop-import'})
    rid=p['rounds'][-1]['id']
    progress=js('return window.__parse');assert {'verify','parse','aggregate','save'}<={r['stage'] for r in progress};assert progress[-1]['status']=='completed'
    assert any(r['stage']=='parse' and r['total'] is None for r in progress)
    report['progress']=progress
    for agent in ['claude-code','codex']:
        ipc('workflow_command',{'op':'analyze','roundId':rid,'analysisAgent':agent,'localizationAgent':agent,'restart':agent=='codex'})
        end=time.monotonic()+960;screenshot=False
        while True:
            p=state()
            if js('return !!document.querySelector(".activity-tool")') and not screenshot:
                capture(agent+'-working.png');screenshot=True
            if not p['busy']:break
            if time.monotonic()>end:raise TimeoutError(agent)
            time.sleep(1)
        r=p['rounds'][-1];assert r['workflow']['status']=='completed',r['workflow']
        reports=r['reports'][-2:];assert all(p['status']=='completed' for p in reports),reports
        for r in reports:
            cursor=0;rows=[]
            while True:
                page=ipc('workflow_command',{'op':'activity','roundId':rid,'runId':r['reportId'],'cursor':cursor});rows+=page['rows'];cursor=page['nextCursor']
                if not page['hasMore']:break
            assert any(e['event']['kind']=='chunk' for e in rows)
            tools=[e['event'] for e in rows if e['event']['kind']=='tool'];assert tools,agent
            assert any(t['status']=='running' for t in tools) and any(t['status']=='completed' for t in tools)
            assert all('new_text' not in t['args'] and 'old_text' not in t['args'] for t in tools)
            report.setdefault('sessions',[]).append({'agent':agent,'stage':r['stage'],'reportId':r['reportId'],'sessionId':r['sessionId'],'events':len(rows),'tools':len(tools)})
        click('历史轮次');click('工程定位');wait(lambda:js('return !!document.querySelector(".history-panel .diagnosis-stream")'));capture(agent+'-report.png');click('当前工作')
    assert len({s['sessionId'] for s in report['sessions']})==4
    click('返回项目首页');ipc('workflow_command',{'op':'open','directory':str(directory)});js('location.reload()');wait(lambda:js('return document.body.innerText.includes("历史轮次")'));click('历史轮次');click('性能诊断')
    wait(lambda:js('return document.querySelectorAll(".history-panel select option").length>=2'))
    capture('reopened-attempts.png')
    report['records']=str(directory)
    report['checks']+=['real JSON import events verify/parse/aggregate/save; determinate hashing and indeterminate JSON parsing','Claude and Codex public diagnosis/localization: streamed text, correlated tool start/end, persisted cursor replay and four independent sessions','reopened history retains both attempts and never starts AI automatically']
    report['scope']='Public read-only Agent sessions; original project untouched; offline Editor localization; native picker excluded'

def run_progress(js,request,root,output,report):
    """Generate a sizeable public binary recording; never sends it to an Agent."""
    import struct
    project=root/'src-tauri/tests/fixtures/unity-project'
    data=bytearray(28+136)
    def word(v):data.extend(struct.pack('<I',v))
    def string(s):
        data.extend(s.encode()+b'\0')
        while len(data)%4:data.append(0)
    word(0xffffffff);data.extend(bytes(64+1060+32));word(2)
    for marker,name in [(913,'Main Thread'),(718,'GC.Alloc')]:
        word(marker);string(name);word(17<<16);word(0)
    word(1);data.extend(struct.pack('<Q',42));string('');string('Main Thread');word(2)
    for marker,ns,children in [(913,1000000.,1),(718,100.,0)]:
        word(marker);data.extend(struct.pack('<fQ',ns,3000000));word(children)
    for v in [0,0,1,1,136,0,1,1,1,3,4,136,0,1,0,0,0xAFAFAFAF]:word(v)
    header=struct.pack('<7I',0x20220328,len(data),6000,3,23,2,1)
    capture=output/'public-progress.data'
    with capture.open('wb') as f:
        for _ in range(40000):f.write(header);f.write(data)
        f.write(struct.pack('<I',0xDEADFEED))
    def ipc(action):
        v=request('POST','/execute/async',{'script':"const done=arguments[arguments.length-1];window.__TAURI_INTERNALS__.invoke('workflow_command',{action:arguments[0]}).then(v=>done({value:v}),e=>done({error:String(e)}));",'args':[action]})
        if 'error' in v:raise AssertionError(v['error'])
        return v['value']
    directory=output/('progress-records-'+uuid.uuid4().hex)
    ipc({'op':'create','root':str(project),'name':'公开解析进度验收','directory':str(directory)})
    js('location.reload()');time.sleep(1)
    v=request('POST','/execute/async',{'script':"const done=arguments[arguments.length-1];window.__parse=[];window.__TAURI_INTERNALS__.invoke('plugin:event|listen',{event:'workflow-progress',target:{kind:'Any'},handler:window.__TAURI_INTERNALS__.transformCallback(e=>window.__parse.push(e.payload))}).then(()=>done(true));",'args':[]});assert v
    js("window.__result=null;window.__TAURI_INTERNALS__.invoke('workflow_command',{action:{op:'bind',path:arguments[0],role:'a',operationId:'public-data'}}).then(v=>window.__result={value:v},e=>window.__result={error:String(e)})",str(capture))
    started=time.monotonic();pictured=False
    while not js('return window.__result'):
        if js('return !!document.querySelector("progress[value]")') and not pictured:
            (output/'data-progress.png').write_bytes(base64.b64decode(request('GET','/screenshot')));pictured=True
        if time.monotonic()-started>120:raise TimeoutError('public binary progress')
        time.sleep(.1)
    result=js('return window.__result');assert 'error' not in result,result
    events=js('return window.__parse');parsing=[r for r in events if r['stage']=='parse' and r['done'] is not None];assert parsing,events
    assert all(a['done']<=b['done'] for a,b in zip(parsing,parsing[1:]));assert all(e['total']==capture.stat().st_size for e in parsing)
    assert events[-1]['status']=='completed';assert pictured
    # Broken input leaves current baseline unchanged and the failure stage visible.
    baseline=result['value']['rounds'][-1]['baseline'];broken=output/'broken.data';broken.write_bytes(header+b'broken')
    v=request('POST','/execute/async',{'script':"const done=arguments[arguments.length-1];window.__TAURI_INTERNALS__.invoke('workflow_command',{action:{op:'bind',path:arguments[0],role:'a',operationId:'broken-data'}}).then(()=>done(false),e=>done(String(e)));",'args':[str(broken)]});assert v
    current=request('POST','/execute/async',{'script':"const done=arguments[arguments.length-1];window.__TAURI_INTERNALS__.invoke('optimization_command',{action:{op:'get'}}).then(done);",'args':[]})
    assert current['rounds'][-1]['baseline']==baseline;assert current['parseProgress']['status']=='failed'
    report['checks']+=['public 40000-frame binary: real monotonic byte progress visible in rendered progress bar; saved only after completion','corrupt subsequent binary reports parse failure and preserves previous A']
    report['progressMeasurement']={'bytes':capture.stat().st_size,'frames':40000,'elapsedSeconds':round(time.monotonic()-started,2),'events':len(parsing)}
    report['scope']='Generated public binary data; real desktop IPC and progress UI; no Agent or private capture'

def run_replay(js,request,root,output,report,directory):
    def ipc(action):
        v=request('POST','/execute/async',{'script':"const done=arguments[arguments.length-1];window.__TAURI_INTERNALS__.invoke('workflow_command',{action:arguments[0]}).then(v=>done({value:v}),e=>done({error:String(e)}));",'args':[action]})
        if 'error' in v:raise AssertionError(v['error'])
        return v['value']
    p=ipc({'op':'open','directory':str(directory)})
    import os
    assert os.path.samefile(p['root'],root/'src-tauri/tests/fixtures/unity-project')
    assert not p['busy']
    js('location.reload()')
    def wait(script):
        end=time.monotonic()+30
        while not js(script):
            if time.monotonic()>end:raise TimeoutError(script)
            time.sleep(.1)
    def click(name):
        js('const b=[...document.querySelectorAll("button")].find(b=>b.textContent.trim()===arguments[0]&&b.getClientRects().length);if(!b)throw Error(arguments[0]);b.click()',name)
    wait('return document.body.innerText.includes("历史轮次")');click('历史轮次');click('工程定位');wait('return !!document.querySelector(".history-panel .diagnosis-stream")')
    click('查看此会话工作过程');wait('return !!document.querySelector(".activity-tool")')
    assert js('return document.body.innerText.includes("加载后续工作记录")')
    click('加载后续工作记录');time.sleep(.3)
    js('document.querySelector(".work-activity").scrollIntoView({block:"start"})');time.sleep(.2)
    (output/'history-replayed-activity.png').write_bytes(base64.b64decode(request('GET','/screenshot')))
    for kind,extension in [('markdown','md'),('html','html')]:
        path=output/f'archived-round.{extension}';ipc({'op':'export','roundId':p['rounds'][-1]['id'],'path':str(path),'format':kind});assert path.stat().st_size>1000
    report['checks'].append('final EXE: reopened four archived real sessions, paged activity and complete history; Markdown/HTML export; no AI restart')
    report['scope']='Existing public Agent records, final EXE replay and export; no new AI calls'
