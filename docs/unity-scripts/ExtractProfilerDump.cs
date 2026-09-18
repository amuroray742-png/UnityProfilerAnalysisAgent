// ExtractProfilerDump.cs
//
// 穷尽导出 .data 文件中所有可见字段（用 Unity 6000.3 的官方
// UnityEditor.Profiling.RawFrameDataView API），输出 JSON。
// 跑法：Unity Editor → Window → General → Search "Extract Profiler Dump"
// 或命令行：Unity.exe -batchMode -projectPath <project> -executeMethod ExtractProfilerDump.RunAll -inputFile <path.data> -outputFile <path.json>
//
// 目的：让我们拿到「真实值」后比对 .data 二进制，反推 Unity 6 wire format。

using System;
using System.Collections.Generic;
using System.IO;
using System.Text;
using UnityEditor;
using UnityEditor.Profiling;
using UnityEditorInternal.Profiling;
using UnityEngine;
using ProfilerDriver = UnityEditorInternal.Profiling.ProfilerDriver;

namespace ExtractProfilerDump
{
    public static class ExtractProfilerDump
    {
        // 菜单入口：Tools > Extract Profiler Dump（GUI）
        [MenuItem("Tools/Extract Profiler Dump…")]
        public static void OpenDialog()
        {
            string dataPath = EditorUtility.OpenFilePanel(
                "选择 .data 文件",
                "",
                "data");
            if (string.IsNullOrEmpty(dataPath)) return;
            string outputPath = EditorUtility.SaveFilePanel(
                "保存 JSON 输出",
                Path.GetDirectoryName(dataPath),
                Path.GetFileName(dataPath) + ".dump.json",
                "json");
            if (string.IsNullOrEmpty(outputPath)) return;
            RunExtraction(dataPath, outputPath);
        }

        public static void RunAll()
        {
            string inputFile = ParseArg("-inputFile");
            string outputFile = ParseArg("-outputFile");
            if (string.IsNullOrEmpty(inputFile))
            {
                Debug.LogError("[ExtractProfilerDump] missing -inputFile <path.data>");
                EditorApplication.Exit(2);
                return;
            }
            if (string.IsNullOrEmpty(outputFile))
            {
                outputFile = inputFile + ".dump.json";
            }
            RunExtraction(inputFile, outputFile);
        }

        static void RunExtraction(string inputFile, string outputFile)
        {
            if (!File.Exists(inputFile))
            {
                Debug.LogError($"[ExtractProfilerDump] input not found: {inputFile}");
                if (Application.isBatchMode) EditorApplication.Exit(3);
                return;
            }

            Debug.Log($"[ExtractProfilerDump] opening: {inputFile}");
            bool loaded = ProfilerDriver.LoadProfile(inputFile, false);
            Debug.Log($"[ExtractProfilerDump] loaded={loaded}, lastFrameIndex={ProfilerDriver.lastFrameIndex}");
            Debug.Log($"[ExtractProfilerDump] opened, lastFrameIndex={ProfilerDriver.lastFrameIndex}");

            var dump = new DumpRoot
            {
                input_file = inputFile,
                captured_at = DateTime.UtcNow.ToString("o"),
                unity_version = Application.unityVersion,
                frame_count = ProfilerDriver.lastFrameIndex + 1,
                frames = new List<FrameDump>(),
            };

            int frameCount = ProfilerDriver.lastFrameIndex + 1;
            int maxFrames = Math.Min(frameCount, 64); // 限制前 64 帧，避免 Editor 卡死
            for (int f = 0; f < maxFrames; f++)
            {
                dump.frames.Add(ExtractFrame(f));
                if ((f + 1) % 16 == 0)
                {
                    Debug.Log($"[ExtractProfilerDump] frame {f + 1}/{maxFrames} done");
                }
            }

            File.WriteAllText(outputFile,
                JsonUtility.ToJson(dump, prettyPrint: true),
                new UTF8Encoding(false));
            Debug.Log($"[ExtractProfilerDump] wrote {outputFile}");
            if (Application.isBatchMode) EditorApplication.Exit(0);
        }

        static FrameDump ExtractFrame(int frameIndex)
        {
            var fd = new FrameDump
            {
                frame_index = frameIndex,
                frame_time_ms = 0f,
                frame_gpu_time_ms = 0f,
                sample_count_total = 0,
                gc_alloc_bytes_total = 0,
                threads = new List<ThreadDump>(),
            };

            // 先用 ProfilerDriver 拿 frame 整体时间（用第一个 thread 读 frame-level metadata）
            for (int threadIdx = 0; ; threadIdx++)
            {
                using (var frame = ProfilerDriver.GetRawFrameDataView(frameIndex, threadIdx))
                {
                    if (!frame.valid)
                    {
                        break;
                    }
                    fd.frame_time_ms = (float)frame.frameTimeMs;
                    fd.frame_gpu_time_ms = (float)frame.frameGpuTimeMs;

                    var td = new ThreadDump
                    {
                        thread_index = threadIdx,
                        thread_id = frame.threadId,
                        thread_name = frame.threadName,
                        thread_group_name = frame.threadGroupName,
                        max_depth = frame.maxDepth,
                        markers = new List<MarkerDump>(),
                        samples = new List<SampleDump>(),
                        gc_alloc_total_bytes = 0,
                    };

                    // 收集所有 samples（含 GC.Alloc metadata）
                    // marker 名字直接来自 sample，无需调用 GetMarkers()
                    int sampleCount = frame.sampleCount;
                    fd.sample_count_total += sampleCount;
                    int gcAllocId = frame.GetMarkerId("GC.Alloc");
                    var markerSeen = new HashSet<int>();
                    for (int s = 0; s < sampleCount; s++)
                    {
                        int mid = frame.GetSampleMarkerId(s);
                        var sd = new SampleDump
                        {
                            sample_index = s,
                            marker_id = mid,
                            marker_name = frame.GetSampleName(s),
                            time_ms = (float)frame.GetSampleTimeMs(s),
                            start_time_ms = (float)frame.GetSampleStartTimeMs(s),
                            children_count = frame.GetSampleChildrenCount(s),
                            category_index = frame.GetSampleCategoryIndex(s),
                            metadata_count = frame.GetSampleMetadataCount(s),
                        };
                        if (gcAllocId != -1 && mid == gcAllocId)
                        {
                            int mc = frame.GetSampleMetadataCount(s);
                            if (mc > 0)
                            {
                                long bytes = frame.GetSampleMetadataAsLong(s, 0);
                                sd.gc_alloc_bytes = bytes;
                                td.gc_alloc_total_bytes += bytes;
                                fd.gc_alloc_bytes_total += bytes;
                            }
                        }
                        // 收集 marker（去重，按 id）
                        if (markerSeen.Add(mid))
                        {
                            td.markers.Add(new MarkerDump
                            {
                                id = mid,
                                name = sd.marker_name,
                                category_index = sd.category_index,
                                flags = "",
                            });
                        }
                        td.samples.Add(sd);
                    }
                    fd.threads.Add(td);
                }
            }
            return fd;
        }

        static string ParseArg(string name)
        {
            var args = Environment.GetCommandLineArgs();
            for (int i = 0; i < args.Length - 1; i++)
            {
                if (args[i] == name) return args[i + 1];
            }
            return null;
        }

        // JSON DTOs
        [Serializable] public class DumpRoot
        {
            public string input_file;
            public string captured_at;
            public string unity_version;
            public int frame_count;
            public List<FrameDump> frames;
        }
        [Serializable] public class FrameDump
        {
            public int frame_index;
            public float frame_time_ms;
            public float frame_gpu_time_ms;
            public int sample_count_total;
            public long gc_alloc_bytes_total;
            public List<ThreadDump> threads;
        }
        [Serializable] public class ThreadDump
        {
            public int thread_index;
            public ulong thread_id;
            public string thread_name;
            public string thread_group_name;
            public int max_depth;
            public List<MarkerDump> markers;
            public List<SampleDump> samples;
            public long gc_alloc_total_bytes;
        }
        [Serializable] public class MarkerDump
        {
            public int id;
            public string name;
            public ushort category_index;
            public string flags;
        }
        [Serializable] public class SampleDump
        {
            public int sample_index;
            public int marker_id;
            public string marker_name;
            public float time_ms;
            public float start_time_ms;
            public int children_count;
            public ushort category_index;
            public int metadata_count;
            public long gc_alloc_bytes;
        }
    }
}