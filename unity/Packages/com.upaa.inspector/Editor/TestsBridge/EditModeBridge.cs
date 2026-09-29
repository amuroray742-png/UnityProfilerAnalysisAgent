using System;
using System.Linq;
using Newtonsoft.Json.Linq;
using UnityEditor;
using UnityEditor.TestTools.TestRunner.Api;
using UnityEngine;
namespace UPAA.Inspector {
 [InitializeOnLoad] public sealed class EditModeBridge:ICallbacks {
  static TestRunnerApi api;static Action<JObject> done;static string job;static string[] selected;static JArray results;
  static EditModeBridge(){OptimizationChecks.RunTests=Run;OptimizationChecks.CancelTests=()=>{if(job!=null)TestRunnerApi.CancelTestRun(job);};}
  static void Run(string[] tests,Action<JObject> callback){api=ScriptableObject.CreateInstance<TestRunnerApi>();done=callback;selected=tests;results=new JArray();api.RegisterCallbacks(new EditModeBridge());job=api.Execute(new ExecutionSettings(new Filter{testMode=TestMode.EditMode,testNames=tests}));}
  public void RunStarted(ITestAdaptor tests){}
  public void TestStarted(ITestAdaptor test){}
  public void TestFinished(ITestResultAdaptor result){if(!result.HasChildren&&results.Count<100)results.Add(new JObject{["name"]=result.FullName,["state"]=result.ResultState,["message"]=result.Message?.Substring(0,Math.Min(result.Message.Length,1000))});}
  public void RunFinished(ITestResultAdaptor result){var missing=selected.Where(s=>!results.Any(r=>(string)r["name"]==s)).ToArray();done?.Invoke(new JObject{["status"]=result.FailCount>0?"failed":missing.Length>0||result.PassCount==0||result.SkipCount>0||result.InconclusiveCount>0?"unavailable":"passed",["passed"]=result.PassCount,["failed"]=result.FailCount,["missing"]=new JArray(missing),["results"]=results});job=null;done=null;}
 }
}
