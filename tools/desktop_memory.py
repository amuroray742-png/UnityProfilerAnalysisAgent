"""Sample the launched Tauri process tree; psutil is needed only in this mode."""
import hashlib
import json
import threading
import time
import psutil


def measure(js, driver_pid, application, input_path, output, workload=None, repeats=3):
    target = str(application.resolve()).casefold()
    matches = [p for p in psutil.Process(driver_pid).children(recursive=True)
               if p.exe().casefold() == target]
    if len(matches) != 1:
        raise RuntimeError(f'Expected exactly one launched app, found {len(matches)}')
    app = matches[0]
    digest = hashlib.sha256()
    with input_path.open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            digest.update(block)
    stop = threading.Event()
    samples, errors = [], []
    stage = ['baseline']
    started = time.monotonic()

    def sample():
        while not stop.is_set():
            try:
                members = [app] + app.children(recursive=True)
                rows = []
                for proc in members:
                    try:
                        mem = proc.memory_info()
                        rows.append({'pid': proc.pid, 'name': proc.name(), 'rss': mem.rss,
                                     'private': mem.private})
                    except psutil.NoSuchProcess:
                        continue
                if not any(p['pid'] == app.pid for p in rows):
                    raise RuntimeError('Application exited during measurement')
                samples.append({'seconds': time.monotonic() - started, 'stage': stage[0],
                                'rss': sum(p['rss'] for p in rows),
                                'private': sum(p['private'] for p in rows), 'processes': rows})
            except Exception as exc:
                errors.append(str(exc))
                return
            stop.wait(.05)

    thread = threading.Thread(target=sample, daemon=True)
    thread.start()
    runs = []
    diagnostics = {}
    try:
        time.sleep(1)
        if workload is not None and hasattr(workload, 'probe'):
            diagnostics['baseline'] = workload.probe()
        for index in range(repeats):
            stage[0] = f'import-query-{index + 1}'
            if workload is not None:
                runs.append(workload(index, stage))
                stage[0] = f'released-{index + 1}'
                time.sleep(2)
                continue
            js('''
                window.__measureResult=null;
                (async()=>{
                    const invoke=window.__TAURI_INTERNALS__.invoke, started=performance.now();
                    const upload=await invoke('upload',{filePath:arguments[0]});
                    const fileId=upload.fileId;
                    let summary;
                    try {
                        const snapshot=await invoke('analyze',{fileId});
                        const analyzed=performance.now();
                        const timeline=snapshot.cpu.frameTimeline;
                        if(!timeline.length) throw new Error('No frames');
                        const indices=[timeline[0].frameIndex,timeline[timeline.length-1].frameIndex];
                        const pages=await Promise.all(indices.map(frameIndex=>invoke('cpu_hierarchy',{
                          fileId,frameIndex,threadIndex:null,start:0,limit:200,maxDepth:8})));
                        if(pages.some(p=>p.samples.length>200)) throw new Error('Query limit exceeded');
                        summary={frames:snapshot.meta.frameCount,declared:snapshot.meta.declaredFrameCount,
                          analyzeMs:analyzed-started,queryMs:performance.now()-analyzed,
                          queryFrames:indices,returned:pages.map(p=>p.samples.length)};
                    } finally {await invoke('release_file',{fileId});}
                    let rejected=false;
                    try {await invoke('frame_details',{fileId,frameIndex:summary.queryFrames[0],start:0,limit:1});}
                    catch {rejected=true;}
                    if(!rejected) throw new Error('Released source still queryable');
                    return summary;
                })().then(result=>window.__measureResult={result},error=>window.__measureResult={error:String(error)});
            ''', str(input_path.resolve()))
            deadline = time.monotonic() + 180
            while True:
                result = js('return window.__measureResult')
                if result is not None:
                    break
                if time.monotonic() > deadline:
                    raise TimeoutError('Import/query timed out')
                time.sleep(.1)
            if 'error' in result:
                raise RuntimeError(result['error'])
            runs.append(result['result'])
            stage[0] = f'released-{index + 1}'
            time.sleep(2)  # Observe natural collection; do not force WebView GC.
        if workload is not None and hasattr(workload, 'probe'):
            stage[0] = 'natural-idle-30s'
            print('Heap probe: observing 30 seconds natural idle', flush=True)
            time.sleep(30)
            diagnostics['afterNaturalIdle'] = workload.probe()
            stage[0] = 'diagnostic-forced-gc'
            workload.collect_garbage()
            time.sleep(2)
            diagnostics['afterDiagnosticGc'] = workload.probe()
    finally:
        stop.set()
        thread.join(timeout=5)
        (output / 'memory-samples.json').write_text(json.dumps(samples), encoding='utf-8')
    if errors or thread.is_alive() or not samples:
        raise RuntimeError(f'Memory sampling failed: {errors}')
    phases = {}
    for name in dict.fromkeys(s['stage'] for s in samples):
        rows = [s for s in samples if s['stage'] == name]
        phases[name] = {'samples':len(rows), 'peakRss':max(s['rss'] for s in rows),
                        'peakPrivate':max(s['private'] for s in rows),
                        'lastRss':rows[-1]['rss'], 'lastPrivate':rows[-1]['private']}
    return {'inputBytes':input_path.stat().st_size, 'inputSha256':digest.hexdigest(),
            'psutilVersion':psutil.__version__, 'sampleIntervalMs':50, 'runs':runs, 'phases':phases,
            'heapDiagnostics':diagnostics, 'sampleCount':len(samples), 'peakRss':max(s['rss'] for s in samples),
            'peakPrivate':max(s['private'] for s in samples),
            'scope':('Tauri app and descendants; rendered overview/tree/reset; native picker response substituted; excludes Agent/drivers' if workload else 'Tauri app and descendants; IPC only; excludes React results rendering, Agent/drivers') + '; summed working sets may double-count shared pages; sampling may miss brief peaks'}
