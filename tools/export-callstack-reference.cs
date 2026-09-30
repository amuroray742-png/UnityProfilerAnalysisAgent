// Execute read-only against a recording already loaded in an isolated Editor.
// Does not enable recording or alter project settings. Output goes to temp only.
int first = UnityEditorInternal.ProfilerDriver.firstFrameIndex;
int last = UnityEditorInternal.ProfilerDriver.lastFrameIndex;
if (first < 0 || last < first) throw new System.InvalidOperationException("Load a capture first");
int start = first;
var requested = System.Environment.GetEnvironmentVariable("UPAA_STACK_START_FRAME");
if (!string.IsNullOrEmpty(requested) && (!int.TryParse(requested, out start) || start < first || start > last))
    throw new System.ArgumentException("UPAA_STACK_START_FRAME is outside loaded recording");
int stop = (int)System.Math.Min((long)last, (long)start + 63);
var frames = new System.Collections.Generic.List<object>();
long inspected = 0, stackSamples = 0, addresses = 0;
bool truncated = false;
for (int fi = start; fi <= stop && !truncated; fi++) {
    var threads = new System.Collections.Generic.List<object>();
    for (int ti = 0; !truncated; ti++) using (var view = UnityEditorInternal.ProfilerDriver.GetRawFrameDataView(fi, ti)) {
        if (!view.valid) break;
        var rows = new System.Collections.Generic.List<object>();
        var stack = new System.Collections.Generic.List<ulong>();
        for (int si = 0; si < view.sampleCount; si++) {
            if (inspected >= 1000000 || addresses >= 500000) { truncated = true; break; }
            inspected++; stack.Clear(); view.GetSampleCallstack(si, stack);
            if (stack.Count == 0) continue;
            stackSamples++;
            var entries = new System.Collections.Generic.List<object>();
            int take = System.Math.Min(System.Math.Min(stack.Count, 2048), (int)(500000 - addresses));
            for (int depth = 0; depth < take; depth++) {
                ulong addr = stack[depth]; object symbol = null; string error = null;
                try { symbol = view.ResolveMethodInfo(addr); }
                catch (System.Exception ex) { error = ex.Message; }
                entries.Add(new { depth, address = addr.ToString(System.Globalization.CultureInfo.InvariantCulture), symbol, error });
            }
            addresses += take;
            rows.Add(new { sampleIndex = si, markerId = view.GetSampleMarkerId(si), marker = view.GetSampleName(si),
                recordedDepth = stack.Count, truncated = take != stack.Count, entries });
        }
        threads.Add(new { threadIndex = ti, threadId = view.threadId.ToString(), threadName = view.threadName, sampleCount = view.sampleCount, rows });
    }
    frames.Add(new { editorFrameIndex = fi, frameIndex = fi - first, threads });
}
var path = System.IO.Path.Combine(System.IO.Path.GetTempPath(), "upaa-callstack-" + System.Guid.NewGuid().ToString("N") + ".json");
var result = new { editorVersion = UnityEngine.Application.unityVersion,
    recordingVersion = System.Environment.GetEnvironmentVariable("UPAA_RECORDING_VERSION") ?? "unknown",
    captureSettings = System.Environment.GetEnvironmentVariable("UPAA_CAPTURE_SETTINGS") ?? "unknown (not inferred from current Editor settings)",
    first, last, start, stop, inspected, stackSamples, addresses, truncated, frames };
System.IO.File.WriteAllText(path, Newtonsoft.Json.JsonConvert.SerializeObject(result, Newtonsoft.Json.Formatting.Indented));
return new { path, inspected, stackSamples, addresses, truncated, requestedFrames = stop - start + 1 };
