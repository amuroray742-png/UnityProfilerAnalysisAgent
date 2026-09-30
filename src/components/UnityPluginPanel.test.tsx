import {render,screen,fireEvent,waitFor,cleanup,act} from '@testing-library/react';
import {afterEach,beforeEach,it,expect,vi} from 'vitest';
import {invoke} from '@tauri-apps/api/core';
import {UnityPluginPanel} from './UnityPluginPanel';
vi.mock('@tauri-apps/api/core',()=>({invoke:vi.fn()}));
afterEach(cleanup);beforeEach(()=>vi.clearAllMocks());
const state=(s='missing')=>({state:s,reason:'公开测试',version:'0.1.0',bundleHash:'abc',record:null,lockWarning:null,editor:{status:'unavailable',reason:'未连接'}});
it('checks and installs only the bound project, without another confirmation',async()=>{
 vi.mocked(invoke).mockResolvedValueOnce(state()).mockResolvedValueOnce(state('installed'));
 render(<UnityPluginPanel projectId="p" busy={false} onBusy={()=>{}}/>);
 fireEvent.click(screen.getByText('重新检查'));fireEvent.click(await screen.findByText('安装到工程'));
 expect(await screen.findByText(/插件文件已安装 ·/)).toBeInTheDocument();expect(screen.getByText(/Editor 尚未连接/)).toBeInTheDocument();
 expect(invoke).toHaveBeenLastCalledWith('workflow_command',{action:{op:'pluginInstall',projectId:'p'}});
});
it('migration and interrupted installs have explicit actions; conflict cannot overwrite',async()=>{
 vi.mocked(invoke).mockResolvedValueOnce(state('migrate')).mockResolvedValueOnce(state('conflict'));
 render(<UnityPluginPanel projectId="p" busy={false} onBusy={()=>{}}/>);fireEvent.click(screen.getByText('重新检查'));
 fireEvent.click(await screen.findByText('迁移到工程内'));await screen.findByText(/插件存在冲突/);expect(screen.queryByText('安装到工程')).toBeNull();
});
it('busy disables inspection and installation',()=>{
 render(<UnityPluginPanel projectId="p" busy onBusy={()=>{}}/>);expect(screen.getByText('重新检查')).toBeDisabled();
});
it('failed installation retains status and permits retry',async()=>{
 vi.mocked(invoke).mockResolvedValueOnce(state()).mockRejectedValueOnce('磁盘写入失败');render(<UnityPluginPanel projectId="p" busy={false} onBusy={()=>{}}/>);
 fireEvent.click(screen.getByText('重新检查'));fireEvent.click(await screen.findByText('安装到工程'));expect(await screen.findByRole('alert')).toHaveTextContent('磁盘写入失败');expect(screen.getByText('安装到工程')).toBeEnabled();
});
it('late result is discarded after switching projects',async()=>{
 let finish!:(v:unknown)=>void;vi.mocked(invoke).mockImplementation(()=>new Promise(r=>{finish=r;}) as any);
 const {rerender}=render(<UnityPluginPanel projectId="a" busy={false} onBusy={()=>{}}/>);fireEvent.click(screen.getByText('重新检查'));await waitFor(()=>expect(finish).toBeDefined());
 rerender(<UnityPluginPanel projectId="b" busy={false} onBusy={()=>{}}/>);await act(async()=>finish(state('installed')));expect(screen.queryByText(/插件文件已安装 ·/)).toBeNull();
});
