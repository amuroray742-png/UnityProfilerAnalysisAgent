"""Project-first production IPC + rendered UI acceptance, public fixture only.

Native file/save dialogs are substituted. All AI sessions, journals, edits,
checks and rollback use the installed application. B is synthetic regression
data, not a claim that a game was optimized.
"""
import base64
import json
import os
import time
import uuid


def run_workflow(js, request, root, output, report, saved=None, resume=None):
    request('POST', '/timeouts', {'script': 960000})
    project = root / '.cache/unity-project-public'
    assert (project / '.upaa-public-fixture').is_file()
    js('''const f=window.fetch.bind(window),p=window.chrome.webview.postMessage.bind(window.chrome.webview);
      window.__pick=null;window.__save=null;
      window.fetch=(u,o)=>{if(typeof u==='string'&&new URL(u).hostname==='ipc.localhost'){
        const cmd=decodeURIComponent(new URL(u).pathname);if(cmd==='/plugin:dialog|open'||cmd==='/plugin:dialog|save')return Promise.resolve(new Response(JSON.stringify(cmd.endsWith('open')?window.__pick:window.__save),{headers:{'Content-Type':'application/json','Tauri-Response':'ok'}}));}return f(u,o);};
      window.chrome.webview.postMessage=raw=>{const m=typeof raw==='string'?JSON.parse(raw):raw;if(m.cmd==='plugin:dialog|open'||m.cmd==='plugin:dialog|save'){window.__TAURI_INTERNALS__.runCallback(m.callback,m.cmd.endsWith('open')?window.__pick:window.__save);return;}return p(raw);};''')
    def ipc(name, action):
        v=request('POST','/execute/async',{'script':"const done=arguments[arguments.length-1];window.__TAURI_INTERNALS__.invoke(arguments[0],{action:arguments[1]}).then(v=>done({value:v}),e=>done({error:String(e)}));",'args':[name,action]})
        if 'error' in v: raise AssertionError(v['error'])
        return v['value']
    def state(): return ipc('optimization_command',{'op':'get'})
    def click(text, tag='button'):
        if tag=='button':
            wait(lambda:js('return [...document.querySelectorAll("button")].some(b=>b.textContent.trim()===arguments[0]&&!b.disabled)',text),30)
        item=request('POST','/element',{'using':'xpath','value':f'//{tag}[normalize-space(.)={json.dumps(text,ensure_ascii=False)}]'})
        request('POST','/element/'+item['element-6066-11e4-a52e-4f735466cecf']+'/click',{})
    def value(label, text):
        # Set React-controlled text/select using the native setter and input event.
        js('''const labels=[...document.querySelectorAll('label')];const l=labels.find(l=>l.firstChild?.textContent===arguments[0]);if(!l)throw Error('label missing '+arguments[0]);const n=l.querySelector('input,textarea,select');const proto=n.tagName==='SELECT'?HTMLSelectElement.prototype:n.tagName==='TEXTAREA'?HTMLTextAreaElement.prototype:HTMLInputElement.prototype;Object.getOwnPropertyDescriptor(proto,'value').set.call(n,arguments[1]);n.dispatchEvent(new Event(n.tagName==='SELECT'?'change':'input',{bubbles:true}));''', label, text)
    def wait(predicate, timeout=960):
        end=time.monotonic()+timeout
        while not predicate():
            error=js('return document.querySelector(".error-banner")?.innerText')
            if error: raise AssertionError(error)
            if time.monotonic()>end: raise TimeoutError('workflow wait')
            time.sleep(.5)
    def ready():
        p=state()
        return p and not p['busy']
    def text(t):return js('return document.body.innerText.includes(arguments[0])', t)
    def reopen(directory):
        click('返回项目首页');wait(lambda:text('新建优化项目'),20)
        js('window.__pick=arguments[0]',str(directory));click('打开优化项目');wait(lambda:state() is not None,20)
    def exports():
        click('报告与每轮记录','summary')
        for kind,ext in [('Markdown','md'),('HTML','html')]:
            path=output/f'workflow-{uuid.uuid4().hex}.{ext}'
            js('window.__save=arguments[0]',str(path));click('导出本轮 '+kind)
            wait(path.exists,30);assert path.stat().st_size>500
            report.setdefault('exports',[]).append(str(path))
        click('报告与每轮记录','summary')
    if saved:
        js('window.__pick=arguments[0]',str(saved));click('打开优化项目');wait(lambda:state() is not None,20)
        p=state();assert os.path.samefile(p['root'],project)
        assert len(p['rounds'])>=2 and p['version']==3
        exports();report['checks'].append('separate application process reopens v3 rounds and exports reports without AI')
        js('window.scrollTo(0,0)');time.sleep(.3)
        (output/'workflow-saved.png').write_bytes(base64.b64decode(request('GET','/screenshot')))
        return
    capture=json.loads((root/'src-tauri/tests/fixtures/isolated-peak.json').read_text(encoding='utf-8'))
    for frame in capture['frames']:
        for thread in frame['threads']:
            for sample in thread['samples']:
                if sample['marker_name']=='Update':sample['marker_name']='AutomaticHotspots.Update'
    a=output/'A.json';a.write_text(json.dumps(capture),encoding='utf-8')
    b=output/'B.json';b.write_text(json.dumps(capture),encoding='utf-8')
    original=(project/'Assets/AutomaticHotspots.cs').read_bytes()
    if resume:
        js('window.__pick=arguments[0]',str(resume));click('打开优化项目');wait(lambda:state() is not None,30)
        assert os.path.samefile(state()['root'],project)
        directory=state()['directory'];report['records']=directory
    else:
        click('新建优化项目');js('window.__pick=arguments[0]',str(project));click('选择工程目录');value('项目名称','公开项目流程验收');click('创建并进入')
        wait(lambda:state() is not None,30);p=state();directory=p['directory'];report['records']=directory
        js('window.__pick=arguments[0]',str(a));click('导入录制 A');wait(lambda:len(state()['rounds'])==1,30)
        reopen(directory);wait(lambda:text('一键诊断并定位'),20)
    for i,agent in enumerate(['claude-code','codex']):
        print('Workflow round',i+1,agent,'analysis',flush=True)
        if not state()['rounds'][-1]['reports']:
            value('分析 AI',agent);click('一键诊断并定位');wait(lambda:state()['busy'],20);wait(ready)
        p=state();r=p['rounds'][-1]
        assert r['workflow']['status']=='completed',r['workflow']
        assert [q['status'] for q in r['reports']]==['completed','completed'],r['reports']
        assert r['reports'][0]['reportId']!=r['reports'][1]['reportId']
        reopen(directory);wait(lambda:text('开始优化'),20);value('修改 AI',agent)
        requirement=('仅处理公开 AutomaticHotspots.Update：读取 AutomaticCaller，新增 Assets/WorkflowBufferCache.cs 缓存4096字节数组，并修改已有 AutomaticHotspots.cs 使用它。明确允许返回数组复用；保持首字节为1。不修改其他已有文件。完成后执行编译检查。' if i==0 else '纯 Marker 任务：只修改 Assets/AutomaticHotspots.cs，在 AutomaticHotspots.Update 加静态 Unity.Profiling.ProfilerMarker 及 Auto() 作用域，命名 Public.AutomaticHotspots.Update。不改变其余行为，不新增文件。完成后编译检查，并说明需要重录，不能宣称性能改善。')
        value('补充要求（选填）',requirement);click('开始优化');wait(lambda:state()['busy'],20);wait(ready)
        p=state();r=p['rounds'][-1];run=r['runs'][-1]
        assert run['status']=='modified',run
        assert run['checks'][-1]['status']=='passed',run['checks']
        assert run['sessionId'] not in [q.get('sessionId',q['reportId']) for q in r['reports']]
        report.setdefault('rounds',[]).append(r)
        reopen(directory);wait(lambda:text('导入复测录制 B'),20)
        js('window.__pick=arguments[0]',str(b));click('导入复测录制 B');wait(lambda:state()['rounds'][-1]['candidate'] is not None,30)
        reopen(directory);wait(lambda:text('对比 A 与 B'),20);click('对比 A 与 B');wait(lambda:state()['rounds'][-1]['comparison'] is not None,30)
        exports()
        if i==0:
            value('玩法是否正常','passed');wait(lambda:state()['rounds'][-1]['correctness']=='passed',20);click('接受本轮修改');wait(lambda:text('开始下一轮'),20)
            prior=state()['rounds'][-1]['candidate'];click('开始下一轮');wait(lambda:len(state()['rounds'])==2,20);assert state()['rounds'][-1]['baseline']==prior
            reopen(directory);wait(lambda:text('一键诊断并定位'),20)
        else:
            click('回退本轮修改');wait(lambda:state()['rounds'][-1]['decision']=='rolled_back',30)
            assert state()['rounds'][-1]['performanceStatus'].startswith('不可判定')
    # Restore the disposable fixture's first-round changes, through production rollback.
    ipc('optimization_command',{'op':'rollback','roundId':state()['rounds'][0]['id']})
    assert (project/'Assets/AutomaticHotspots.cs').read_bytes()==original
    assert not (project/'Assets/WorkflowBufferCache.cs').exists()
    assert not (project/'Assets/WorkflowBufferCache.cs.meta').exists()
    report['checks']+=['project-first create/import/pipeline; saved reopen after A, localization, edit, B and compare','Claude and Codex new sessions modify public code, compile and export; two rounds accept B as baseline then rollback','both rounds restore exact public code and created helper/meta through backend rollback']
    report['scope']='Public synthetic captures; native dialogs substituted; no measured gameplay or real re-recording improvement claim'
    js('window.scrollTo(0,0)');time.sleep(.3)
    (output/'workflow.png').write_bytes(base64.b64decode(request('GET','/screenshot')))
