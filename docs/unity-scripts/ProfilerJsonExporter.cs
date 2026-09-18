// ProfilerJsonExporter.cs
// 用法：复制到 Unity 工程的 Assets/Editor/ 下，等编译后菜单出
//       Tools > Profiler > Export Current Frame Stats to JSON
//
// 这不是 MCP / AI 集成脚本，只是把 ProfilerRecorder 收集的关键指标
// 导出成 JSON，让 Unity Profiler Analysis Agent 工具能解析。
//
// 数据来源：UnityEngine.Profiling.ProfilerRecorder
// 输出格式：本工具 parser/json.rs 兼容的 V1 schema

using System.IO;
using System.Text;
using UnityEditor;
using UnityEngine;
using UnityEngine.Profiling;

namespace UnityProfilerAnalysisAgent
{
    public static class ProfilerJsonExporter
    {
        // 工具 parser 关注的 Unity 6 marker 名
        static readonly string[] Markers = {
            "Main Thread",
            "PlayerLoop",
            "BehaviourUpdate",
            "FixedBehaviourUpdate",
            "PreLateUpdate.ScriptRunBehaviourLateUpdate",
            "Update.ScriptRunBehaviourUpdate",
            "CoroutinesDelayedCalls",
            "GC.Alloc",
            "GC.Collect",
            "UnsafeUtility.Malloc",
            "Camera.Render",
            "Render.Mesh",
            "Render.OpaqueGeometry",
            "Render.TransparentGeometry",
            "Render.UI",
            "Gfx.WaitForPresentOnGfxThread",
            "Gfx.WaitForRenderThread",
            "Gfx.PresentFrame",
            "WaitForTargetFPS",
            "RenderGraph.Execute",
            "RenderGraph.Compile",
            "Physics.Simulate",
            "Physics.FetchResults",
            "JobHandle.Complete",
        };

        [MenuItem("Tools/Profiler/Export Current Frame Stats to JSON")]
        public static void Export()
        {
            int sampleFrames = 60; // 收集多少帧
            var frameStats = new System.Collections.Generic.Dictionary<string, long>();
            var perMarkerSamples = new System.Collections.Generic.Dictionary<string, long[]>();

            foreach (var name in Markers)
            {
                using var rec = ProfilerRecorder.StartNew(
                    ProfilerCategory.Markers, name, sampleFrames,
                    ProfilerRecorderOptions.Default);

                // 让 Profiler 收集 sampleFrames 帧的数据
                // 这里仅作快照示例；生产应通过 ProfilerRecorderHandle + 帧回调
                perMarkerSamples[name] = new long[sampleFrames];
                long sum = 0;
                for (int i = 0; i < sampleFrames; i++)
                {
                    EditorApplication.QueuePlayerLoopUpdate();
                    rec.CollectOnlyProfilerSamples();
                    long v = rec.LastValue;
                    perMarkerSamples[name][i] = v;
                    sum += v;
                }
                frameStats[name] = sum;
            }

            // 序列化成 JSON（与 toolkit 兼容的 V1 schema）
            var sb = new StringBuilder();
            sb.Append("{\n");
            sb.Append("  \"meta\": {\n");
            sb.Append("    \"unityVersion\": \"").Append(Application.unityVersion).Append("\",\n");
            sb.Append("    \"platform\": \"").Append(Application.platform.ToString()).Append("\",\n");
            sb.Append("    \"exportedAtUtc\": \"").Append(System.DateTime.UtcNow.ToString("o")).Append("\",\n");
            sb.Append("    \"sampledFrames\": ").Append(sampleFrames).Append("\n");
            sb.Append("  },\n");
            sb.Append("  \"frames\": [\n");

            // 每帧一个 frame 对象
            for (int f = 0; f < sampleFrames; f++)
            {
                if (f > 0) sb.Append(",\n");
                sb.Append("    {\n");
                sb.Append("      \"index\": ").Append(f).Append(",\n");
                sb.Append("      \"durationMs\": ").Append(perMarkerSamples["Main Thread"][f] / 1e6).Append(",\n");
                sb.Append("      \"cpuMs\": ").Append(perMarkerSamples["Main Thread"][f] / 1e6).Append(",\n");
                sb.Append("      \"gcAllocBytes\": ").Append(perMarkerSamples["GC.Alloc"][f]).Append(",\n");
                sb.Append("      \"mainThreadSamples\": [\n");
                bool firstSample = true;
                foreach (var name in Markers)
                {
                    if (firstSample) firstSample = false; else sb.Append(",\n");
                    sb.Append("        { \"name\": \"").Append(name).Append("\", ");
                    sb.Append("\"totalMs\": ").Append(perMarkerSamples[name][f] / 1e6).Append(", ");
                    sb.Append("\"callCount\": 1, ");
                    sb.Append("\"maxMs\": ").Append(perMarkerSamples[name][f] / 1e6).Append(" }");
                }
                sb.Append("\n      ]\n");
                sb.Append("    }");
            }
            sb.Append("\n  ]\n");
            sb.Append("}\n");

            var path = EditorUtility.SaveFilePanel(
                "Export Profiler JSON", "", "profiler_export.json", "json");
            if (!string.IsNullOrEmpty(path))
            {
                File.WriteAllText(path, sb.ToString(), new UTF8Encoding(false));
                Debug.Log($"[ProfilerJsonExporter] 写入 {path} ({sampleFrames} 帧)");
            }
        }
    }
}