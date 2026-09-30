"""Install shipped plugin through real desktop IPC into disposable public projects."""
import base64,json,time,uuid,shutil,tempfile,os
from pathlib import Path

def run_plugin(js,request,root,output,report):
    project=Path(tempfile.mkdtemp(prefix='upaa-plugin-public-'))/'Project'
    shutil.copytree(root/'src-tauri/tests/fixtures/unity-project',project)
    manifest=project/'Packages/manifest.json'
    manifest.write_text(json.dumps({'dependencies':{'com.upaa.inspector':'file:C:/upaa-missing-public-plugin'},'scopedRegistries':[]}),encoding='utf-8')
    records=output/('records-'+uuid.uuid4().hex)
    def ipc(action):
        value=request('POST','/execute/async',{'script':"const done=arguments[arguments.length-1];window.__TAURI_INTERNALS__.invoke('workflow_command',{action:arguments[0]}).then(v=>done({value:v}),e=>done({error:String(e)}));",'args':[action]})
        if 'error' in value:raise AssertionError(value['error'])
        return value['value']
    p=ipc({'op':'create','root':str(project),'directory':str(records),'name':'公开插件迁移验收'})
    js('location.reload()');time.sleep(1)
    def click(label):
        assert js("const b=Array.from(document.querySelectorAll('button')).find(b=>b.textContent===arguments[0]);if(!b||b.disabled)return false;b.click();return true;",label),label
    def wait(text):
        deadline=time.monotonic()+60
        while not js('return document.body.innerText.includes(arguments[0])',text):
            if time.monotonic()>deadline:raise AssertionError(js('return document.body.innerText'))
            time.sleep(.2)
    click('重新检查');wait('迁移到工程内');click('迁移到工程内');wait('插件文件已安装')
    (output/'installed.png').write_bytes(base64.b64decode(request('GET','/screenshot')))
    s=ipc({'op':'pluginStatus','projectId':p['id']});assert s['state']=='installed',s
    assert 'com.upaa.inspector' not in json.loads(manifest.read_text(encoding='utf-8'))['dependencies']
    shipped=root/'unity/Packages/com.upaa.inspector'
    for f in shipped.rglob('*'):
        if f.is_file():assert (project/'Packages/com.upaa.inspector'/f.relative_to(shipped)).read_bytes()==f.read_bytes()
    s=ipc({'op':'pluginInstall','projectId':p['id']});assert s['state']=='installed'
    ipc({'op':'pluginRecords','projectId':p['id']})
    # Pure IPC change must not provide a frontend-controlled destination or cross-project identity.
    for a in [{'op':'pluginInstall','projectId':'wrong'}, {'op':'pluginInstall','projectId':p['id'],'root':'C:/other'}]:
        try:ipc(a)
        except (AssertionError,RuntimeError):pass
        else:raise AssertionError('invalid request accepted')
    report['checks']+=['rendered migration button installs embedded resources, preserves .meta and removes only local dependency','repeat install idempotent; read record; wrong project and unknown destination rejected']
    report['pluginProject']=str(project);report['pluginRecords']=str(records);report['pluginStatus']=s
    report['scope']='Release desktop embedded plugin migration through actual buttons; no Agent; Unity compilation checked separately'

    live_root=os.environ.get('UPAA_PLUGIN_CHECK_PROJECT')
    if live_root:
        live=Path(live_root).resolve()
        assert live.is_relative_to(Path(tempfile.gettempdir()).resolve()) and live.name.startswith('upaa-plugin-'), 'only public disposable plugin projects'
        request('POST','/execute/async',{'script':"const done=arguments[arguments.length-1];window.__TAURI_INTERNALS__.invoke('optimization_command',{action:{op:'close'}}).then(done,e=>done({error:String(e)}));",'args':[]})
        p=ipc({'op':'create','root':str(live),'directory':str(output/('connected-records-'+uuid.uuid4().hex)),'name':'公开迁移后 Editor 验收'})
        s=ipc({'op':'pluginStatus','projectId':p['id']})
        assert s['state']=='installed' and s['editor']['status']=='ready' and not s['lockWarning'],s
        try:ipc({'op':'pluginInstall','projectId':p['id']})
        except (AssertionError,RuntimeError) as e:assert '关闭' in str(e),e
        else:raise AssertionError('running Editor did not prevent install')
        js('location.reload()');time.sleep(1);click('重新检查');wait('Editor 已连接')
        (output/'connected.png').write_bytes(base64.b64decode(request('GET','/screenshot')))
        report['connectedPluginStatus']=s
        report['checks']+=['relocated public Unity 6000.3 Editor ready shown separately, embedded relative lock accepted, live Editor rejects install']
