// Public one-click optimization fixture. No private project code.
public static class AutomaticHotspots
{
    public static byte[] Update()
    {
        var bytes = new byte[4096];
        bytes[0] = 1;
        return bytes;
    }
}

public static class AutomaticCaller
{
    public static byte[] Invoke() { return AutomaticHotspots.Update(); }
}
