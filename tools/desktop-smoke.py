"""Windows release WebView startup and real IPC smoke.

Uses the W3C WebDriver wire protocol with tauri-driver and matching Edge driver.
Optional --ui checks rendered controls using only a substituted native picker
response; --real-agent additionally tests the configured Agent on public data.
"""
import argparse
import base64
import datetime
import hashlib
import json
import os
from pathlib import Path
import socket
import subprocess
import time
import urllib.request
import urllib.error


def main():
    root = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser()
    parser.add_argument('--plugin-install', action='store_true', help='Public embedded plugin install and migration')
    parser.add_argument('--application', type=Path, default=root / 'src-tauri/target/release/unity-profiler-analysis-agent.exe')
    parser.add_argument('--driver', type=Path, default=root / '.cache/webdriver/bin/tauri-driver.exe')
    parser.add_argument('--native-driver', type=Path, default=root / '.cache/webdriver/edge/msedgedriver.exe')
    parser.add_argument('--ui', action='store_true', help='Exercise rendered UI with only the native picker response substituted')
    parser.add_argument('--real-agent', action='store_true', help='Also send the public fixture to the selected ACP Agent and test UI cancellation')
    parser.add_argument('--measure-input', type=Path, help='Measure actual desktop processes via IPC on a local capture; never calls an Agent')
    parser.add_argument('--measure-rendered', action='store_true', help='Measure actual result pages and reset with substituted native picker response')
    parser.add_argument('--measure-repeats', type=int, default=3, help='Measurement repetitions, 1..30')
    parser.add_argument('--measure-heap', action='store_true', help='Rendered mode: inspect DOM/heap, idle 30s, then diagnostically collect GC')
    parser.add_argument('--measure-dom-control', action='store_true', help='Heap investigation control: DOM events instead of WebDriver element handles; not UI acceptance')
    parser.add_argument('--measure-handle-control', action='store_true', help='DOM-event control plus WebDriver element lookup, without WebDriver click')
    parser.add_argument('--render-input', type=Path, help='Validate rendering page on a local capture; never calls an Agent')
    parser.add_argument('--render-reference', type=Path, help='Editor counter reference for --render-input')
    parser.add_argument('--agent-id', choices=['claude-code', 'codex', 'gemini'], default='claude-code', help='Agent preset for explicit --real-agent validation')
    parser.add_argument('--report-source', action='store_true', help='Real Agent report/export/C# workflow using public fixtures only')
    parser.add_argument('--optimization-loop', action='store_true', help='Real public optimization workflow with saved tasks, two Agents, checks and rollback')
    parser.add_argument('--optimization-saved', type=Path, help='Reopen the public acceptance records and exercise final UI exports without calling an Agent')
    parser.add_argument('--project-workflow', action='store_true')
    parser.add_argument('--workflow-saved', type=Path)
    parser.add_argument('--workflow-resume', type=Path)
    parser.add_argument('--history-replay', type=Path)
    parser.add_argument('--history-progress', action='store_true')
    parser.add_argument('--history-activity', action='store_true')
    parser.add_argument('--history-records', type=Path)
    args = parser.parse_args()
    # Keep optimization acceptance entrypoints usable after the project-first UI.
    if args.optimization_loop:
        args.project_workflow=True
        args.optimization_loop=False
    if args.optimization_saved:
        args.workflow_saved=args.optimization_saved
        args.optimization_saved=None
    if args.optimization_loop and (args.ui or args.real_agent or args.report_source or args.measure_input or args.render_input):
        parser.error('--optimization-loop is a separate public real-Agent workflow')
    if args.report_source and (args.ui or args.real_agent or args.measure_input or args.render_input):
        parser.error('--report-source is a separate public real-Agent workflow')
    if bool(args.render_input) != bool(args.render_reference):
        parser.error('--render-input and --render-reference must be supplied together')
    if args.render_input and (args.ui or args.real_agent or args.measure_input):
        parser.error('--render-input is separate from --ui, --real-agent and --measure-input')
    for path in (args.render_input, args.render_reference):
        if path and not path.is_file():
            raise FileNotFoundError(path)
    if args.measure_handle_control and not args.measure_dom_control:
        parser.error('--measure-handle-control requires --measure-dom-control')
    if args.measure_dom_control and not args.measure_heap:
        parser.error('--measure-dom-control requires --measure-heap')
    if args.measure_heap and not args.measure_rendered:
        parser.error('--measure-heap requires --measure-rendered')
    if args.measure_rendered and not args.measure_input:
        parser.error('--measure-rendered requires --measure-input')
    if not 1 <= args.measure_repeats <= 30:
        parser.error('--measure-repeats must be 1..30')
    if args.real_agent and not args.ui:
        parser.error('--real-agent requires --ui')
    if args.measure_input and (args.ui or args.real_agent):
        parser.error('--measure-input is separate from --ui/--real-agent')
    if args.measure_input and not args.measure_input.is_file():
        raise FileNotFoundError(args.measure_input)
    for path in (args.application, args.driver, args.native_driver):
        if not path.is_file():
            raise FileNotFoundError(path)
    output = root / '.cache' / ('desktop-plugin' if args.plugin_install else 'desktop-history-replay' if args.history_replay else 'desktop-progress' if args.history_progress else 'desktop-history' if args.history_activity else 'desktop-workflow-saved' if args.workflow_saved else 'desktop-workflow' if args.project_workflow else 'desktop-optimization-saved' if args.optimization_saved else 'desktop-optimization' if args.optimization_loop else 'desktop-reports' if args.report_source else 'desktop-render' if args.render_input else 'desktop-memory' if args.measure_input else 'desktop-ui' if args.ui else 'desktop-smoke')
    output.mkdir(parents=True, exist_ok=True)
    def port():
        with socket.socket() as sock:
            sock.bind(('127.0.0.1', 0))
            return sock.getsockname()[1]
    address, native = port(), port()
    while native == address:
        native = port()
    base = f'http://127.0.0.1:{address}'
    def request(method, path, body=None):
        raw = None if body is None else json.dumps(body).encode()
        req = urllib.request.Request(base + path, data=raw, method=method, headers={'Content-Type': 'application/json'})
        try:
            with urllib.request.urlopen(req, timeout=960 if (args.optimization_loop or args.project_workflow or args.history_activity) and path.endswith('/execute/async') else 45) as response:
                value = json.load(response)['value']
        except urllib.error.HTTPError as error:
            raise RuntimeError(error.read().decode()) from error
        if isinstance(value, dict) and value.get('error'):
            raise RuntimeError(value)
        return value
    env = dict(os.environ)
    env['WEBVIEW2_USER_DATA_FOLDER'] = str(output / 'webview-profile')
    session = None
    report = {'dateUtc': datetime.datetime.now(datetime.timezone.utc).isoformat(), 'applicationSha256': hashlib.sha256(args.application.read_bytes()).hexdigest(), 'scope': 'Release WebView startup and direct real IPC; excludes native picker, rendered analysis workflow, Agent and MSI', 'checks': []}
    with (output / 'driver.log').open('w', encoding='utf-8') as log:
        process = subprocess.Popen([str(args.driver), '--port', str(address), '--native-port', str(native), '--native-driver', str(args.native_driver.resolve())], env=env, stdout=log, stderr=log, creationflags=subprocess.CREATE_NO_WINDOW)
        try:
            deadline = time.monotonic() + 15
            while True:
                try:
                    request('GET', '/status')
                    break
                except (OSError, ValueError):
                    if process.poll() is not None or time.monotonic() > deadline:
                        raise RuntimeError('WebDriver unavailable; see driver.log')
                    time.sleep(.1)
            created = request('POST', '/session', {'capabilities': {'alwaysMatch': {'tauri:options': {'application': str(args.application.resolve())}}}})
            session = created['sessionId']
            def js(script, *values):
                return request('POST', f'/session/{session}/execute/sync', {'script': script, 'args': list(values)})
            def wait(script):
                deadline = time.monotonic() + 20
                while not js(script):
                    if time.monotonic() > deadline:
                        raise AssertionError('Timed out: ' + script)
                    time.sleep(.1)
            wait('return !!document.querySelector(".project-workflow")')
            report['checks'].append('release UI startup')
            data = request('POST', f'/session/{session}/execute/async', {'script': '''
                const done=arguments[arguments.length-1], path=arguments[0];
                const invoke=window.__TAURI_INTERNALS__.invoke;
                (async()=>{
                  const upload=await invoke('upload',{filePath:path});
                  try {
                    const snapshot=await invoke('analyze',{fileId:upload.fileId});
                    const threads10=await invoke('frame_details',{fileId:upload.fileId,frameIndex:10,start:0,limit:128});
                    const threads12=await invoke('frame_details',{fileId:upload.fileId,frameIndex:12,start:0,limit:128});
                    const worker=await invoke('cpu_hierarchy',{fileId:upload.fileId,frameIndex:10,threadIndex:1,start:0,limit:200,maxDepth:8});
                    const page=await invoke('cpu_hierarchy',{fileId:upload.fileId,frameIndex:10,threadIndex:null,start:2,limit:2,maxDepth:64});
                    await invoke('release_file',{fileId:upload.fileId});
                    let released=false;
                    try {await invoke('cpu_hierarchy',{fileId:upload.fileId,frameIndex:10,threadIndex:null,start:0,limit:2,maxDepth:64});}
                    catch {released=true;}
                    return {snapshot,page,threads10,threads12,worker,released};
                  } finally {await invoke('release_file',{fileId:upload.fileId});}
                })().then(done,e=>done({error:String(e)}));
                ''', 'args': [str(root / 'src-tauri/tests/fixtures/editor-dump.json')]})
            if 'error' in data:
                raise AssertionError(data['error'])
            assert data['snapshot']['meta']['frameCount'] == 2
            assert data['snapshot']['meta']['declaredFrameCount'] == 20
            assert data['snapshot']['cpu']['mainThreadMs']['p95'] == 12
            assert data['snapshot']['gc']['totalAllocBytes'] == 32
            assert data['snapshot']['rendering']['drawCalls']['p95'] is None
            assert [t['name'] for t in data['threads10']['threads']] == ['Main Thread', 'Worker']
            assert [t['name'] for t in data['threads12']['threads']] == ['Main Thread']
            assert data['worker']['thread']['name'] == 'Worker'
            assert data['worker']['samples'][1]['gcAllocBytes'] == 8
            assert [s['gcAllocBytes'] for s in data['page']['samples']] == [20, 4]
            assert data['released']
            report['checks'] += ['public dump real IPC CPU/GC/null metrics', 'nested tree real IPC byte values', 'released capture no longer queryable']
            report['checks'].append('frame 10 exposes Worker and its 8 B allocation; frame 12 has only Main Thread')
            if args.ui:
                from desktop_ui_checks import run_ui
                report['scope'] = 'Release rendered UI and real backend; only native picker response substituted; excludes native picker and MSI'
                report['realAgentRequested'] = args.real_agent
                report['agentId'] = args.agent_id if args.real_agent else None
                run_ui(js, lambda method, path, body=None: request(method, f'/session/{session}' + path, body), root, output, report, args.real_agent, args.agent_id)
            if args.plugin_install:
                from desktop_plugin_checks import run_plugin
                run_plugin(js,lambda method,path,body=None: request(method,f'/session/{session}'+path,body),root,output,report)
            if args.history_replay:
                from desktop_history_checks import run_replay
                run_replay(js,lambda method,path,body=None: request(method,f'/session/{session}'+path,body),root,output,report,args.history_replay)
            if args.history_progress:
                from desktop_history_checks import run_progress
                run_progress(js,lambda method,path,body=None: request(method,f'/session/{session}'+path,body),root,output,report)
            if args.history_activity:
                from desktop_history_checks import run_history_activity
                run_history_activity(js,lambda method,path,body=None: request(method,f'/session/{session}'+path,body),root,output,report,args.history_records)
            if args.project_workflow or args.workflow_saved:
                from desktop_workflow_checks import run_workflow
                run_workflow(js,lambda method,path,body=None: request(method,f'/session/{session}'+path,body),root,output,report,args.workflow_saved,args.workflow_resume)
            if args.optimization_loop:
                from desktop_optimization_checks import run_optimization_checks
                report['scope']='Public release optimization workflow; native dialogs substituted; synthetic A/B is not game benefit evidence'
                run_optimization_checks(js, lambda method,path,body=None: request(method, f'/session/{session}'+path,body),root,output,report,args.agent_id)
            if args.optimization_saved:
                from desktop_optimization_checks import run_saved_checks
                run_saved_checks(js,lambda method,path,body=None: request(method,f'/session/{session}'+path,body),root,output,report,args.optimization_saved)
            if args.report_source:
                from desktop_report_checks import run_report_checks
                report['scope'] = 'Release public real-Agent reports/source/export; native picker/save responses substituted'
                run_report_checks(js, lambda method, path, body=None: request(method, f'/session/{session}' + path, body), root, output, report, args.agent_id)
            if args.render_input:
                from desktop_render_checks import run_render_checks
                report['scope'] = 'Release rendering page vs local Editor reference; native picker response substituted; excludes Agent'
                run_render_checks(js, lambda method, path, body=None: request(method, f'/session/{session}' + path, body), args.render_input, args.render_reference, output, report)
            if args.measure_input:
                from desktop_memory import measure
                report['scope'] = 'Actual Tauri/WebView processes, real IPC import/query/release; excludes results rendering, Agent and native picker'
                workload = None
                if args.measure_rendered:
                    from desktop_memory_ui import prepare_workload
                    workload = prepare_workload(js, lambda method, path, body=None: request(method, f'/session/{session}' + path, body), args.measure_input, args.measure_heap, args.measure_dom_control, args.measure_handle_control)
                    report['scope'] = 'Actual Tauri/WebView processes and rendered overview/tree/reset; native picker response substituted; excludes Agent'
                    report['domEventControl'] = args.measure_dom_control
                    report['elementHandleControl'] = args.measure_handle_control
                report['memory'] = measure(js, process.pid, args.application, args.measure_input, output, workload, args.measure_repeats)
                report['checks'].append('repeated rendered import/tree/reset' if workload else 'repeated IPC imports, concurrent bounded frame queries, release rejects further queries')
            report['text'] = js('return document.body.innerText')
            screenshot = request('GET', f'/session/{session}/screenshot')
            (output / 'startup.png').write_bytes(base64.b64decode(screenshot))
            report['passed'] = True
        except Exception as error:
            report['error'] = str(error)
            if session:
                report['text'] = request('POST', f'/session/{session}/execute/sync', {'script': 'return document.body.innerText', 'args': []})
                (output / 'failure.png').write_bytes(base64.b64decode(request('GET', f'/session/{session}/screenshot')))
            raise
        finally:
            if session:
                try:
                    request('DELETE', f'/session/{session}')
                except Exception as error:
                    report['cleanupError'] = str(error)
                    report['passed'] = False
            process.terminate()
            process.wait(timeout=10)  # tauri-driver Job Object reaps descendants.
            (output / 'result.json').write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding='utf-8')
        if not report.get('passed'):
            raise AssertionError('Desktop smoke or cleanup failed')
        summary = {'passed': report['passed'], 'evidence': str(output / 'result.json'), 'checks': report['checks']} if args.optimization_loop or args.optimization_saved or args.project_workflow or args.workflow_saved else report
        print(json.dumps(summary, ensure_ascii=True), flush=True)


if __name__ == '__main__':
    main()
