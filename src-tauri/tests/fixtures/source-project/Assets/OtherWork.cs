// Same method name with unrelated behavior: name matches are not proof.
using UnityEngine;
public sealed class OtherWork : MonoBehaviour
{
    private void Update() { transform.Rotate(0f, Time.deltaTime, 0f); }
}
