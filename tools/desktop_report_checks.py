"""Public fixture report/source workflow on the release desktop. Calls the selected real Agent."""
import base64
import time
from desktop_memory_ui import prepare_workload


def run_report_checks(js, request, root, output, report, agent_id):
    public_input = root / 'src-tauri/tests/fixtures/isolated-peak.json'
    source_root = root / 'src-tauri/tests/fixtures/source-project'
    prepare_workload(js, request, public_input)
    def click(selector, using='css selector'):
        node = request('POST', '/element', {'using': using, 'value': selector})
        request('POST', '/element/' + node['element-6066-11e4-a52e-4f735466cecf'] + '/click', {})
    def button(label): click(f'//button[normalize-space(.)="{label}"]', 'xpath')
    def wait(script, timeout=420):
        deadline = time.monotonic() + timeout
        while not js(script):
            error = js('return document.querySelector(".error-banner")?.innerText')
            if error: raise AssertionError(error)
            if time.monotonic() > deadline: raise TimeoutError(script)
            time.sleep(.2)
    click('.dropzone')
    wait('return document.body.innerText.includes("已就绪，等待 AI 诊断")', 30)
    click(f'.agent-selector select option[value="{agent_id}"]')
    button('开始 AI 诊断')
    wait('return document.body.innerText.includes("AI 诊断中")', 15)
    wait('return !![...document.querySelectorAll("button")].find(e=>e.textContent==="开始源码定位")')
    assert js('return document.querySelectorAll(".diagnosis-content").length') >= 1
    js('window.__memoryInput=arguments[0]', str(source_root))
    button('选择目录')
    wait('return document.querySelector("input[aria-label=源码目录]").value===window.__memoryInput', 5)
    button('开始源码定位')
    wait('return document.body.innerText.includes("AI 诊断中")', 20)
    wait('return document.querySelectorAll(".diagnosis-content").length===2 && document.body.innerText.includes("诊断完成")')
    js('document.querySelectorAll(".diagnosis-content")[1].scrollIntoView()')
    (output / 'source-report.png').write_bytes(base64.b64decode(request('GET', '/screenshot')))
    # Only save dialog responses are replaced; export goes through the production command.
    js('''window.__reportSave=null;
      const fetchBeforeSave=window.fetch.bind(window);
      window.fetch=(url,options)=>{
        if(typeof url==='string' && new URL(url).hostname==='ipc.localhost' && decodeURIComponent(new URL(url).pathname)==='/plugin:dialog|save')
          return Promise.resolve(new Response(JSON.stringify(window.__reportSave),{headers:{'Content-Type':'application/json','Tauri-Response':'ok'}}));
        return fetchBeforeSave(url,options);
      };
      const postBeforeSave=window.chrome.webview.postMessage.bind(window.chrome.webview);
      window.chrome.webview.postMessage=(raw)=>{let m;try{m=typeof raw==='string'?JSON.parse(raw):raw;}catch{}
        if(m?.cmd==='plugin:dialog|save'){window.__TAURI_INTERNALS__.runCallback(m.callback,window.__reportSave);return;}return postBeforeSave(raw);};''')
    button('导出报告') # cancelled picker
    wait('return ![...document.querySelectorAll("button")].find(e=>e.textContent==="导出报告").disabled', 5)
    js('window.__reportSave=arguments[0]', str(output / 'missing-parent' / 'report.md'))
    button('导出报告')
    wait('return document.body.innerText.includes("导出失败")', 10)
    assert js('return document.querySelectorAll(".diagnosis-content").length') == 2
    for scope in ['performance', 'source']:
        click(f'select[aria-label=导出报告范围] option[value={scope}]')
        target = output / (scope + '.md')
        js('window.__reportSave=arguments[0]', str(target.resolve()))
        button('导出报告')
        wait('return document.body.innerText.includes("报告已导出")', 10)
        text = target.read_text(encoding='utf-8')
        title = '# 性能诊断报告' if scope == 'performance' else '# C# 源码定位报告'
        assert text.startswith(title)
    click('select[aria-label=导出报告范围] option[value=combined]')
    for fmt, suffix in [('markdown','md'),('html','html')]:
        target = output / ('combined.' + suffix)
        js('window.__reportSave=arguments[0]', str(target.resolve()))
        click(f'select[aria-label=导出格式] option[value={fmt}]')
        button('导出报告')
        wait('return document.body.innerText.includes("报告已导出")', 10)
        text = target.read_text(encoding='utf-8')
        assert '性能诊断报告' in text and 'C# 源码定位报告' in text and '覆盖率' in text
        assert 'AllocationWork' in text or 'OtherWork' in text
        if fmt == 'html': assert "default-src 'none'" in text and '@media print' in text and '<script' not in text
    report['checks'].append('public real Agent: first report retained, directory picker, C# source follow-up, separate/combined exports, Markdown/HTML, save cancellation, failed write and retry')
    report['sourceOutputPublic'] = True
    button('重置')
    wait('return !!document.querySelector(".dropzone")', 5)
    assert '源码定位报告' not in js('return document.body.innerText')
