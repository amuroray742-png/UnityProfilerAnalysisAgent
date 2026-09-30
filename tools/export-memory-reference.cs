// Run with Unity CLI/MCP eval_file after loading the capture in Profiler.
// Memory module Counter API reference. Compare existence AND values.
// A mismatch with sample observations is evidence to investigate, never overwrite.
// This is not an application input.
var names = new[] { "Total Used Memory", "Total Reserved Memory", "GC Used Memory", "GC Reserved Memory", "Gfx Used Memory", "Profiler Used Memory", "Profiler Reserved Memory", "System Used Memory" };
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
            counters.Add(new { name, available, value = available ? value.ToString(System.Globalization.CultureInfo.InvariantCulture) : null });
        }
        frames.Add(new { frame_index = index - first, counters });
    }
}
var path = System.IO.Path.Combine(UnityEngine.Application.temporaryCachePath, "memory-counter-reference-" + System.Guid.NewGuid().ToString("N") + ".json");
System.IO.File.WriteAllText(path, Newtonsoft.Json.JsonConvert.SerializeObject(new { editor_version = UnityEngine.Application.unityVersion, frames }));
return new { path, frames = frames.Count };
