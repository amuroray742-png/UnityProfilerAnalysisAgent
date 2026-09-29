// Test-only generator. Never install this script into a user project.
using System;
using System.IO;
using UnityEditor;
using UnityEditor.SceneManagement;
using UnityEngine;
public static class UPAAFixture
{
    public static void Build()
    {
        var root = Path.GetFullPath(Path.Combine(Application.dataPath, ".."));
        if (!File.Exists(Path.Combine(root, ".upaa-public-fixture"))) throw new Exception("Not an authorized public fixture project");
        if (File.Exists("Assets/PublicScene.unity")) { Debug.Log("UPAA_FIXTURE_ALREADY_BUILT"); return; }
        var scene = EditorSceneManager.NewScene(NewSceneSetup.EmptyScene, NewSceneMode.Single);
        var texture = new Texture2D(64, 32); var colors = new Color[64 * 32];
        for (int i = 0; i < colors.Length; i++) colors[i] = i % 2 == 0 ? Color.red : Color.blue;
        texture.SetPixels(colors); texture.Apply(); File.WriteAllBytes("Assets/PublicTexture.png", texture.EncodeToPNG()); UnityEngine.Object.DestroyImmediate(texture);
        AssetDatabase.ImportAsset("Assets/PublicTexture.png");
        var importer = (TextureImporter)AssetImporter.GetAtPath("Assets/PublicTexture.png"); importer.isReadable = true; importer.mipmapEnabled = false;
        importer.SetPlatformTextureSettings(new TextureImporterPlatformSettings { name = "Android", overridden = true, maxTextureSize = 32, format = TextureImporterFormat.RGBA32 }); importer.SaveAndReimport();
        var material = new Material(Shader.Find("Unlit/Texture")); material.mainTexture = AssetDatabase.LoadAssetAtPath<Texture2D>("Assets/PublicTexture.png"); material.enableInstancing = false; AssetDatabase.CreateAsset(material, "Assets/PublicMaterial.mat");
        var mesh = new Mesh { name = "PublicTriangle" }; mesh.vertices = new[] { Vector3.zero, Vector3.right, Vector3.up }; mesh.triangles = new[] { 0, 1, 2 }; mesh.RecalculateBounds(); AssetDatabase.CreateAsset(mesh, "Assets/PublicMesh.asset");
        var go = new GameObject("AllocationRenderer"); go.AddComponent<MeshFilter>().sharedMesh = mesh; go.AddComponent<MeshRenderer>().sharedMaterial = material; go.AddComponent<AllocationWork>().effectsMaterial = material;
        go.AddComponent<LODGroup>().SetLODs(new[] { new LOD(0.5f, new[] { go.GetComponent<Renderer>() }) });
        PrefabUtility.SaveAsPrefabAsset(go, "Assets/Public.prefab"); UnityEngine.Object.DestroyImmediate(go);
        var instance = (GameObject)PrefabUtility.InstantiatePrefab(AssetDatabase.LoadAssetAtPath<GameObject>("Assets/Public.prefab"));
        instance.GetComponent<Renderer>().shadowCastingMode = UnityEngine.Rendering.ShadowCastingMode.Off; PrefabUtility.RecordPrefabInstancePropertyModifications(instance.GetComponent<Renderer>());
        PrefabUtility.SaveAsPrefabAsset(instance, "Assets/PublicVariant.prefab");
        new GameObject("OtherUpdateCandidate").AddComponent<OtherWork>();
        EditorSceneManager.SaveScene(scene, "Assets/PublicScene.unity"); AssetDatabase.SaveAssets();
        File.WriteAllText(Path.Combine(root, "fixture-baseline.json"), "{\"meshVertices\":3,\"meshIndices\":3,\"textureWidth\":64,\"textureHeight\":32,\"androidMaxSize\":32,\"sceneDirty\":" + scene.isDirty.ToString().ToLowerInvariant() + "}");
        Debug.Log("UPAA_FIXTURE_BUILT");
    }
}
