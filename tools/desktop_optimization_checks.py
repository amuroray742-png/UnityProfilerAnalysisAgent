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
    data=json.loads((records/'optimization.json').read_text(encoding='utf-8'))
    from pathlib import Path
    import os
    assert os.path.samefile(Path(data['root']),root/'.cache/unity-project-public')
    assert (Path(data['root'])/'.upaa-public-fixture').is_file()
    assert data['rounds'][-1]['runs']
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
    button('打开优化项目 · 修改与 A/B 复验')
    node=request('POST','/element',{'using':'xpath','value':'//summary[normalize-space(.)="打开或管理优化记录"]'})
    request('POST','/element/'+node['element-6066-11e4-a52e-4f735466cecf']+'/click',{})
    button('打开优化项目')
    wait('return document.body.innerText.includes("已回退")')
    node=request('POST','/element',{'using':'xpath','value':'//summary[contains(.,"查看详细记录")]'})
    request('POST','/element/'+node['element-6066-11e4-a52e-4f735466cecf']+'/click',{})
    button('查看完整修改记录')
    wait('return !!document.querySelector(".optimization-detail")')
    assert js('return document.querySelector(".optimization-detail").textContent.length')<=12000
    for label,ext in [('导出 Markdown','md'),('导出 HTML','html')]:
        path=output/('final-optimization-'+uuid.uuid4().hex+'.'+ext);report.setdefault('exports',[]).append(str(path));js('window.__savePath=arguments[0]',str(path));button(label)
        until=time.monotonic()+20
        while not path.exists():
            assert time.monotonic()<until
            time.sleep(.2)
        text=path.read_text(encoding='utf-8');assert 'rolled_back' in text and all(run['agentId'] in text for run in data['rounds'][-1]['runs'])
        assert 'Some(' not in text
    report['checks'].append('saved public project reopens, rolled-back runs and paged detail render, actual UI Markdown/HTML final exports include saved session records')
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
    # The report automatically reveals one-click controls; no task, file or save-folder approval.
    assert js('return !![...document.querySelectorAll("button")].find(e=>e.textContent==="开始优化")')
    assert not js('return document.body.innerText.includes("本轮任务与授权范围")')
    requirement="公开验收仅处理 AllocationWork 的数组分配：新增 Assets/DesktopBufferCache.cs 提供可复用的 8*1024*1024 字节数组，修改已有 AllocationWork.cs 使用它，保留 lastFrame 字段和 lastFrame[0]=1。不要修改其他现有文件、测试或资源。其余热点仅调查说明。"
    field=request('POST','/element',{'using':'xpath','value':'//label[contains(.,"补充要求")]/textarea'})
    request('POST','/element/'+field['element-6066-11e4-a52e-4f735466cecf']+'/value',{'text':requirement})
    button('开始优化')
    print('Automatic acceptance: actual one-click start, no task selection',flush=True)
    wait('return document.body.innerText.includes("本轮结果：已修改")')
    data=action(op='get');round_id=data['rounds'][-1]['id'];run=data['rounds'][-1]['runs'][-1]
    records=data['directory'];report['recordDirectory']=records
    try:
        assert run['automatic'] and run['status']=='modified',run
        assert run['checks'][-1]['status']=='passed',run['checks']
        assert (project/'Assets/DesktopBufferCache.cs').is_file()
        assert target.read_bytes()!=original
        report['modificationRun']=run
        assert not js('return [...document.querySelectorAll(".optimization details")].filter(e=>e.querySelector("summary")?.textContent.includes("查看详细记录")).some(e=>e.open)')
        js("""const nativeFetch=window.fetch.bind(window),nativePost=window.chrome.webview.postMessage.bind(window.chrome.webview);
          window.fetch=(url,opts)=>typeof url==='string'&&new URL(url).hostname==='ipc.localhost'&&decodeURIComponent(new URL(url).pathname)==='/plugin:dialog|save'?Promise.resolve(new Response(JSON.stringify(window.__savePath),{headers:{'Content-Type':'application/json','Tauri-Response':'ok'}})):nativeFetch(url,opts);
          window.chrome.webview.postMessage=raw=>{let m=typeof raw==='string'?JSON.parse(raw):raw;if(m.cmd==='plugin:dialog|save'){window.__TAURI_INTERNALS__.runCallback(m.callback,window.__savePath);return;}return nativePost(raw);};""")
        for label,extension in [('导出 Markdown','md'),('导出 HTML','html')]:
            path=output/('automatic-'+uuid.uuid4().hex+'.'+extension);report.setdefault('exports',[]).append(str(path));js('window.__savePath=arguments[0]',str(path));button(label)
            until=time.monotonic()+20
            while not path.exists():
                assert time.monotonic()<until
                time.sleep(.2)
            text=path.read_text(encoding='utf-8');assert 'Assets/DesktopBufferCache.cs' in text and 'create' in text
        (output/'automatic.png').write_bytes(base64.b64decode(request('GET','/screenshot')))
        button('撤销本轮代码修改');wait('return document.body.innerText.includes("本轮结果：已回退")')
        assert target.read_bytes()==original
        assert not (project/'Assets/DesktopBufferCache.cs').exists()
        assert not (project/'Assets/DesktopBufferCache.cs.meta').exists()
        action(op='close');data=action(op='open',directory=records)
        assert data['rounds'][-1]['runs'][-1]['status']=='rolled_back'
        report['checks'].append('actual one-click UI -> automatic persistent records -> independent fresh Agent -> real existing edit + new helper + Unity check -> Markdown/HTML -> UI exact rollback -> reopen')
        action(op='close')
    finally:
        if target.read_bytes()!=original or (project/'Assets/DesktopBufferCache.cs').exists():
            try:action(op='cancel');action(op='rollback',roundId=round_id)
            except Exception as error:report['rollbackFailure']=str(error)
        assert target.read_bytes()==original,'Public test changes require recovery from saved records'
