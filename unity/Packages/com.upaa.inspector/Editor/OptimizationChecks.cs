// Fixed validation commands. No arbitrary code, scene opening, save, or build.
using System;
using System.IO;
using System.Linq;
using Newtonsoft.Json.Linq;
using Unity.Pipeline.Commands;
using UnityEditor;
using UnityEditor.Compilation;
using UnityEngine;
namespace UPAA.Inspector {
 [InitializeOnLoad] public static class OptimizationChecks {
  static string Root => Path.GetFullPath(Path.Combine(Application.dataPath,".."));
  static string Journal => Path.Combine(Root,"Library","UPAA-optimization-check.json");
  static JObject job;
  static bool journalDirty;
  static string journalError;
#if UNITY_EDITOR_WIN
  [System.Runtime.InteropServices.DllImport("kernel32.dll", CharSet=System.Runtime.InteropServices.CharSet.Unicode, SetLastError=true)]
  static extern bool MoveFileEx(string source,string destination,int flags);
#endif
  public static Action<string[],Action<JObject>> RunTests;
  public static Action CancelTests;
  static OptimizationChecks() {
   try { if(File.Exists(Journal)) job=JObject.Parse(File.ReadAllText(Journal)); } catch {job=null;}
   CompilationPipeline.assemblyCompilationFinished += (assembly,messages)=> {
    if(job==null || (string)job["phase"]!="compiling")return;
    var errors=(JArray)job["errors"];
    foreach(var m in messages.Where(m=>m.type==CompilerMessageType.Error)) {
     if(errors.Count<100)errors.Add(new JObject{["file"]=m.file,["line"]=m.line,["message"]=m.message.Length>1000?m.message.Substring(0,1000):m.message});else job["errorsTruncated"]=true;
    } Persist();
   };
   CompilationPipeline.compilationFinished += _=> {if(job!=null && (string)job["phase"]=="compiling"){job["phase"]="afterCompilation";Persist();}};
   EditorApplication.update += Tick;
  }
  static JObject Envelope() => new JObject{["protocolVersion"]=1,["checkProtocolVersion"]=1,["projectRoot"]=Root,["status"]="ready",["unityVersion"]=Application.unityVersion,["sampledAt"]=DateTime.UtcNow.ToString("O"),["targetPlatform"]=EditorUserBuildSettings.activeBuildTarget.ToString()};
  [UnityEditor.Callbacks.DidReloadScripts]
  static void AfterReload(){
   // Compilation can reload the domain before the old domain's final callback is durable.
   // A successfully loaded script domain completes this explicitly requested compile.
   if(job!=null&&(string)job["phase"]=="compiling"&&(bool?)job["compileRequested"]==true){job["phase"]="afterCompilation";Persist();}
  }
  static void Persist(){
   if(job==null)return;journalDirty=true;
   try{var temp=Journal+".tmp";File.WriteAllText(temp,job.ToString());
#if UNITY_EDITOR_WIN
    if(!MoveFileEx(temp,Journal,0x1|0x8))throw new IOException(new System.ComponentModel.Win32Exception(System.Runtime.InteropServices.Marshal.GetLastWin32Error()).Message);
#else
    if(File.Exists(Journal))File.Replace(temp,Journal,null);else File.Move(temp,Journal);
#endif
    journalDirty=false;journalError=null;}
   catch(IOException e){journalError=e.Message;}catch(UnauthorizedAccessException e){journalError=e.Message;}
  }
  static void End(string status,string reason=null){job["phase"]="finished";job["checkStatus"]=status;job["reason"]=reason;Persist();}
  static void Tick(){
   if(journalDirty)Persist();
   if(job==null||(string)job["phase"]=="finished")return;
   if(DateTime.UtcNow.Ticks-(long)(job["startedTicks"]??0)>TimeSpan.FromMinutes(5).Ticks){CancelTests?.Invoke();End("unavailable","检查超时");return;}
   if(EditorApplication.isCompiling||EditorApplication.isUpdating)return;
   if((string)job["phase"]!="afterCompilation")return;
   if(((JArray)job["errors"]).Count>0){End("failed","C# 或 Shader 检查失败");return;}
   if((bool?)job["shaderCoverageIncomplete"]==true){End("unavailable","已导入相关代码，但 HLSL/CGINC/Compute 的依赖 Shader 或所有变体错误覆盖不完整；需在目标平台核对");return;}
   var tests=((JArray)job["tests"]).Values<string>().ToArray();
   if(tests.Length==0){End("passed","编译检查通过，未选择 EditMode 测试");return;}
   if(RunTests==null){End("unavailable","当前工程未提供 EditMode 测试适配器");return;}
   job["phase"]="testing";Persist();
   try{RunTests(tests,result=>{if(job==null||(string)job["phase"]!="testing")return;job["testsResult"]=result;End((string)result["status"]);});}catch(Exception e){End("unavailable",e.Message);}
  }
  [CliCommand("upaa_check_start","Compile current code and check selected shaders/tests; no scene save or build",MainThreadRequired=true)]
  public static JObject Start([CliArg("request","Fixed check request JSON")]string request="{}") {
   var r=JObject.Parse(request);var output=Envelope();
   if(EditorApplication.isCompiling||EditorApplication.isUpdating||EditorApplication.isPlayingOrWillChangePlaymode){output["checkStatus"]="unavailable";output["reason"]="Editor 忙碌或 Play Mode";return output;}
   if(job!=null&&(string)job["phase"]!="finished")throw new InvalidOperationException("Existing check still running");
   var id=(string)r["checkId"];if(!Guid.TryParse(id,out _))throw new ArgumentException("Invalid check ID");
   var paths=(r["paths"] as JArray??new JArray()).Values<string>().Distinct().ToArray();
   if(paths.Length>400)throw new ArgumentException("Too many files");
   foreach(var path in paths){
    if(!(path.StartsWith("Assets/",StringComparison.Ordinal)||path.StartsWith("Packages/",StringComparison.Ordinal))||path.Contains('\\')||path.Contains(':')||path.Split('/').Any(s=>s==".."||s=="."||s==""))throw new ArgumentException("Invalid path");
    if(!new[]{".cs",".shader",".hlsl",".cginc",".compute"}.Contains(Path.GetExtension(path)))throw new ArgumentException("Not code");
    var absolute=Path.Combine(Root,path);if(!File.Exists(absolute))throw new ArgumentException("Missing existing code");
    for(var p=absolute;p!=Root&&!string.IsNullOrEmpty(p);p=Path.GetDirectoryName(p))if((File.GetAttributes(p)&FileAttributes.ReparsePoint)!=0)throw new ArgumentException("Links not allowed");
   }
   var tests=r["tests"] as JArray??new JArray();if(tests.Count>100||tests.Values<string>().Any(t=>string.IsNullOrWhiteSpace(t)||t.Length>300))throw new ArgumentException("Invalid tests");
   job=new JObject{["id"]=id,["phase"]="compiling",["checkStatus"]="running",["startedAt"]=DateTime.UtcNow.ToString("O"),["startedTicks"]=DateTime.UtcNow.Ticks,["errors"]=new JArray(),["tests"]=tests.DeepClone(),["paths"]=new JArray(paths),["shaderCoverageIncomplete"]=paths.Any(p=>new[]{".hlsl",".cginc",".compute"}.Contains(Path.GetExtension(p))),["shaderScope"]="仅查询已导入 Shader 当前可取得的错误，不证明所有平台及变体通过"};Persist();if(journalDirty){End("unavailable","无法保存检查记录："+journalError);output["check"]=job.DeepClone();output["journalDurable"]=!journalDirty;return output;}
   // Defer imports/compile so the response reaches the caller before a domain reload.
   EditorApplication.delayCall+=()=>{
    if(job==null||(string)job["id"]!=id||(string)job["phase"]!="compiling")return;
    try{
     foreach(var path in paths){AssetDatabase.ImportAsset(path,ImportAssetOptions.ForceUpdate);if(path.EndsWith(".shader",StringComparison.Ordinal)){var shader=AssetDatabase.LoadAssetAtPath<Shader>(path);if(shader)foreach(var e in ShaderUtil.GetShaderMessages(shader)){if(e.severity.ToString()=="Error")((JArray)job["errors"]).Add(new JObject{["file"]=path,["line"]=e.line,["message"]=e.message});}}}
     job["compileRequested"]=true;Persist();if(journalDirty){End("unavailable","编译前检查记录无法保存");return;}CompilationPipeline.RequestScriptCompilation();
    }catch(Exception e){End("unavailable",e.Message);}
   };
   output["check"]=job.DeepClone();output["journalDurable"]=!journalDirty;output["journalError"]=journalError;return output;
  }
  [CliCommand("upaa_check_status","Read persisted compilation/test result",MainThreadRequired=true)]
  public static JObject Status([CliArg("request","Check ID JSON")]string request="{}") {
   var r=JObject.Parse(request);var output=Envelope();if(job==null||(string)r["checkId"]!=(string)job["id"])throw new ArgumentException("Unknown check ID");output["check"]=job.DeepClone();output["journalDurable"]=!journalDirty;output["journalError"]=journalError;return output;
  }
  [CliCommand("upaa_check_cancel","Cancel pending check without reverting code",MainThreadRequired=true)]
  public static JObject Cancel([CliArg("request","Check ID JSON")]string request="{}") {
   var r=JObject.Parse(request);if(job!=null&&(string)r["checkId"]==(string)job["id"]&&(string)job["phase"]!="finished"){CancelTests?.Invoke();End("unavailable","已取消，编译或导入无法撤销");}return Envelope();
  }
 }
}
