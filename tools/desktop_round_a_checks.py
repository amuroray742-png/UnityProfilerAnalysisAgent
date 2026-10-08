"""Public diagnosed A replacement: real desktop UI/IPC, substituted native picker.

Only old diagnosis state is synthesized while the disposable workspace is closed.
No Agent is called and no user project is opened or modified.
"""
import base64
import copy
import hashlib
import json
import time
import uuid


def run_round_a(js, request, root, output, report):
    js('''const f=window.fetch.bind(window),p=window.chrome.webview.postMessage.bind(window.chrome.webview);
      window.__pick=null;
      window.fetch=(u,o)=>{if(typeof u==='string'&&new URL(u).hostname==='ipc.localhost'&&decodeURIComponent(new URL(u).pathname)==='/plugin:dialog|open')return Promise.resolve(new Response(JSON.stringify(window.__pick),{headers:{'Content-Type':'application/json','Tauri-Response':'ok'}}));return f(u,o);};
      window.chrome.webview.postMessage=raw=>{const m=typeof raw==='string'?JSON.parse(raw):raw;if(m.cmd==='plugin:dialog|open'){window.__TAURI_INTERNALS__.runCallback(m.callback,window.__pick);return;}return p(raw);};''')

    def ipc(name, action):
        result = request('POST', '/execute/async', {'script': '''const done=arguments[arguments.length-1];
          window.__TAURI_INTERNALS__.invoke(arguments[0],{action:arguments[1]}).then(v=>done({value:v}),e=>done({error:String(e)}));''', 'args': [name, action]})
        if 'error' in result:
            raise AssertionError(result['error'])
        return result['value']

    def state():
        return ipc('optimization_command', {'op': 'get'})

    def wait(predicate):
        deadline = time.monotonic() + 30
        while not predicate():
            if time.monotonic() > deadline:
                raise TimeoutError('diagnosed A replacement UI')
            time.sleep(.1)

    def text(value):
        return js('return document.body.innerText.includes(arguments[0])', value)

    def click(value):
        wait(lambda: js('return [...document.querySelectorAll("button")].some(b=>b.textContent.trim()===arguments[0]&&!b.disabled)', value))
        element = request('POST', '/element', {'using': 'xpath', 'value': '//button[normalize-space(.)=' + json.dumps(value, ensure_ascii=False) + ']'})
        request('POST', '/element/' + element['element-6066-11e4-a52e-4f735466cecf'] + '/click', {})

    project = output / ('public-project-' + uuid.uuid4().hex)
    records = output / ('records-' + uuid.uuid4().hex)
    for folder in ['Assets', 'Packages', 'ProjectSettings']:
        (project / folder).mkdir(parents=True)
    original = b'class PublicWork {}\n'
    (project / 'Assets/PublicWork.cs').write_bytes(original)
    (project / '.upaa-public-fixture').write_text('Public diagnosed A replacement fixture\n', encoding='utf-8')
    name = '公开诊断后重选 A 验收'
    ipc('workflow_command', {'op': 'create', 'root': str(project), 'name': name, 'directory': str(records)})
    ipc('workflow_command', {'op': 'bind', 'role': 'a', 'path': str(root / 'src-tauri/tests/fixtures/editor-dump.json')})
    ipc('optimization_command', {'op': 'close'})
    manifest_path = records / 'optimization.json'
    manifest = json.loads(manifest_path.read_bytes())

    def resolve(ref):
        return json.loads((records / 'objects' / (ref['object'] + '.json')).read_bytes())

    def object_ref(data):
        raw = json.dumps(data, ensure_ascii=False, separators=(',', ':')).encode('utf-8')
        key = hashlib.sha256(raw).hexdigest()
        (records / 'objects' / (key + '.json')).write_bytes(raw)
        return {'object': key}

    current = resolve(manifest['rounds'][0])
    current_id = current['id']
    history = []
    for decision in ['accepted', 'rolled_back']:
        previous = copy.deepcopy(current)
        previous['id'] = str(uuid.uuid4())
        previous['decision'] = decision
        history.append(object_ref(previous))
    meta = resolve(manifest['captures'][0])['snapshot']['meta']
    performance_id, project_id = str(uuid.uuid4()), str(uuid.uuid4())
    old_reports = []
    for stage, status, key, parent in [('performance', 'completed', performance_id, None), ('project', 'interrupted', project_id, performance_id)]:
        old_reports.append(object_ref(dict(reportId=key, fileId=current['baseline'], sessionId=key, stage=stage,
            parentReportId=parent, text='公开旧诊断报告', createdAt='2026-10-08T03:00:00+00:00', agentId='codex',
            status=status, incompleteReason=None, fileName=meta['fileName'], unityVersion=meta['unityVersion'],
            frameCount=meta['frameCount'], coverage='公开夹具')))
    current.update(reports=old_reports, tasks=[dict(id='old-task', kind='investigate', title='公开旧任务',
        evidence='旧 A', files={}, instructions='调查', acceptance='核对', constraints='', selected=True)],
        taskVersion=7, taskVerifications={'old-task': 'passed'}, tests=['Public.EditMode'],
        workflow=dict(stage='project', status='interrupted', reason='公开定位已中断', analysisAgent='codex', localizationAgent='codex'))
    manifest.update(rounds=history + [object_ref(current)], budgets={'cpuMs': 16})
    manifest_path.write_text(json.dumps(manifest, ensure_ascii=False, separators=(',', ':')), encoding='utf-8')
    (output / 'before.json').write_text(json.dumps(manifest, ensure_ascii=False, indent=2), encoding='utf-8')

    def open_records():
        js('window.__pick=arguments[0]', str(records))
        click('打开优化项目')
        wait(lambda: text(name + ' · 第 3 轮'))

    open_records()
    wait(lambda: text('重新导入 A') and text('继续诊断定位'))
    assert text('重新导入会放弃本轮已有诊断与定位结果，轮次编号不变，需要重新诊断。')
    before = state()
    assert before['rounds'][-1]['reports'][0]['status'] == 'completed'
    assert before['rounds'][-1]['reports'][1]['status'] == 'interrupted'
    assert not before['rounds'][-1]['runs']
    report['checks'].append('public round 3: completed performance + interrupted localization + no optimization run offers A replacement')

    js('window.__pick=null')
    click('重新导入 A')
    wait(lambda: js('return [...document.querySelectorAll("button")].some(b=>b.textContent.trim()==="重新导入 A"&&!b.disabled)'))
    assert state()['rounds'] == before['rounds']
    invalid = output / 'invalid.json'
    invalid.write_text('invalid profiler recording', encoding='utf-8')
    js('window.__pick=arguments[0]', str(invalid))
    click('重新导入 A')
    wait(lambda: js('return !!document.querySelector(".error-banner")'))
    assert state()['rounds'] == before['rounds']
    assert state()['captures'] == before['captures']
    assert text('继续诊断定位')
    report['checks'].append('cancelled picker and corrupt capture preserve old A, diagnosis and tasks and allow retry')

    js('window.__pick=arguments[0]', str(root / 'src-tauri/tests/fixtures/isolated-peak.json'))
    click('重新导入 A')
    wait(lambda: text('一键诊断并定位') and not text('继续诊断定位'))
    after = state()
    r = after['rounds'][-1]
    assert len(after['rounds']) == 3 and r['id'] == current_id
    assert r['baseline'] != before['rounds'][-1]['baseline']
    assert not r['reports'] and not r['tasks'] and not r['runs'] and not r['taskVerifications']
    assert r['candidate'] is None and r['comparison'] is None and r['correctness'] == 'pending'
    assert r['taskVersion'] == 8 and r['tests'] == ['Public.EditMode']
    assert not r['workflow']['stage'] and not r['workflow']['status'] and r['workflow']['reason'] is None
    assert r['workflow']['analysisAgent'] == 'codex' and r['workflow']['localizationAgent'] == 'codex'
    assert after['budgets'] == {'cpuMs': 16}
    assert after['rounds'][:2] == before['rounds'][:2]
    assert not text('公开定位已中断') and not text('公开旧任务')
    assert not js('return [...document.querySelectorAll("button")].some(b=>b.textContent.trim()==="开始优化")')
    assert (project / 'Assets/PublicWork.cs').read_bytes() == original
    capture = next(c for c in after['captures'] if c['id'] == r['baseline'])
    assert capture['snapshot']['frameCount'] == 21
    committed = json.loads(manifest_path.read_bytes())
    assert committed['rounds'][:2] == history
    assert committed['captures'][0] == manifest['captures'][0]
    for action, message in [({'op': 'report', 'roundId': current_id, 'reportId': performance_id, 'start': 0}, '报告不属于此轮次'),
                            ({'op': 'activity', 'roundId': current_id, 'runId': project_id, 'cursor': 0}, '工作记录不属于此轮次')]:
        try:
            ipc('workflow_command', action)
        except (AssertionError, RuntimeError) as error:
            assert message in str(error)
        else:
            raise AssertionError('abandoned diagnosis remains accessible')
    report['checks'].append('replacement commits new 21-frame A in same round; clears old diagnosis/tasks; preserves prior rounds, original capture, AI choices, tests and budget; no Agent started')

    click('返回项目首页')
    open_records()
    wait(lambda: text('一键诊断并定位'))
    reopened = state()
    assert reopened['rounds'] == after['rounds'] and reopened['captures'] == after['captures']
    assert not reopened['busy'] and not reopened['rounds'][-1]['reports']
    report['checks'].append('reopen restores new A and same round with no old diagnosis and no automatic AI work')
    report['scope'] = 'Public synthetic old diagnosis; real Windows desktop WebView/UI and production IPC. Native picker return substituted; excludes native dialog and real Agent acceptance.'
    report['records'] = str(records)
    report['roundId'] = current_id
    report['before'] = before
    report['after'] = reopened
    (output / 'replacement.png').write_bytes(base64.b64decode(request('GET', '/screenshot')))
    click('返回项目首页')
