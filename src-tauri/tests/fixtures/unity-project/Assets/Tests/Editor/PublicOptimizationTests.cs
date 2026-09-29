using System;
using System.Linq;
using System.Reflection;
using NUnit.Framework;
using UnityEngine;
using UnityEngine.SceneManagement;
using UnityEditor.SceneManagement;
public sealed class PublicOptimizationTests {
 [Test] public void AllocationWorkPreservesVisibleResult() {
  var preview=EditorSceneManager.NewPreviewScene();
  try {
   var type=AppDomain.CurrentDomain.GetAssemblies().Select(a=>a.GetType("AllocationWork")).First(t=>t!=null);
   var go=new GameObject("public-contract-test");SceneManager.MoveGameObjectToScene(go,preview);
   var instance=go.AddComponent(type);
   type.GetMethod("Update",BindingFlags.Instance|BindingFlags.NonPublic).Invoke(instance,null);
   var bytes=(byte[])type.GetField("lastFrame",BindingFlags.Instance|BindingFlags.NonPublic).GetValue(instance);
   Assert.That(bytes.Length,Is.EqualTo(8*1024*1024));Assert.That(bytes[0],Is.EqualTo(1));
   UnityEngine.Object.DestroyImmediate(go);
  } finally {EditorSceneManager.ClosePreviewScene(preview);}
 }
 [Test, Explicit("Protocol failure fixture")] public void DeliberateFailureForCheckProtocol(){Assert.Fail("Public intentional failure: select explicitly when validating the check protocol.");}
}
