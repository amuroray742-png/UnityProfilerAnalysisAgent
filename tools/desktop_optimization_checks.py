"""Public release workflow: real diagnosis -> real project diagnosis -> new modification sessions.
Native dialogs are substituted; all reports, file edits, checks and A/B use production IPC.
Synthetic B is a statistical regression fixture, not a claim of measured game improvement.
"""
import base64
import hashlib
import json
import time
import uuid
from desktop_memory_ui import prepare_workload


def run_saved_checks(js,request,root,output,report,records):
    assert records.resolve().is_relative_to((root/'.cache/desktop-optimization').resolve())
    data=json.loads((records/'optimization.json').read_text(encoding='utf-8'))
    assert len(data['rounds'][-1]['runs'])==3
    assert all(r['status']=='rolled_back' for r in data['rounds'][-1]['runs'])
    prepare_workload(js,request,records)
    js('''const nativeFetch=window.fetch.bind(window),nativePost=window.chrome.webview.postMessage.bind(window.chrome.webview);
      window.fetch=(url,opts)=>typeof url==='string'&&new URL(url).hostname==='ipc.localhost'&&decodeURIComponent(new URL(url).pathname)==='/plugin:dialog|save'?Promise.resolve(new Response(JSON.stringify(window.__savePath),{headers:{'Content-Type':'application/json','Tauri-Response':'ok'}})):nativeFetch(url,opts);
      window.chrome.webview.postMessage=raw=>{let m=typeof raw==='string'?JSON.parse(raw):raw;if(m.cmd==='plugin:dialog|save'){window.__TAURI_INTERNALS__.runCallback(m.callback,window.__savePath);return;}return nativePost(raw);};''')
    def button(label):
        node=request('POST','/element',{'using':'xpath','value':'//button[normalize-space(.)='+json.dumps(label,ensure_ascii=False)+']'})
        request('POST','/element/'+node['element-6066-11e4-a52e-4f735466cecf']+'/click',{})
    def wait(script):
        until=time.monotonic()+20
        while not js(script):
            error=js('return document.querySelector(".error-banner")?.innerText')
            assert not error,error
            assert time.monotonic()<until,script
            time.sleep(.2)
    button('打开优化项目 · 修改与 A/B 复验');button('打开优化项目')
    wait('return document.body.innerText.includes("已回退")')
    button('查看完整修改记录')
    wait('return !!document.querySelector(".optimization-detail")')
    assert js('return document.querySelector(".optimization-detail").textContent.length')<=12000
    for label,ext in [('导出 Markdown','md'),('导出 HTML','html')]:
        path=output/('final-optimization.'+ext);js('window.__savePath=arguments[0]',str(path));button(label)
        until=time.monotonic()+20
        while not path.exists():
            assert time.monotonic()<until
            time.sleep(.2)
        text=path.read_text(encoding='utf-8');assert 'Public.AllocationWork' in text and 'claude-code' in text and 'codex' in text and 'rolled_back' in text
        assert 'Some(' not in text
    report['checks'].append('saved public project reopens, rolled-back runs and paged detail render, actual UI Markdown/HTML final exports include three fresh sessions')
    report['scope']='Saved public optimization UI and exports; only native dialogs substituted; no Agent or project writes'
    (output/'optimization.png').write_bytes(base64.b64decode(request('GET','/screenshot')))
    button('关闭项目')


def run_optimization_checks(js, request, root, output, report, agent_id):
    request('POST','/timeouts',{'script':960000})
    project=root/'.cache/unity-project-public'
    assert (project/'.upaa-public-fixture').is_file()
    target=project/'Assets/AllocationWork.cs'
    original=target.read_bytes()
    capture=json.loads((root/'src-tauri/tests/fixtures/isolated-peak.json').read_text(encoding='utf-8'))
    for frame in capture['frames']:
        for thread in frame['threads']:
            for sample in thread['samples']:
                if sample['marker_name']=='Update':sample['marker_name']='AllocationWork.Update'
                elif sample['marker_name']=='Work':sample['marker_name']='Unmapped.Native'
    a=output/'public-A.json';a.write_text(json.dumps(capture),encoding='utf-8')
    prepare_workload(js,request,a)
    def click(selector,using='css selector'):
        node=request('POST','/element',{'using':using,'value':selector});request('POST','/element/'+node['element-6066-11e4-a52e-4f735466cecf']+'/click',{})
    def button(label):click('//button[normalize-space(.)='+json.dumps(label,ensure_ascii=False)+']','xpath')
    def wait(predicate,timeout=960):
        end=time.monotonic()+timeout
        while not js(predicate):
            error=js('return document.querySelector(".error-banner")?.innerText')
            if error:raise AssertionError(error)
            if time.monotonic()>end:raise TimeoutError(predicate)
            time.sleep(.3)
    def ipc(command,args):
        value=request('POST','/execute/async',{'script':"const done=arguments[arguments.length-1];window.__TAURI_INTERNALS__.invoke(arguments[0],arguments[1]).then(v=>done({value:v})).catch(e=>done({error:String(e)}));",'args':[command,args]})
        if 'error' in value:raise AssertionError(value['error'])
        return value['value']
    def action(**value):return ipc('optimization_command',{'action':value})
    click('.dropzone');wait('return document.body.innerText.includes("已就绪，等待 AI 诊断")',30)
    click(f'.agent-selector select option[value="{agent_id}"]');button('开始 AI 诊断')
    print('Optimization acceptance: real performance diagnosis',flush=True)
    wait('return !![...document.querySelectorAll("button")].find(e=>e.textContent==="开始工程联合定位")')
    js('window.__memoryInput=arguments[0]',str(project));button('选择目录');button('开始工程联合定位')
    print('Optimization acceptance: real project diagnosis and task drafts',flush=True)
    wait('return document.querySelectorAll(".diagnosis-content").length===2 && document.body.innerText.includes("诊断完成")')
    (output/'localization.txt').write_text(js('return document.querySelectorAll(".diagnosis-content")[1].innerText'),encoding='utf-8')
    button('打开优化项目 · 修改与 A/B 复验')
    records=output/('records-'+str(uuid.uuid4()));records.mkdir()
    # Native directory selection only; commands and state remain real.
    js('window.__optPickers=arguments[0]',[str(project),str(records)])
    button('新建优化项目');wait('return document.body.innerText.includes("将当前定位保存为一轮")',15)
    button('将当前定位保存为一轮');wait('return document.body.innerText.includes("本轮任务与授权范围")',20)
    data=action(op='get');round_id=data['rounds'][-1]['id'];tasks=data['rounds'][-1]['tasks']
    candidates=[t for t in tasks if t['kind']=='optimize' and 'Assets/AllocationWork.cs' in t['files']]
    assert candidates,'Agent did not supply an executable, evidence-backed AllocationWork task'
    selected=candidates[0]
    selected['files']={'Assets/AllocationWork.cs':selected['files']['Assets/AllocationWork.cs']}
    for task in tasks:task['selected']=task['id']==selected['id']
    selected['instructions']+='\n公开验收约束：复用 lastFrame，保持长度 8*1024*1024、lastFrame[0]=1；不修改其他字段或文件。'
    condition={k:'public fixture' for k in ['device','platform','scenario','operation','build','quality','resolution','profiling','codeVersion']}
    baseline=data['rounds'][-1]['baseline']
    action(op='update',roundId=round_id,tasks=tasks,budgets={'gcBytes':1024},conditions={baseline:condition},tests=['PublicOptimizationTests.AllocationWorkPreservesVisibleResult'])
    sessions=[]
    try:
        for modification_agent in [agent_id,'codex' if agent_id!='codex' else 'claude-code']:
            print('Optimization acceptance: modification Agent '+modification_agent,flush=True)
            action(op='start',roundId=round_id,agentId=modification_agent)
            end=time.monotonic()+960
            while True:
                data=action(op='get')
                if not data['busy']:break
                if time.monotonic()>end:raise TimeoutError('modification')
                time.sleep(.6)
            run=data['rounds'][-1]['runs'][-1];sessions.append(run['sessionId'])
            assert run['status']=='modified',run
            assert run['changes'] and target.read_bytes()!=original
            assert run['checks'][-1]['status']=='passed',run['checks']
            assert run['agentId']==modification_agent
            report.setdefault('modificationRuns',[]).append(run)
            if len(sessions)==1:
                b=output/'public-B-synthetic.json';b.write_text(a.read_text(encoding='utf-8'),encoding='utf-8')
                data=action(op='import',path=str(b),conditions=condition);candidate=data['captures'][-1]['id']
                data=action(op='compare',roundId=round_id,candidateId=candidate,rangeA=None,rangeB=None,confirmed=True)
                comparison=data['rounds'][-1]['comparison'];assert comparison['metrics'][0]['delta']['p95']['absolute']==0
                assert comparison['hotspots']['rows']
                for fmt,extension in [('markdown','md'),('html','html')]:
                    path=output/('optimization.'+extension);action(op='export',path=str(path),format=fmt)
                    text=path.read_text(encoding='utf-8');assert '优化记录' in text and 'Assets/AllocationWork.cs' in text
                    if fmt=='html':assert "default-src 'none'" in text and '<script' not in text
            action(op='rollback',roundId=round_id);assert target.read_bytes()==original
        assert len(set(sessions))==2
        print('Optimization acceptance: pure Marker task in another fresh session',flush=True)
        selected.update(kind='marker',title='公开 Marker 验收',instructions='在现有 AllocationWork.cs 中增加一个静态 Unity.Profiling.ProfilerMarker，固定名称 Public.AllocationWork。用 Auto() 同步作用域仅包住 Update 内已有的分配与写入，不改变原有行为，不缓存数组，不新增文件。',constraints='只改 Assets/AllocationWork.cs，保留每次分配 8 MiB 和 lastFrame[0]=1，不改测试。',acceptance='编译及公开行为测试通过；重录后人工检查新 Marker。本测试的合成 B 仅验证对比呈现，不证明真实采样有效。')
        action(op='update',roundId=round_id,tasks=tasks,budgets={'gcBytes':1024},conditions={},tests=['PublicOptimizationTests.AllocationWorkPreservesVisibleResult'])
        action(op='start',roundId=round_id,agentId=agent_id)
        end=time.monotonic()+960
        while True:
            data=action(op='get')
            if not data['busy']:break
            if time.monotonic()>end:raise TimeoutError('marker modification')
            time.sleep(.6)
        marker_run=data['rounds'][-1]['runs'][-1];assert marker_run['status']=='modified',marker_run
        assert 'Public.AllocationWork' in target.read_text(encoding='utf-8')
        assert marker_run['checks'][-1]['status']=='passed',marker_run['checks']
        assert marker_run['sessionId'] not in sessions
        report['markerRun']=marker_run
        data=action(op='compare',roundId=round_id,candidateId=candidate,rangeA=None,rangeB=None,confirmed=True)
        assert '纯 Marker' in data['rounds'][-1]['comparison']['taskOutcome']
        assert data['rounds'][-1]['taskVerifications']=={}
        action(op='rollback',roundId=round_id);assert target.read_bytes()==original
        action(op='close');data=action(op='open',directory=str(records));assert len(data['rounds'][-1]['runs'])==3
        report['checks'].append('real diagnosis/localization/task draft, same/different modification Agents in fresh sessions, actual edits, Unity compile/EditMode pass, synthetic A/B, exports, exact rollback, persisted reopen')
        report['comparisonBoundary']='B is an unchanged public synthetic fixture; validates A/B mechanics, not measured optimization benefit.'
        (output/'optimization.png').write_bytes(base64.b64decode(request('GET','/screenshot')))
        action(op='close')
    finally:
        # Never overwrite unexpected contents. Backend rollback is the only restoration path.
        if target.read_bytes()!=original:
            try:action(op='cancel');action(op='rollback',roundId=round_id)
            except Exception as error:report['rollbackFailure']=str(error)
        assert target.read_bytes()==original,'Public test project has outstanding changes; inspect records before further work'
