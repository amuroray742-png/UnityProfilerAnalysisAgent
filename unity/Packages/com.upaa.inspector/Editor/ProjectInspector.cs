// MIT. Fixed read-only commands; no eval, writes, imports, scene opening, or instantiation.
using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Threading.Tasks;
using System.Diagnostics;
using Newtonsoft.Json.Linq;
using Unity.Pipeline.Commands;
using UnityEditor;
using UnityEditor.PackageManager;
using UnityEngine;
using UnityEngine.Rendering;
using UnityEngine.SceneManagement;
using Object = UnityEngine.Object;

namespace UPAA.Inspector
{
    [InitializeOnLoad]
    public static class ProjectInspector
    {
        const int MaxRows = 4096;
        static long Revision;
        static ProjectInspector() { ObjectChangeEvents.changesPublished += Changed; }
        static void Changed(ref ObjectChangeEventStream changes) { Revision++; }
        static readonly HashSet<string> Cancelled = new HashSet<string>();
        static readonly HashSet<string> Active = new HashSet<string>();
        [CliCommand("upaa_cancel", "Cancel one UPAA read request", MainThreadRequired = true)]
        public static JObject Cancel([CliArg("request", "JSON requestId")] string request = "{}")
        {
            var id = (string)JObject.Parse(request)["requestId"];
            if (id != null && Active.Contains(id)) Cancelled.Add(id);
            return Envelope();
        }
        static async Task Batch(string id, Stopwatch watch)
        {
            if (Cancelled.Contains(id)) throw new OperationCanceledException("Read cancelled");
            if (watch.Elapsed.TotalSeconds > 10) throw new TimeoutException("Read timed out; narrow the asset query");
            var next = new TaskCompletionSource<bool>();
            void Tick() { EditorApplication.update -= Tick; next.TrySetResult(true); }
            EditorApplication.update += Tick;
            await next.Task;
            if (Cancelled.Contains(id)) throw new OperationCanceledException("Read cancelled");
            if (EditorApplication.isCompiling || EditorApplication.isUpdating || EditorApplication.isPlayingOrWillChangePlaymode) throw new InvalidOperationException("Editor state changed during read");
        }
        static string Root => Path.GetFullPath(Path.Combine(Application.dataPath, "..")).TrimEnd(Path.DirectorySeparatorChar);
        static JObject Envelope() => new JObject {
            ["protocolVersion"] = 1, ["projectRoot"] = Root, ["unityVersion"] = Application.unityVersion,
            ["sampledAt"] = DateTime.UtcNow.ToString("O"), ["targetPlatform"] = EditorUserBuildSettings.activeBuildTarget.ToString(),
            ["status"] = EditorApplication.isCompiling || EditorApplication.isUpdating || EditorApplication.isPlayingOrWillChangePlaymode ? "busy" : "ready",
            ["reason"] = "编译/导入/Play Mode 时不采集；当前 Editor 状态不代表录制当帧"
        };

        [CliCommand("upaa_context", "Read UPAA protocol, project identity, versions and loaded scene state", MainThreadRequired = true)]
        public static JObject Context([CliArg("request", "Reserved JSON object")] string request = "{}")
        {
            var result = Envelope();
            var scenes = new JArray();
            for (int i = 0; i < SceneManager.sceneCount && i < 20; i++) { var s = SceneManager.GetSceneAt(i); scenes.Add(new JObject { ["path"] = s.path, ["loaded"] = s.isLoaded, ["dirty"] = s.isDirty }); }
            result["loadedScenes"] = scenes;
            result["qualityLevel"] = QualitySettings.GetQualityLevel();
            result["renderPipeline"] = GraphicsSettings.currentRenderPipeline ? AssetDatabase.GetAssetPath(GraphicsSettings.currentRenderPipeline) : "Built-in";
            return result;
        }

        static string Resolve(string path)
        {
            if (string.IsNullOrEmpty(path) || !(path.StartsWith("Assets/", StringComparison.Ordinal) || path.StartsWith("Packages/", StringComparison.Ordinal)) || path.Contains('\\') || path.Contains(':') || path.Split('/').Any(p => p == ".." || p == "." || p == "")) throw new ArgumentException("Asset path outside project scope");
            string physical, boundary;
            if (path.StartsWith("Packages/", StringComparison.Ordinal)) {
                var package = UnityEditor.PackageManager.PackageInfo.FindForAssetPath(path);
                if (package == null || string.IsNullOrEmpty(package.resolvedPath)) throw new ArgumentException("Package is not resolved in this project");
                var prefix = "Packages/" + package.name + "/";
                if (!path.StartsWith(prefix, StringComparison.Ordinal)) throw new ArgumentException("Package identity mismatch");
                boundary = Path.GetFullPath(package.resolvedPath); physical = Path.GetFullPath(Path.Combine(boundary, path.Substring(prefix.Length)));
            } else { boundary = Root; physical = Path.GetFullPath(Path.Combine(Root, path)); }
            if (!physical.StartsWith(boundary.TrimEnd(Path.DirectorySeparatorChar) + Path.DirectorySeparatorChar, StringComparison.OrdinalIgnoreCase)) throw new ArgumentException("Asset escaped scope");
            for (var p = physical; !string.IsNullOrEmpty(p); p = Path.GetDirectoryName(p)) {
                if ((File.Exists(p) || Directory.Exists(p)) && (File.GetAttributes(p) & FileAttributes.ReparsePoint) != 0) throw new ArgumentException("Links/junctions are excluded");
                if (string.Equals(p, boundary, StringComparison.OrdinalIgnoreCase)) break;
            }
            return physical;
        }

        static JObject Row(string kind, string key, object value, Object owner = null)
        {
            var row = new JObject { ["kind"] = kind, ["property"] = key, ["value"] = value == null ? JValue.CreateNull() : JToken.FromObject(value) };
            if (owner) {
                row["objectName"] = owner.name;
                if (AssetDatabase.TryGetGUIDAndLocalFileIdentifier(owner, out string guid, out long id)) { row["guid"] = guid; row["fileId"] = id.ToString(); }
                else row["globalObjectId"] = GlobalObjectId.GetGlobalObjectIdSlow(owner).ToString();
            }
            return row;
        }
        static void Properties(Object obj, List<JObject> rows)
        {
            if (!obj || rows.Count >= MaxRows) return;
            using (var so = new SerializedObject(obj)) {
                var p = so.GetIterator(); int count = 0;
                while (p.NextVisible(true) && count++ < 256 && rows.Count < MaxRows) {
                    if (p.propertyType == SerializedPropertyType.Generic) continue;
                    object value = null;
                    switch (p.propertyType) {
                        case SerializedPropertyType.Integer: value = p.longValue; break;
                        case SerializedPropertyType.Boolean: value = p.boolValue; break;
                        case SerializedPropertyType.Float: value = double.IsNaN(p.doubleValue) || double.IsInfinity(p.doubleValue) ? "non-finite" : (object)p.doubleValue; break;
                        case SerializedPropertyType.String: value = p.stringValue.Length <= 256 ? p.stringValue : p.stringValue.Substring(0, 256) + "… [truncated]"; break;
                        case SerializedPropertyType.Enum: value = p.enumValueIndex; break;
                        case SerializedPropertyType.Color:
                            var color = p.colorValue; value = new { color.r, color.g, color.b, color.a }; break;
                        case SerializedPropertyType.Vector2:
                            var v2 = p.vector2Value; value = new { v2.x, v2.y }; break;
                        case SerializedPropertyType.Vector3:
                            var v3 = p.vector3Value; value = new { v3.x, v3.y, v3.z }; break;
                        case SerializedPropertyType.Vector4:
                            var v4 = p.vector4Value; value = new { v4.x, v4.y, v4.z, v4.w }; break;
                        case SerializedPropertyType.ObjectReference:
                            if (p.objectReferenceValue) { var reference = p.objectReferenceValue; AssetDatabase.TryGetGUIDAndLocalFileIdentifier(reference, out string g, out long id); value = new { path = AssetDatabase.GetAssetPath(reference), guid = g, fileId = id.ToString(), type = reference.GetType().FullName }; }
                            break;
                        default: continue;
                    }
                    rows.Add(Row("serializedProperty", p.propertyPath, value, obj));
                }
                if (count >= 256) rows.Add(Row("coverage", "propertiesTruncated", true, obj));
            }
        }
        static void InspectObject(Object obj, List<JObject> rows)
        {
            if (!obj || rows.Count >= MaxRows) return;
            rows.Add(Row("object", "type", obj.GetType().FullName, obj));
            if (obj is Mesh mesh) {
                rows.Add(Row("mesh", "vertexCount", mesh.vertexCount, obj)); rows.Add(Row("mesh", "subMeshCount", mesh.subMeshCount, obj));
                for (int i = 0; i < mesh.subMeshCount && i < 100 && rows.Count < MaxRows; i++) rows.Add(Row("mesh", "subMesh[" + i + "]", new { indexCount = mesh.GetIndexCount(i), topology = mesh.GetTopology(i).ToString() }, obj));
            } else if (obj is Texture texture) {
                rows.Add(Row("texture", "dimensions", new { width = texture.width, height = texture.height, dimension = texture.dimension.ToString() }, obj));
                if (texture is Texture2D t) rows.Add(Row("texture", "imported", new { format = t.format.ToString(), mipmapCount = t.mipmapCount, isReadable = t.isReadable }, obj));
            } else if (obj is Material mat) {
                rows.Add(Row("material", "shader", mat.shader ? mat.shader.name : "missing", obj)); rows.Add(Row("material", "passCount", mat.passCount, obj));
                rows.Add(Row("material", "flags", new { mat.enableInstancing, mat.renderQueue, keywords = mat.shaderKeywords.Take(32).ToArray() }, obj));
                if (mat.shader) InspectObject(mat.shader, rows);
            } else if (obj is Shader shader) {
                for (int i = 0; i < shader.GetPropertyCount() && i < 64 && rows.Count < MaxRows; i++) rows.Add(Row("shader", shader.GetPropertyName(i), new { type = shader.GetPropertyType(i).ToString(), flags = shader.GetPropertyFlags(i).ToString() }, obj));
                if (shader.GetPropertyCount() > 64) rows.Add(Row("coverage", "shaderPropertiesTruncated", true, obj));
            } else if (obj is MeshFilter filter && filter.sharedMesh) {
                InspectObject(filter.sharedMesh, rows);
            } else if (obj is LODGroup lod) {
                rows.Add(Row("lod", "levels", lod.GetLODs().Take(16).Select(l => new { transition = l.screenRelativeTransitionHeight, renderers = l.renderers.Length }).ToArray(), obj));
            } else if (obj is Renderer renderer) {
                rows.Add(Row("renderer", "settings", new { renderer.enabled, shadowCasting = renderer.shadowCastingMode.ToString(), renderer.receiveShadows, materials = renderer.sharedMaterials.Take(32).Select(AssetDatabase.GetAssetPath).ToArray() }, obj));
                if (renderer is SkinnedMeshRenderer skin && skin.sharedMesh) InspectObject(skin.sharedMesh, rows);
            }
            Properties(obj, rows);
        }

        [CliCommand("upaa_asset", "Read one asset or an already-loaded scene; never opens scenes or instantiates prefabs", MainThreadRequired = true)]
        public static async Task<JObject> Asset([CliArg("request", "JSON {path,start,limit}", Required = true)] string request)
        {
            var result = Envelope(); if ((string)result["status"] != "ready") return result;
            string requestId = Guid.NewGuid().ToString();
            try {
                var input = JObject.Parse(request); var path = (string)input["path"]; Resolve(path);
                requestId = (string)input["requestId"] ?? requestId; Active.Add(requestId);
                var watch = Stopwatch.StartNew(); await Batch(requestId, watch);
                var revision = Revision; var initialPlatform = EditorUserBuildSettings.activeBuildTarget;
                int start = (int?)input["start"] ?? 0, limit = (int?)input["limit"] ?? 50;
                if (start < 0 || limit < 1 || limit > 100) throw new ArgumentException("Page outside bounds");
                result["path"] = path; var before = AssetDatabase.GetAssetDependencyHash(path).ToString();
                var rows = new List<JObject>();
                if (path.EndsWith(".unity", StringComparison.OrdinalIgnoreCase)) {
                    var scene = SceneManager.GetSceneByPath(path);
                    if (!scene.IsValid() || !scene.isLoaded) throw new ArgumentException("Scene is not loaded; inspect offline serialized evidence (will not open it)");
                    result["sceneDirty"] = scene.isDirty;
                    foreach (var go in scene.GetRootGameObjects()) { await Walk(go, rows, requestId, watch); if (rows.Count >= MaxRows) break; }
                } else {
                    // Asset load only. No Instantiate/LoadPrefabContents/OpenScene calls.
                    var asset = AssetDatabase.LoadMainAssetAtPath(path);
                    if (!asset) throw new ArgumentException("Asset is unavailable");
                    if (asset is GameObject go) await Walk(go, rows, requestId, watch); else InspectObject(asset, rows);
                    var importer = AssetImporter.GetAtPath(path);
                    if (importer is TextureImporter ti) {
                        foreach (var platform in new[] { "DefaultTexturePlatform", "Standalone", "Android", "iPhone", "WebGL" }) {
                            var setting = platform == "DefaultTexturePlatform" ? ti.GetDefaultPlatformTextureSettings() : ti.GetPlatformTextureSettings(platform);
                            rows.Insert(0, Row("textureImporter", platform, new { setting.overridden, setting.maxTextureSize, format = setting.format.ToString(), setting.textureCompression, setting.compressionQuality, ti.mipmapEnabled, ti.isReadable }));
                        }
                    } else if (importer is ModelImporter mi) rows.Insert(0, Row("modelImporter", "settings", new { mi.isReadable, mi.globalScale, mi.importAnimation, mi.importBlendShapes, meshCompression = mi.meshCompression.ToString() }));
                }
                await Batch(requestId, watch);
                var dependencies = AssetDatabase.GetDependencies(path, false);
                foreach (var dependency in dependencies.Take(1000)) rows.Add(Row("dependency", "path", dependency));
                var after = AssetDatabase.GetAssetDependencyHash(path).ToString(); if (before != after || revision != Revision || initialPlatform != EditorUserBuildSettings.activeBuildTarget) throw new ArgumentException("Resource/Editor state changed during inspection; retry after preparing project");
                // Hash all observed rows, including unsaved loaded-scene state, not only disk imports.
                result["fingerprint"] = Hash128.Compute(Application.unityVersion + EditorUserBuildSettings.activeBuildTarget + before + Newtonsoft.Json.JsonConvert.SerializeObject(rows)).ToString();
                result["assetDependencyHash"] = before; result["guid"] = AssetDatabase.AssetPathToGUID(path);
                result["origin"] = path.StartsWith("Packages/", StringComparison.Ordinal) ? "resolved-package-summary" : "project-asset";
                result["rows"] = JArray.FromObject(rows.Skip(start).Take(limit)); result["nextStart"] = start + limit < rows.Count ? (JToken)(start + limit) : JValue.CreateNull();
                result["coverage"] = "partial"; result["warnings"] = new JArray("Editor 当前状态不代表录制当帧；属性只读取支持的类型，每对象至多256项，层级至多4096条记录，依赖至多1000项；不是运行时内存/GPU测量", rows.Count >= MaxRows ? "对象/属性列表达到上限" : "", dependencies.Length > 1000 ? "依赖达到上限" : "");
            } catch (Exception e) { result["status"] = "unavailable"; result["reason"] = e.Message; }
            finally { Active.Remove(requestId); Cancelled.Remove(requestId); }
            return result;
        }
        static async Task Walk(GameObject root, List<JObject> rows, string requestId, Stopwatch watch)
        {
            var queue = new Queue<Transform>(); queue.Enqueue(root.transform);
            while (queue.Count > 0 && rows.Count < MaxRows) {
                await Batch(requestId, watch);
                var t = queue.Dequeue(); InspectObject(t.gameObject, rows);
                foreach (var c in t.GetComponents<Component>()) { if (rows.Count >= MaxRows) break; InspectObject(c, rows); }
                foreach (Transform child in t) { if (queue.Count < MaxRows) queue.Enqueue(child); }
            }
        }
    }
}
