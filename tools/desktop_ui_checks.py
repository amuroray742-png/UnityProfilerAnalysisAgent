"""Rendered release UI checks. Only the native picker response is substituted."""
import base64
import time


def run_ui(js, request, root, output, report, real_agent=False, agent_id='claude-code'):
    for name in ('normal', 'zero-gc', 'partial-gc', 'invalid-tree', 'pagination'):
        if not (root / '.cache/manual-fixtures' / (name + '.json')).is_file():
            raise FileNotFoundError('Run python tools/prepare-manual-fixtures.py first')
    def wait(script, timeout=25):
        deadline = time.monotonic() + timeout
        while not js(script):
            if time.monotonic() > deadline:
                raise AssertionError('UI timeout: ' + script)
            time.sleep(.1)

    def click(selector, using='css selector'):
        item = request('POST', '/element', {'using': using, 'value': selector})
        request('POST', '/element/' + item['element-6066-11e4-a52e-4f735466cecf'] + '/click', {})

    def button(text):
        click(f'//button[normalize-space(.)="{text}"]', 'xpath')

    def text():
        return js('return document.body.innerText')

    def card(label):
        return js('return [...document.querySelectorAll(".metric-card")].find(e=>e.querySelector(".metric-label").textContent===arguments[0])?.innerText', label)

    def shot(name):
        (output / (name + '.png')).write_bytes(base64.b64decode(request('GET', '/screenshot')))

    # Tauri's invoke properties are immutable. Intercept only the native dialog
    # plugin on either IPC transport; forward every other request unchanged.
    js('''const nativeFetch=window.fetch.bind(window);
      window.__pickerResult=null; window.__pickerCalls=0;
      window.fetch=(url,options)=>{
        if(typeof url==='string' && new URL(url).hostname==='ipc.localhost'
          && decodeURIComponent(new URL(url).pathname)==='/plugin:dialog|open') {
          window.__pickerCalls++;
          return Promise.resolve(new Response(JSON.stringify(window.__pickerResult),
            {headers:{'Content-Type':'application/json','Tauri-Response':'ok'}}));
        }
        return nativeFetch(url,options);
      };
      const nativePost=window.chrome.webview.postMessage.bind(window.chrome.webview);
      window.chrome.webview.postMessage=(raw)=>{
        let message;
        try {message=typeof raw==='string'?JSON.parse(raw):raw;} catch {}
        if(message?.cmd==='plugin:dialog|open') {
          window.__pickerCalls++;
          window.__TAURI_INTERNALS__.runCallback(message.callback,window.__pickerResult);
          return;
        }
        return nativePost(raw);
      };''')

    def load(name):
        if js('return !!document.querySelector(".tabs")'):
            button('重置')
        wait('return !!document.querySelector(".dropzone")')
        js('window.__pickerResult=arguments[0]', str(root / '.cache/manual-fixtures' / (name + '.json')))
        click('.dropzone')
        if name == 'invalid-tree':
            wait('return !!document.querySelector(".error-banner")')
        else:
            wait('return document.body.innerText.includes("已就绪，等待 AI 诊断")')
            button('概览')

    click('.dropzone')
    wait('return window.__pickerCalls===1')
    assert '就绪' in text() and js('return !document.querySelector(".error-banner")')
    report['checks'].append('picker cancellation response leaves UI idle')
    load('normal')
    assert '12.00 ms' in card('主线程 p95')
    assert '32 B' in card('GC 分配（每帧 p95）')
    assert '—' in card('Draw Call p95') and '不可用' in card('Draw Call p95')
    assert '分析 2 / 声明 20 帧' in text()
    shot('normal-overview')
    button('CPU')
    wait('return document.querySelector("select[aria-label=线程]")?.innerText.includes("Worker")')
    wait('return document.body.innerText.includes("5 个样本")')
    assert '20 B' in text() and '4 B' in text()
    click('//select[@aria-label="深度"]/option[.="3"]', 'xpath')
    wait('return document.body.innerText.includes("存在超出当前深度")')
    click('//select[@aria-label="深度"]/option[.="8"]', 'xpath')
    wait('return !document.body.innerText.includes("存在超出当前深度")')
    click('select[aria-label="线程"] option[value="1"]')
    wait('return document.body.innerText.includes("2 个样本")')
    assert '8 B' in text()
    shot('worker-tree')
    assert js('return [...document.querySelectorAll("table[aria-label=慢帧列表] tbody tr")].map(r=>r.cells[0].textContent)') == ['10', '12']
    click('button[aria-label="查看帧 12 调用树"]')
    wait('return document.body.innerText.includes("1 个样本")')
    assert '帧 GC 0 B' in text()
    assert not js('return document.querySelector("select[aria-label=线程]").innerText.includes("Worker")')
    assert js('return document.querySelector("table[aria-label=调用树样本] tbody tr td:last-child").textContent') == '—'
    shot('frame12-zero-total')
    report['checks'].append('slow-frame ranking jumps to original frame 12, resets Worker selection and preserves zero GC')
    button('GC')
    assert '32 B' in card('总 GC 分配') and '—' in card('Gen0 回收')
    assert 'Main Thread #0 / Update' in text() and '24 B' in text() and 'Worker #1 / Worker' in text()
    wait('return !!document.querySelector("table[aria-label=高分配帧列表]")')
    assert js('return [...document.querySelectorAll("table[aria-label=高分配帧列表] tbody tr")].map(r=>[r.cells[0].textContent,r.cells[1].textContent])') == [['10', '32 B'], ['12', '0 B']]
    click('button[aria-label="查看帧 10 调用树"]')
    wait('return document.body.innerText.includes("5 个样本")')
    click('select[aria-label="线程"] option[value="1"]')
    wait('return document.body.innerText.includes("2 个样本")')
    assert '8 B' in text()
    click('button[aria-label="查看帧 12 调用树"]')
    wait('return document.body.innerText.includes("1 个样本")')
    assert '帧 GC 0 B' in text()
    shot('gc-frame-drilldown')
    report['checks'].append('GC ranked frames retain 32 B and zero, drill into Worker allocation and reset thread on frame change')
    button('渲染')
    assert all('—' in card(label) for label in ['Draw Call p95', 'SetPass p95', 'SRP Batcher 节省'])
    report['checks'].append('normal overview, CPU depth/thread/frame controls, GC attribution, unavailable rendering')
    load('zero-gc')
    assert '0 B' in card('GC 分配（每帧 p95）') and '有效帧 1/1' in card('GC 分配（每帧 p95）')
    load('partial-gc')
    assert '0 B' in card('GC 分配（每帧 p95）') and '有效帧 1/2' in card('GC 分配（每帧 p95）')
    assert '部分可用' in card('GC 分配（每帧 p95）')
    assert js('return !document.querySelector(".metric-card.warning,.metric-card.danger")')
    shot('partial-gc')
    button('GC')
    wait('return !!document.querySelector("table[aria-label=高分配帧列表]")')
    assert js('return [...document.querySelectorAll("table[aria-label=高分配帧列表] tbody tr")].map(r=>[r.cells[0].textContent,r.cells[1].textContent])') == [['12', '0 B']]
    assert '可查看 1/2 帧' in text()
    load('invalid-tree')
    assert 'sample 2' in text() and 'sample_index' in text()
    load('pagination')
    button('CPU')
    wait('return document.body.innerText.includes("211 个样本")')
    assert js('return document.querySelectorAll("table[aria-label=调用树样本] tbody tr").length') == 200
    button('下一页样本')
    wait('return document.querySelectorAll("table[aria-label=调用树样本] tbody tr").length===11')
    assert js('return document.querySelector("table[aria-label=调用树样本] tbody td").textContent') == '200 / 0'
    button('上一页样本')
    wait('return document.querySelectorAll("table[aria-label=调用树样本] tbody tr").length===200')
    report['checks'].append('zero vs partial GC, invalid input recovery, 211-node forward/back pagination')
    print('UI: overview, tree controls, GC quality, error recovery and pagination passed', flush=True)
    load('normal')
    shot('normal-reimported')
    if real_agent:
        wait(f'return !!document.querySelector(".agent-selector option[value={agent_id}]:not(:disabled)")')
        click(f'.agent-selector option[value="{agent_id}"]')
        button('开始 AI 诊断')
        print('UI: waiting for first real Agent diagnosis', flush=True)
        wait('return document.body.innerText.includes("诊断完成") || !!document.querySelector(".error-banner")', 180)
        assert js('return !document.querySelector(".error-banner")')
        assert js('return document.querySelector(".diagnosis-content").innerText.trim().length') > 0
        report['diagnoses'] = [js('return document.querySelector(".diagnosis-content").innerText')]
        shot('agent-complete')
        button('Agent 日志')
        assert '[mcp → ' in text() and '[finished] end_turn' in text()
        report['checks'].append('real Agent UI completion with MCP calls and end_turn')
        print('UI: first real Agent diagnosis completed with MCP calls', flush=True)
        button('概览')
        wait('return document.querySelector(".tab.active")?.textContent==="概览"')
        button('开始 AI 诊断')
        wait('return [...document.querySelectorAll("button")].some(b=>b.textContent.trim()==="取消")')
        button('Agent 日志')
        wait('return document.querySelector(".tab.active")?.textContent==="Agent 日志"')
        wait('return document.body.innerText.includes("[mcp → ") || !!document.querySelector(".error-banner")', 90)
        assert js('return !document.querySelector(".error-banner")')
        button('取消')
        wait('return document.body.innerText.includes("已就绪，等待 AI 诊断")')
        button('概览')
        frozen = js('return document.querySelector(".diagnosis-content")?.innerText || ""')
        time.sleep(2)
        assert js('return document.querySelector(".diagnosis-content")?.innerText || ""') == frozen
        shot('agent-cancelled')
        print('UI: cancellation returned to ready; checking a fresh diagnosis', flush=True)
        button('开始 AI 诊断')
        wait('return document.body.innerText.includes("诊断完成") || !!document.querySelector(".error-banner")', 180)
        assert js('return !document.querySelector(".error-banner")')
        report['checks'].append('cancel during real MCP activity, no late text, new diagnosis completes')
        report['diagnoses'].append(js('return document.querySelector(".diagnosis-content").innerText'))
