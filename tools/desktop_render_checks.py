"""Release rendering page acceptance against local Editor reference; never calls an Agent."""
import base64
import json
import math
import time
from desktop_memory_ui import prepare_workload


def run_render_checks(js, request, input_path, reference_path, output, report):
    reference = json.loads(reference_path.read_text(encoding='utf-8'))['frames']
    assert reference
    prepare_workload(js, request, input_path)  # Substitute only the native picker response.

    def click(selector, using='css selector'):
        node = request('POST', '/element', {'using': using, 'value': selector})
        request('POST', '/element/' + node['element-6066-11e4-a52e-4f735466cecf'] + '/click', {})

    started = time.monotonic()
    click('.dropzone')
    deadline = started + 180
    while not js('return document.body.innerText.includes("已就绪，等待 AI 诊断")'):
        error = js('return document.querySelector(".error-banner,[role=alert]")?.innerText')
        if error:
            raise AssertionError(error)
        if time.monotonic() > deadline:
            raise TimeoutError('render capture import')
        time.sleep(.1)
    report['renderImportSeconds'] = time.monotonic() - started
    click('//button[normalize-space(.)="渲染"]', 'xpath')
    counters = {}
    for name, label in [('Draw Calls Count', 'Draw Call'), ('SetPass Calls Count', 'SetPass'),
                        ('Batches Count', 'Batches'), ('Triangles Count', 'Triangles'), ('Vertices Count', 'Vertices')]:
        values = sorted(c['value'] for f in reference for c in f['counters'] if c['name'] == name and c['available'])
        card = js('''return [...document.querySelectorAll('.metric-card')].find(e=>e.querySelector('.metric-label').textContent===arguments[0])?.innerText''', label + ' p95')
        assert card is not None, label
        assert f'有效帧 {len(values)}/{len(reference)}' in card, card
        if values:
            p95 = values[math.floor((len(values)-1)*.95+.5)]
            expected = str(p95) if p95 < 1000 else f'{p95/1000:.1f}K' if p95 < 1_000_000 else f'{p95/1_000_000:.1f}M'
            assert expected in card.splitlines(), card
            assert f'max {max(values)}' in card, card
        else:
            assert '—' in card, card
        counters[label] = card
    body = js('return document.body.textContent')
    assert '渲染 CPU marker 热点' in body and '不代表 GPU 时间' in body
    assert js('return [...document.querySelectorAll(".metric-card")].find(e=>e.querySelector(".metric-label").textContent==="SRP Batcher 节省")?.querySelector(".metric-value")?.textContent') == '—'
    (output / 'rendering.png').write_bytes(base64.b64decode(request('GET', '/screenshot')))
    report['renderCards'] = counters
    report['checks'].append('render page five counter p95/max/coverage match local Editor reference; GPU/SRP not fabricated')
    click('//button[normalize-space(.)="重置"]', 'xpath')
    assert js('return !!document.querySelector(".dropzone")')
