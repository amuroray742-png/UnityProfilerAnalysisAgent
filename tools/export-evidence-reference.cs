// Run with Unity CLI eval_file in an isolated Editor after loading a capture.
// Writes only to the OS temporary directory. Never modifies project assets.
var indices = new[] { 0, 1, 127, 511, 999, 1500, 1999 };
var frames = new System.Collections.Generic.List<object>();
long samplesChecked = 0, stackSamples = 0, flowEvents = 0;
foreach (int index in indices) {
    var threads = new System.Collections.Generic.List<object>();
    for (int ti = 0;; ti++) {
        using (var v = UnityEditorInternal.ProfilerDriver.GetRawFrameDataView(index, ti)) {
            if (!v.valid) break;
            var rows = new System.Collections.Generic.List<object>();
            var flows = new System.Collections.Generic.List<UnityEditor.Profiling.RawFrameDataView.FlowEvent>();
            v.GetFlowEvents(flows); flowEvents += flows.Count;
            var stack = new System.Collections.Generic.List<ulong>();
            for (int si = 0; si < v.sampleCount; si++) {
                samplesChecked++; stack.Clear(); v.GetSampleCallstack(si, stack);
                if (stack.Count > 0) stackSamples++;
                int count = v.GetSampleMetadataCount(si);
                if (count == 0) continue;
                var definitions = v.GetMarkerMetadataInfo(v.GetSampleMarkerId(si));
                var fields = new System.Collections.Generic.List<object>();
                for (int fi = 0; fi < count; fi++) {
                    var bytes = v.GetSampleMetadataAsSpan<byte>(si, fi).ToArray();
                    string numeric = null;
                    if (definitions != null && fi < definitions.Length) {
                        int type = (int)definitions[fi].type;
                        if (type >= 1 && type <= 5) numeric = v.GetSampleMetadataAsLong(si, fi).ToString(System.Globalization.CultureInfo.InvariantCulture);
                        if (type == 6) numeric = v.GetSampleMetadataAsFloat(si, fi).ToString("R", System.Globalization.CultureInfo.InvariantCulture);
                        if (type == 7) numeric = v.GetSampleMetadataAsDouble(si, fi).ToString("R", System.Globalization.CultureInfo.InvariantCulture);
                    }
                    fields.Add(new { index = fi, numeric, rawHex = System.BitConverter.ToString(bytes).Replace("-", "").ToLowerInvariant(),
                        definition = definitions != null && fi < definitions.Length ? (object)new { type=(int)definitions[fi].type, unit=(int)definitions[fi].unit, name=definitions[fi].name } : null });
                }
                bool counter = (((int)v.GetSampleFlags(si)) & 128) != 0;
                rows.Add(new { sampleIndex = si, name = v.GetSampleName(si), counter, fields });
            }
            threads.Add(new { threadIndex=ti, threadName=v.threadName, sampleCount=v.sampleCount, rows });
        }
    }
    frames.Add(new { frameIndex=index, threads });
}
var path=System.IO.Path.Combine(System.IO.Path.GetTempPath(), "upaa-evidence-reference.json");
System.IO.File.WriteAllText(path, Newtonsoft.Json.JsonConvert.SerializeObject(new { editorVersion=UnityEngine.Application.unityVersion, samplesChecked, stackSamples, flowEvents, frames }));
return new { path, frames=frames.Count, samplesChecked, stackSamples, flowEvents };
