// Run with Unity CLI/MCP eval_file after loading the capture in Profiler.
// This exports counter references only; it is not an application dump input.
var names = new[] { "Draw Calls Count", "SetPass Calls Count", "Batches Count", "Triangles Count", "Vertices Count" };
var frames = new System.Collections.Generic.List<object>();
int first = UnityEditorInternal.ProfilerDriver.firstFrameIndex;
int last = UnityEditorInternal.ProfilerDriver.lastFrameIndex;
if (first < 0 || last < first) throw new System.InvalidOperationException("Load a capture in Profiler first");
for (int index = first; index <= last; index++) {
    using (var view = UnityEditorInternal.ProfilerDriver.GetRawFrameDataView(index, 0)) {
        if (!view.valid) throw new System.InvalidOperationException("Invalid frame: " + index);
        var markers = new System.Collections.Generic.List<UnityEditor.Profiling.FrameDataView.MarkerInfo>();
        view.GetMarkers(markers);
        var counters = new System.Collections.Generic.List<object>();
        foreach (var name in names) {
            bool available = false; long value = 0;
            foreach (var marker in markers) {
                if (marker.name != name || !view.HasCounterValue(marker.id)) continue;
                long observed = view.GetCounterValueAsLong(marker.id);
                if (observed < 0 || (available && value != observed))
                    throw new System.InvalidOperationException("Ambiguous/negative counter at frame " + index + ": " + name);
                available = true; value = observed;
            }
            counters.Add(new { name, available, value });
        }
        frames.Add(new { frame_index = index, counters });
    }
}
var path = System.IO.Path.Combine(UnityEngine.Application.temporaryCachePath, "render-counter-reference.json");
System.IO.File.WriteAllText(path, Newtonsoft.Json.JsonConvert.SerializeObject(new { editor_version = UnityEngine.Application.unityVersion, frames }));
return new { path, frames = frames.Count };
