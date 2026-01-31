using System;
using System.Linq;
using System.Net;
using System.Net.Sockets;
using BlobHandles;
using OscCore;
using UnityEngine;
using VRC.OSCQuery;

public class OscService : IDisposable
{
    private OSCQueryService _oscQuery;
    private OscServer _receiver;
    private OscClient _sender;
    private readonly int _tcpPort;
    private readonly int _udpPort;

    public event Action<OSCQueryServiceProfile> OnVRChatConnected;
    public event Action<string, string> OnAvatarChanged;

    public OSCQueryServiceProfile QueryServiceProfile { get; private set; }

    public OscService()
    {
        _tcpPort = Extensions.GetAvailableTcpPort();
        _udpPort = Extensions.GetAvailableUdpPort();
    }

    public void Initialize()
    {
        _sender = new OscClient("127.0.0.1", 9000);
        IDiscovery discovery = new MeaModDiscovery();
        _receiver = OscServer.GetOrCreate(_udpPort);

        _receiver.AddMonitorCallback(OnMessageReceived);

        _oscQuery = new OSCQueryServiceBuilder()
            .WithServiceName("VRCParameterSaveStates")
            .WithHostIP(GetLocalIPAddress())
            .WithOscIP(GetLocalIPAddressNonLoopback())
            .WithTcpPort(_tcpPort)
            .WithUdpPort(_udpPort)
            .WithDiscovery(discovery)
            .StartHttpServer()
            .AdvertiseOSC()
            .AdvertiseOSCQuery()
            .Build();

        _oscQuery.RefreshServices();
        _oscQuery.OnOscQueryServiceAdded += HandleOscQueryServiceAdded;
        _oscQuery.AddEndpoint<string>("/avatar/change", Attributes.AccessValues.WriteOnly);
    }

    private void HandleOscQueryServiceAdded(OSCQueryServiceProfile profile)
    {
        Debug.Log($"\nfound service {profile.name} at {profile.port} on {profile.address}");
        if (!profile.name.Contains("VRChat")) return;

        QueryServiceProfile = profile;
        Debug.Log("QueryRoot: " + QueryServiceProfile.address + ":" + QueryServiceProfile.port);

        var test = Extensions.GetOSCTree(QueryServiceProfile.address, QueryServiceProfile.port);
        test.Wait();
        var tree = test.Result;
        var node = tree.GetNodeWithPath("/avatar/change");
        var currentAvatar = node.Value[0].ToString();
        Debug.Log("Current Avatar: " + currentAvatar);

        _oscQuery.OnOscQueryServiceAdded -= HandleOscQueryServiceAdded;
        OnVRChatConnected?.Invoke(QueryServiceProfile);
        OnAvatarChanged?.Invoke(null, currentAvatar);
    }

    private void OnMessageReceived(BlobString address, OscMessageValues values)
    {
        var addressString = address.ToString();
        if (addressString != "/avatar/change") return;

        var newAvatar = values.ReadStringElement(0);
        OnAvatarChanged?.Invoke(null, newAvatar);
    }

    public void ReconnectToVRChat()
    {
        QueryServiceProfile = null;
        _oscQuery.OnOscQueryServiceAdded += HandleOscQueryServiceAdded;
    }

    public void SendFloat(string address, float value) => _sender.Send(address, value);
    public void SendInt(string address, int value) => _sender.Send(address, value);
    public void SendBool(string address, bool value) => _sender.Send(address, value);

    private static IPAddress GetLocalIPAddress()
    {
#if UNITY_ANDROID
        return GetLocalIPAddressNonLoopback();
#else
        return IPAddress.Loopback;
#endif
    }

    private static IPAddress GetLocalIPAddressNonLoopback()
    {
        var hostName = Dns.GetHostName();
        return Dns.GetHostEntry(hostName).AddressList
            .FirstOrDefault(ip => ip.AddressFamily == AddressFamily.InterNetwork);
    }

    public void Dispose()
    {
        _receiver?.Dispose();
        _oscQuery?.Dispose();
    }
}
