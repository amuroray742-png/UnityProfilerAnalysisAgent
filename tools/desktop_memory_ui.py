"""Real WebDriver clicks for memory measurements; never starts diagnosis."""
import time


def prepare_workload(js, request, input_path, inspect_heap=False, dom_control=False, handle_control=False):
    def click(selector, using='css selector'):
        if dom_control:
            if handle_control:
                # Discard Python's response immediately; only the remote element
                # registration differs from the no-handle DOM-event control.
                request('POST', '/element', {'using': using, 'value': selector})
            js('''const node=arguments[1]==='xpath'
              ? document.evaluate(arguments[0],document,null,XPathResult.FIRST_ORDERED_NODE_TYPE,null).singleNodeValue
              : document.querySelector(arguments[0]);
              if(!node) throw new Error('Control target missing');
              if(node.tagName==='OPTION') {node.parentElement.value=node.value; node.parentElement.dispatchEvent(new Event('change',{bubbles:true}));}
              else node.click();
              return null;''', selector, using)
            return
        item = request('POST', '/element', {'using': using, 'value': selector})
        request('POST', '/element/' + item['element-6066-11e4-a52e-4f735466cecf'] + '/click', {})

    def button(label):
        click(f'//button[normalize-space(.)="{label}"]', 'xpath')

    def wait(script):
        deadline = time.monotonic() + 180
        while not js(script):
            error = js('return document.querySelector(".error-banner,[role=alert]")?.innerText')
            if error:
                raise RuntimeError(error)
            if time.monotonic() > deadline:
                raise TimeoutError(script)
            time.sleep(.1)

    js('''
      const nativeFetch=window.fetch.bind(window);
      window.__memoryInput=arguments[0];
      window.fetch=(url,options)=>{
        if(typeof url==='string' && new URL(url).hostname==='ipc.localhost'
          && decodeURIComponent(new URL(url).pathname)==='/plugin:dialog|open')
          return Promise.resolve(new Response(JSON.stringify(window.__memoryInput),
            {headers:{'Content-Type':'application/json','Tauri-Response':'ok'}}));
        return nativeFetch(url,options);
      };
      const nativePost=window.chrome.webview.postMessage.bind(window.chrome.webview);
      window.chrome.webview.postMessage=(raw)=>{
        let message; try {message=typeof raw==='string'?JSON.parse(raw):raw;} catch {}
        if(message?.cmd==='plugin:dialog|open') {
          window.__TAURI_INTERNALS__.runCallback(message.callback,window.__memoryInput);
          return;
        }
        return nativePost(raw);
      };
    ''', str(input_path.resolve()))

    def cdp(command):
        return request('POST', '/ms/cdp/execute', {'cmd': command, 'params': {}})

    def probe():
        return {'heap':cdp('Runtime.getHeapUsage'), 'dom':cdp('Memory.getDOMCounters'),
                'attachedElements':js('return document.querySelectorAll("*").length')}

    def run(index, stage):
        wait('return !!document.querySelector(".dropzone")')
        started = time.monotonic()
        click('.dropzone')
        wait('return document.body.innerText.includes("已就绪，等待 AI 诊断")')
        button('概览')
        wait('return document.querySelectorAll(".metric-card").length===4')
        ready_ms = (time.monotonic() - started) * 1000
        stage[0] = f'overview-held-{index + 1}'
        time.sleep(2)
        stage[0] = f'tree-query-{index + 1}'
        started = time.monotonic()
        button('CPU')
        wait('return !!document.querySelector("table[aria-label=调用树样本] tbody tr")')
        details = js('''const select=document.querySelector('select[aria-label=帧]');
          return {frames:select.options.length, first:select.options[0].value,
            last:select.options[select.options.length-1].value};''')
        click('select[aria-label="帧"] option:last-child')
        wait('''const section=document.querySelector('section[aria-label=原始调用树]');
          const value=document.querySelector('select[aria-label=帧]').value;
          return section.innerText.includes('帧 '+value+' ·') && !!section.querySelector('table[aria-label=调用树样本] tbody tr');''')
        details['lastPageRows'] = js('return document.querySelectorAll("table[aria-label=调用树样本] tbody tr").length')
        assert 0 < details['lastPageRows'] <= 200
        query_ms = (time.monotonic() - started) * 1000
        stage[0] = f'tree-held-{index + 1}'
        time.sleep(2)
        button('GC')
        wait('return !!document.querySelector(".metric-card")')
        stage[0] = f'gc-held-{index + 1}'
        time.sleep(2)
        button('重置')
        wait('return !!document.querySelector(".dropzone") && !document.querySelector(".tabs")')
        assert js('return !document.querySelector("section[aria-label=原始调用树]")')
        result = {'readyWithRenderAndWebdriverMs':ready_ms, 'treeWithWebdriverMs':query_ms, **details}
        if inspect_heap:
            result['afterReset'] = probe()
        print(f'Rendered memory: completed round {index + 1}', flush=True)
        return result
    if inspect_heap:
        run.probe = probe
        run.collect_garbage = lambda: cdp('HeapProfiler.collectGarbage')
    return run
