"""Public scene UI acceptance. Real IPC, no Agent, no native picker claims."""
import base64
import json
import time
import uuid
from pathlib import Path


def run_scenes(js, request, root, output, report, saved_records=None):
    def ipc(command, action, scene=None, error=False):
        result = request('POST', '/execute/async', {'script': '''
          const done=arguments[arguments.length-1];
          window.__TAURI_INTERNALS__.invoke(arguments[0],{action:arguments[1],sceneId:arguments[2]})
          .then(value=>done({value}),e=>done({failure:String(e)}));''', 'args': [command, action, scene]})
        if error:
            assert 'failure' in result, result
            return result['failure']
        assert 'failure' not in result, result
        return result['value']

    def wait(predicate):
        deadline = time.monotonic() + 25
        while not predicate():
            if time.monotonic() > deadline:
                raise TimeoutError('scene UI did not reach expected state')
            time.sleep(.1)

    def element(xpath):
        return request('POST', '/element', {'using': 'xpath', 'value': xpath})['element-6066-11e4-a52e-4f735466cecf']

    def click(text):
        wait(lambda: js('return [...document.querySelectorAll("button")].some(b=>b.textContent.trim()===arguments[0]&&!b.disabled)', text))
        request('POST', '/element/' + element('//button[normalize-space(.)=' + json.dumps(text, ensure_ascii=False) + ']') + '/click', {})

    def fill(label, value):
        target = element('//label[contains(.,' + json.dumps(label, ensure_ascii=False) + ')]/*[self::input or self::textarea]')
        request('POST', '/element/' + target + '/value', {'text': value})

    def state():
        return ipc('optimization_command', {'op': 'get'})

    if saved_records:
        saved_records = saved_records.resolve()
        manifest = json.loads((saved_records / 'optimization.json').read_bytes())
        fixture = Path(manifest['root'])
        assert (fixture / '.upaa-public-fixture').is_file(), 'Only public acceptance records may be reopened'
        ipc('workflow_command', {'op': 'open', 'directory': str(saved_records)})
        js('location.reload();return true')
        wait(lambda: js('return document.body.innerText.includes("主城跑图")'))
        p = state()
        assert len(p['scenes']) == 2 and len(p['rounds']) == 3
        assert p['activeSceneId'] == p['scenes'][0]['id']
        assert p['scenes'][0]['description'] == '绕广场一圈'
        assert not p['scenes'][0]['completed']
        click('历史轮次')
        wait(lambda: js('return document.querySelectorAll(".round-list button").length===2'))
        (output / 'scenes-reopened.png').write_bytes(base64.b64decode(request('GET', '/screenshot')))
        report['checks'].append('fresh application process restores selected scene, two histories, description and reopened state')
        report['scope'] = 'Fresh Windows application process opens public v4 scene archive; no Agent or Unity runtime'
        report['sceneRecords'] = str(saved_records)
        return

    project = output / ('public-project-' + uuid.uuid4().hex)
    records = output / ('records-' + uuid.uuid4().hex)
    for directory in ['Assets', 'Packages', 'ProjectSettings']:
        (project / directory).mkdir(parents=True)
    (project / '.upaa-public-fixture').write_text('Public scene acceptance fixture\n', encoding='utf-8')
    (project / 'Assets/PublicWork.cs').write_text('class PublicWork {}\n', encoding='utf-8')
    ipc('workflow_command', {'op': 'create', 'root': str(project), 'directory': str(records), 'name': '公开场景分层验收'})
    # ProjectWorkflow initially loads once; reload so the created workspace is displayed.
    js('location.reload();return true')
    wait(lambda: js('return document.body.innerText.includes("新建场景")'))
    assert state()['scenes'] == []
    click('新建场景'); fill('场景名称', '主城跑图'); fill('复现说明', '绕广场一圈'); click('保存场景')
    wait(lambda: len(state()['scenes']) == 1)
    x = state()['activeSceneId']
    capture = str(root / 'src-tauri/tests/fixtures/editor-dump.json')
    assert '确认' in ipc('workflow_command', {'op': 'bind', 'path': capture, 'role': 'a'}, x, error=True)
    ipc('workflow_command', {'op': 'bind', 'path': capture, 'role': 'a', 'currentCodeConfirmed': True}, x)
    rid = state()['rounds'][0]['id']
    # Empty rollback closes the fixture round without synthesizing AI output or edits.
    ipc('optimization_command', {'op': 'rollback', 'roundId': rid}, x)
    ipc('workflow_command', {'op': 'next'}, x)
    assert len(state()['rounds']) == 2
    click('标记场景已完成')
    wait(lambda: state()['scenes'][0]['completed'])
    click('新建场景'); fill('场景名称', '百人战斗'); fill('复现说明', '固定队伍进入战斗'); click('保存场景')
    wait(lambda: len(state()['scenes']) == 2)
    y = state()['activeSceneId']
    wait(lambda: js('return document.body.innerText.includes("第一步：导入优化前的录制")'))
    assert state()['scenes'][1]['roundCount'] == 0
    ipc('workflow_command', {'op': 'bind', 'path': capture, 'role': 'a', 'currentCodeConfirmed': True}, y)
    assert state()['scenes'][1]['roundCount'] == 1
    assert len(state()['rounds']) == 3
    assert '不属于' in ipc('workflow_command', {'op': 'analyze', 'roundId': rid, 'analysisAgent': 'codex', 'localizationAgent': 'codex', 'restart': False}, y, error=True)
    # Switch with the actual scene button; it contains a multi-line status summary.
    target = element('//nav[@aria-label="选择优化场景"]/button[strong[text()="主城跑图"]]')
    request('POST', '/element/' + target + '/click', {})
    wait(lambda: state()['activeSceneId'] == x)
    click('历史轮次')
    wait(lambda: js('return document.body.innerText.includes("主城跑图 历史轮次")'))
    assert js('return document.querySelectorAll(".round-list button").length') == 2
    click('重新开启场景')
    wait(lambda: not state()['scenes'][0]['completed'])
    click('返回项目首页')
    ipc('workflow_command', {'op': 'open', 'directory': str(records)})
    js('location.reload();return true')
    wait(lambda: js('return document.body.innerText.includes("主城跑图")'))
    restored = state()
    assert restored['activeSceneId'] == x
    assert len(restored['scenes']) == 2 and len(restored['rounds']) == 3
    assert restored['scenes'][0]['description'] == '绕广场一圈'
    assert not restored['scenes'][0]['completed']
    (output / 'scenes.png').write_bytes(base64.b64decode(request('GET', '/screenshot')))
    report['checks'] += ['scene creation and descriptions via desktop controls', 'recording freshness confirmation enforced by IPC',
                         'X two rounds and Y independent first round', 'cross-scene round identity rejected before Agent launch',
                         'scene completion, reopening and history filtering', 'selected scene and histories survive workspace close/reopen and WebView reload']
    report['scope'] = 'Windows release scene hierarchy UI and real IPC with public fixture. No AI, Unity replay, native picker or real-game benefit validation.'
    report['sceneRecords'] = str(records)
