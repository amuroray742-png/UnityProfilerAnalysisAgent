// Public synthetic example; not taken from a private Unity project.
using UnityEngine;
public sealed class AllocationWork : MonoBehaviour
{
    private byte[] lastFrame;
    // Profiler marker candidate: AllocationWork.Update.
    private void Update()
    {
        lastFrame = new byte[8 * 1024 * 1024];
        lastFrame[0] = 1;
    }
}
