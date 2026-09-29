// Unity CLI eval_file in an isolated Editor after loading a capture.
// Read-only recording query; writes a reference to the OS temporary directory.
var frames = new System.Collections.Generic.List<object>();
long total = 0;
for (int fi = UnityEditorInternal.ProfilerDriver.firstFrameIndex; fi <= UnityEditorInternal.ProfilerDriver.lastFrameIndex; fi++) {
    var threads = new System.Collections.Generic.List<object>();
    for (int ti = 0;; ti++) {
        using (var v = UnityEditorInternal.ProfilerDriver.GetRawFrameDataView(fi, ti)) {
            if (!v.valid) break;
            var events = new System.Collections.Generic.List<UnityEditor.Profiling.RawFrameDataView.FlowEvent>();
            v.GetFlowEvents(events);
            total += events.Count;
            if (events.Count > 0) threads.Add(new { threadIndex=ti, events });
        }
    }
    frames.Add(new { frameIndex=fi, threads });
}
var path = System.IO.Path.Combine(System.IO.Path.GetTempPath(), "upaa-flow-reference.json");
System.IO.File.WriteAllText(path, Newtonsoft.Json.JsonConvert.SerializeObject(new { editorVersion=UnityEngine.Application.unityVersion, total, frames }));
return new { path, total, frames=frames.Count, types=System.Enum.GetNames(typeof(UnityEditor.Profiling.RawFrameDataView.FlowEvent).GetField("FlowEventType").FieldType) };
