// Read-only inventory of the recording already loaded in an isolated Editor.
var first=UnityEditorInternal.ProfilerDriver.firstFrameIndex;
var last=UnityEditorInternal.ProfilerDriver.lastFrameIndex;
if(first<0 || last<first) throw new System.InvalidOperationException("Load a recording first");
long samples=0,stacks=0,flows=0,metadataSamples=0; int frames=0,threads=0;
var names=new System.Collections.Generic.HashSet<string>();
for(int frame=first;frame<=last;frame++) {
    frames++;
    for(int ti=0;;ti++) using(var v=UnityEditorInternal.ProfilerDriver.GetRawFrameDataView(frame,ti)) {
        if(!v.valid)break;threads++;
        var events=new System.Collections.Generic.List<UnityEditor.Profiling.RawFrameDataView.FlowEvent>();
        v.GetFlowEvents(events);flows+=events.Count;
        var stack=new System.Collections.Generic.List<ulong>();
        for(int si=0;si<v.sampleCount;si++) {
            samples++;stack.Clear();v.GetSampleCallstack(si,stack);if(stack.Count>0)stacks++;
            if(v.GetSampleMetadataCount(si)>0)metadataSamples++;
            if((((int)v.GetSampleFlags(si))&128)!=0)names.Add(v.GetSampleName(si));
        }
    }
}
var result=new {editorVersion=UnityEngine.Application.unityVersion,first,last,frames,threads,samples,stackSamples=stacks,flowEvents=flows,metadataSamples,counters=names.OrderBy(n=>n).ToArray()};
var path=System.IO.Path.Combine(System.IO.Path.GetTempPath(),"upaa-feature-inventory.json");
System.IO.File.WriteAllText(path,Newtonsoft.Json.JsonConvert.SerializeObject(result,Newtonsoft.Json.Formatting.Indented));
return new {path,frames,threads,samples,stackSamples=stacks,flowEvents=flows,metadataSamples,counters=names.Count};
