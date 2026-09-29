// Public synthetic workload; not a private recording reproduction.
using UnityEngine;
public sealed class AllocationWork : MonoBehaviour
{
    public Material effectsMaterial;
    private byte[] lastFrame;
    private void Update()
    {
        lastFrame = new byte[8 * 1024 * 1024];
        lastFrame[0] = 1;
    }
}
