// Public investigation-only examples. These methods are not invoked by acceptance tests.
using UnityEngine;

public static class BatchSpawnEntry
{
    public static void SpawnWave(GameObject prefab, int count)
    {
        for (var i = 0; i < count; i++) Object.Instantiate(prefab);
    }

    public static GameObject LoadBeforeSpawn()
    {
        return Resources.Load<GameObject>("PublicTestOnly/Unit");
    }
}

public sealed class PublicVoiceQueue
{
    private readonly string[] queue = { "public" };
    public bool IsIdle => ((string[])queue.Clone()).Length == 0;
}

public sealed class PublicTipPresenter
{
    private readonly PublicVoiceQueue voice = new PublicVoiceQueue();
    public bool LateUpdate() { return voice.IsIdle; }
}
