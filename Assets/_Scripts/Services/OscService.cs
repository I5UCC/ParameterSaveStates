using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;
using System.Net;
using System.Net.Sockets;
using BlobHandles;
using OscCore;
using UnityEngine;
using VRC.OSCQuery;

public class OscService : IDisposable
{
    public sealed class AvatarParameterValue
    {
        public string OscType;
        public string Value;
    }

    private OSCQueryService _oscQuery;
    private OscServer _receiver;
    private OscClient _sender;
    private readonly int _tcpPort;
    private readonly int _udpPort;
    private readonly object _parameterCacheLock = new object();
    private readonly Dictionary<string, AvatarParameterValue> _currentAvatarParameterValues =
        new Dictionary<string, AvatarParameterValue>(StringComparer.OrdinalIgnoreCase);
    private Dictionary<string, AvatarParameterValue> _lastAvatarParameterSnapshot =
        new Dictionary<string, AvatarParameterValue>(StringComparer.OrdinalIgnoreCase);

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

        var tree = Extensions.GetOSCTree(QueryServiceProfile.address, QueryServiceProfile.port).GetAwaiter().GetResult();
        var node = tree.GetNodeWithPath("/avatar/change");
        if (node?.Value == null || node.Value.Length == 0)
        {
            Debug.LogWarning("OscService: /avatar/change node has no value; cannot determine current avatar.");
            _oscQuery.OnOscQueryServiceAdded -= HandleOscQueryServiceAdded;
            OnVRChatConnected?.Invoke(QueryServiceProfile);
            return;
        }
        var currentAvatar = node.Value[0].ToString();
        Debug.Log("Current Avatar: " + currentAvatar);

        _oscQuery.OnOscQueryServiceAdded -= HandleOscQueryServiceAdded;
        OnVRChatConnected?.Invoke(QueryServiceProfile);
        OnAvatarChanged?.Invoke(null, currentAvatar);
    }

    private void OnMessageReceived(BlobString address, OscMessageValues values)
    {
        var addressString = address.ToString();
        if (addressString == "/avatar/change")
        {
            var newAvatar = values.ReadStringElement(0);
            lock (_parameterCacheLock)
            {
                _lastAvatarParameterSnapshot = CloneParameterMap(_currentAvatarParameterValues);
                _currentAvatarParameterValues.Clear();
            }

            // previousAvatar is intentionally null here; callers track previous state themselves.
            OnAvatarChanged?.Invoke(null, newAvatar);
            return;
        }

        if (!addressString.StartsWith("/avatar/parameters/", StringComparison.OrdinalIgnoreCase)
            || values.ElementCount <= 0)
        {
            return;
        }

        values.ForEachElement((index, tag) =>
        {
            if (index != 0)
            {
                return;
            }

            AvatarParameterValue cachedValue;
            switch (tag)
            {
                case TypeTag.Float32:
                    cachedValue = new AvatarParameterValue
                    {
                        OscType = "f",
                        Value = values.ReadFloatElement(index).ToString(CultureInfo.InvariantCulture)
                    };
                    break;
                case TypeTag.Int32:
                    cachedValue = new AvatarParameterValue
                    {
                        OscType = "i",
                        Value = values.ReadIntElement(index).ToString(CultureInfo.InvariantCulture)
                    };
                    break;
                case TypeTag.True:
                case TypeTag.False:
                    cachedValue = new AvatarParameterValue
                    {
                        OscType = "T",
                        Value = values.ReadBooleanElement(index).ToString()
                    };
                    break;
                default:
                    return;
            }

            lock (_parameterCacheLock)
            {
                _currentAvatarParameterValues[addressString] = cachedValue;
            }
        });
    }

    public void ReconnectToVRChat()
    {
        QueryServiceProfile = null;
        _oscQuery.OnOscQueryServiceAdded += HandleOscQueryServiceAdded;
        _oscQuery.RefreshServices();
    }

    public Dictionary<string, AvatarParameterValue> GetLastAvatarParameterSnapshot()
    {
        lock (_parameterCacheLock)
        {
            return CloneParameterMap(_lastAvatarParameterSnapshot);
        }
    }

    public void SendFloat(string address, float value) => _sender.Send(address, value);
    public void SendInt(string address, int value) => _sender.Send(address, value);
    public void SendBool(string address, bool value) => _sender.Send(address, value);

    private static Dictionary<string, AvatarParameterValue> CloneParameterMap(
        Dictionary<string, AvatarParameterValue> source)
    {
        var copy = new Dictionary<string, AvatarParameterValue>(StringComparer.OrdinalIgnoreCase);
        foreach (var pair in source)
        {
            copy[pair.Key] = new AvatarParameterValue
            {
                OscType = pair.Value.OscType,
                Value = pair.Value.Value
            };
        }

        return copy;
    }

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
                   .FirstOrDefault(ip => ip.AddressFamily == AddressFamily.InterNetwork)
               ?? IPAddress.Loopback;
    }

    public void Dispose()
    {
        _receiver?.Dispose();
        _oscQuery?.Dispose();
    }
}
