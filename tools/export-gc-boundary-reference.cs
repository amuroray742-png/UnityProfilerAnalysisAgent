// Run via Unity CLI eval_file in an isolated Editor with the capture already loaded.
// Hierarchy GC columns are the reference; raw metadata is recorded separately and
// must never be substituted for the allocation total (continuations repeat it).
var first = UnityEditorInternal.ProfilerDriver.firstFrameIndex;
var last = UnityEditorInternal.ProfilerDriver.lastFrameIndex;
if (first < 0 || last < first) throw new System.InvalidOperationException("Load a capture first");
var frames = new System.Collections.Generic.List<object>();
var boundaries = new System.Collections.Generic.List<object>();
var selected = new System.Collections.Generic.HashSet<int>(new[] { 266, 267, 1592, 1593, 1595, 1596, 1769, 1770 });
ulong total = 0;
ulong metadataTotal = 0;
for (int f = first; f <= last; f++) {
    var threads = new System.Collections.Generic.List<object>();
    ulong frameBytes = 0;
    for (int t = 0; ; t++) {
        using (var raw = UnityEditorInternal.ProfilerDriver.GetRawFrameDataView(f, t)) {
            if (!raw.valid) break;
            using (var tree = UnityEditorInternal.ProfilerDriver.GetHierarchyFrameDataView(f, t,
                UnityEditor.Profiling.HierarchyFrameDataView.ViewModes.Default,
                UnityEditor.Profiling.HierarchyFrameDataView.columnDontSort, false)) {
                if (!tree.valid) throw new System.InvalidOperationException("Invalid hierarchy");
                int root = tree.GetRootItemID();
                double observed = tree.GetItemColumnDataAsDouble(root, UnityEditor.Profiling.HierarchyFrameDataView.columnGcMemory);
                if (double.IsNaN(observed) || observed < 0 || observed > 9007199254740991d || observed != System.Math.Floor(observed))
                    throw new System.InvalidOperationException("Non-exact GC column");
                ulong bytes = checked((ulong)observed);
                ulong metadataBytes = 0;
                int gcMarker = raw.GetMarkerId("GC.Alloc");
                var samples = new System.Collections.Generic.List<object>();
                for (int s = 0; s < raw.sampleCount; s++) {
                    if (raw.GetSampleMarkerId(s) != gcMarker) continue;
                    int fields = raw.GetSampleMetadataCount(s);
                    if (fields != 1) throw new System.InvalidOperationException("GC metadata arity");
                    long value = raw.GetSampleMetadataAsLong(s, 0);
                    if (value < 0) throw new System.InvalidOperationException("Negative GC metadata");
                    metadataBytes = checked(metadataBytes + (ulong)value);
                    if (selected.Contains(f - first) && raw.threadGroupName == "AndroidLogcat" && (s == 1 || s == raw.sampleCount - 1)) {
                        samples.Add(new { sample_index = s, marker_id = raw.GetSampleMarkerId(s),
                            start_ns = raw.GetSampleStartTimeNs(s).ToString(), duration_ns = raw.GetSampleTimeNs(s).ToString(),
                            children = raw.GetSampleChildrenCount(s), metadata_bytes = value.ToString(), flags = raw.GetSampleFlags(s).ToString() });
                    }
                }
                threads.Add(new { thread_index = t, thread_id = raw.threadId.ToString(), name = raw.threadName,
                    group = raw.threadGroupName, sample_count = raw.sampleCount, gc_bytes = bytes.ToString(), metadata_bytes = metadataBytes.ToString() });
                frameBytes = checked(frameBytes + bytes);
                metadataTotal = checked(metadataTotal + metadataBytes);
                if (samples.Count > 0) {
                    // Default mode keeps individual samples. Locate sample 1's GC
                    // column independently of its (potentially repeated) metadata.
                    var pending = new System.Collections.Generic.Stack<int>();
                    var children = new System.Collections.Generic.List<int>();
                    var indices = new System.Collections.Generic.List<int>();
                    var columns = new System.Collections.Generic.List<object>();
                    pending.Push(root);
                    while (pending.Count > 0) {
                        int item = pending.Pop();
                        tree.GetItemRawFrameDataViewIndices(item, indices);
                        if (indices.Contains(1) || indices.Contains(raw.sampleCount - 1))
                            columns.Add(new { indices = indices.ToArray(), gc_bytes = tree.GetItemColumnDataAsDouble(item, UnityEditor.Profiling.HierarchyFrameDataView.columnGcMemory).ToString(System.Globalization.CultureInfo.InvariantCulture) });
                        tree.GetItemChildren(item, children);
                        foreach (int child in children) pending.Push(child);
                    }
                    boundaries.Add(new { frame_index = f - first, thread_index = t, thread_id = raw.threadId.ToString(), samples, columns });
                }
            }
        }
    }
    if (threads.Count == 0) throw new System.InvalidOperationException("Empty frame");
    total = checked(total + frameBytes);
    frames.Add(new { frame_index = f - first, gc_bytes = frameBytes.ToString(), threads });
}
var path = System.IO.Path.Combine(UnityEngine.Application.temporaryCachePath, "gc-boundary-reference-" + System.Guid.NewGuid().ToString("N") + ".json");
System.IO.File.WriteAllText(path, Newtonsoft.Json.JsonConvert.SerializeObject(new {
    editor_version = UnityEngine.Application.unityVersion, first_frame = first, last_frame = last,
    gc_bytes = total.ToString(), metadata_bytes = metadataTotal.ToString(), frames, boundaries }));
return new { path, frames = frames.Count, gc_bytes = total.ToString(), metadata_bytes = metadataTotal.ToString() };
