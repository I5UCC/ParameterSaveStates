using System.IO;
using UnityEngine;
using UnityEngine.UI;
using Valve.VR;
using IniParser;
using IniParser.Model;
using System.Threading;
using VRC.OSCQuery;
using OscCore;
using BlobHandles;
using System.Net;
using System.Net.Sockets;
using System;
using PimDeWitte.UnityMainThreadDispatcher;
using System.Collections.Generic;
using JetBrains.Annotations;
using System.Runtime.InteropServices;
using System.Threading.Tasks;
using System.Linq;
using Newtonsoft.Json;

public class ParameterSaveStates_Director : MonoBehaviour
{
    private readonly string MANIFESTLFILEPATH = Path.GetFullPath("app.vrmanifest");

    public Unity_Overlay menuOverlay;

    public GameObject profile_Container;

    public GameObject keyboard;
    
    public Text CurrentAvatarText;
    public InputField inputField;

    private bool initialized = false;

    private string previousAvatar;
    private string currentAvatar;

    private OSCQueryService _oscQuery;
    private int tcpPort = Extensions.GetAvailableTcpPort();
    private int udpPort = Extensions.GetAvailableUdpPort();
    private OscServer _receiver;
    private OscClient _sender;
    private UnityMainThreadDispatcher _mainTheadDispatcher;

    private OSCQueryServiceProfile QueryServiceProfile;

    void Start()
    {
        _mainTheadDispatcher = UnityMainThreadDispatcher.Instance();
        ShiftKeys();
        Start_OSC();
    }

    private void Start_OSC()
    {
        _sender = new OscClient("127.0.0.1", 9000);
        VRC.OSCQuery.IDiscovery discovery = new MeaModDiscovery();
        _receiver = OscServer.GetOrCreate(udpPort);

        // Listen to all incoming messages
        _receiver.AddMonitorCallback(OnMessageReceived);

        _oscQuery = new OSCQueryServiceBuilder()
            .WithServiceName("VRCParameterSaveStates")
            .WithHostIP(GetLocalIPAddress())
            .WithOscIP(GetLocalIPAddressNonLoopback())
            .WithTcpPort(tcpPort)
            .WithUdpPort(udpPort)
            .WithDiscovery(discovery)
            .StartHttpServer()
            .AdvertiseOSC()
            .AdvertiseOSCQuery()
            .Build();
        _oscQuery.RefreshServices();
        _oscQuery.OnOscQueryServiceAdded += OnOscQueryServiceAdded;
        _oscQuery.AddEndpoint<string>("/avatar/change", Attributes.AccessValues.WriteOnly);
    }

    private async void OnOscQueryServiceAdded(OSCQueryServiceProfile profile)
    {
        Debug.Log($"\nfound service {profile.name} at {profile.port} on {profile.address}");
        if (profile.name.Contains("VRChat"))
        {
            QueryServiceProfile = profile;
            Debug.Log("QueryRoot: " + QueryServiceProfile.address + ":" + QueryServiceProfile.port);
            var tree = await Extensions.GetOSCTree(QueryServiceProfile.address, QueryServiceProfile.port);
            var node = tree.GetNodeWithPath("/avatar/change");
            currentAvatar = node.Value[0].ToString();
            Debug.Log("Current Avatar: " + currentAvatar);
            _mainTheadDispatcher.Enqueue(() => {
                CurrentAvatarText.text = "Current Avatar: " + currentAvatar;
                SetActiveProfiles();
            });
            _oscQuery.OnOscQueryServiceAdded -= OnOscQueryServiceAdded;
        }
    }

    private void OnMessageReceived(BlobString address, OscMessageValues values)
    {
        string address_string = address.ToString();

        if (address_string == "/avatar/change")
        {
            var temp = values.ReadStringElement(0);
            if (temp == currentAvatar)
            {
                return;
            }
            previousAvatar = currentAvatar;
            currentAvatar = values.ReadStringElement(0);
            Debug.Log("Avatar changed from: " + previousAvatar);
            Debug.Log("Avatar changed to: " + currentAvatar);
            _mainTheadDispatcher.Enqueue(() => {
                CurrentAvatarText.text = "Current Avatar: " + currentAvatar;
                SetActiveProfiles();
            });
            return;
        }
    }

    public static IPAddress GetLocalIPAddress()
    {
        // Android can always serve on the non-loopback address
#if UNITY_ANDROID
        return GetLocalIPAddressNonLoopback();
#else
        // Windows can only serve TCP on the loopback address, but can serve UDP on the non-loopback address
        return IPAddress.Loopback;
#endif
    }

    public static IPAddress GetLocalIPAddressNonLoopback()
    {
        // Get the host name of the local machine
        string hostName = Dns.GetHostName();

        // Get the IP address of the first IPv4 network interface found on the local machine
        foreach (IPAddress ip in Dns.GetHostEntry(hostName).AddressList)
        {
            if (ip.AddressFamily == AddressFamily.InterNetwork)
            {
                return ip;
            }
        }
        return null;
    }

    public void OnApplicationQuit()
    {
        _receiver.Dispose();
        _oscQuery.Dispose();
    }

    public void OnSteamVRConnect()
    {
        initialized = true;
        if (File.Exists(MANIFESTLFILEPATH))
        {
            OpenVR.Applications.AddApplicationManifest(MANIFESTLFILEPATH, false);
        }
    }

    public void OnSteamVRDisconnect()
    {
        Debug.Log("Quitting!");
        Application.Quit();
    }

    public void OnDashBoardOpen()
    {
        return;
    }

    public void OnDashBoardClose()
    {
        return;
    }

    private void SetActiveProfiles()
    {
        keyboard.SetActive(false);
        profile_Container.SetActive(true);
        foreach (Transform child in profile_Container.transform)
        {
            child.gameObject.SetActive(false);
        }

        string folderPath = Path.Combine(Application.persistentDataPath, $"Profiles/{currentAvatar}");
        if (!Directory.Exists(folderPath))
        {
            return;
        }

        // get all json files in the folder
        string[] files = Directory.GetFiles(folderPath, "*");
        for (int i = 0; i < files.Length; i++)
        {
            string fileName = Path.GetFileName(files[i]);
            GameObject profile = profile_Container.transform.GetChild(i).gameObject;
            profile.SetActive(true);
            profile.transform.GetChild(0).GetComponent<Text>().text = fileName;
        }
    }

    public void SetProfile(GameObject profile)
    {
        string profilepath = Path.Combine(Application.persistentDataPath, $"Profiles/{currentAvatar}/{profile.transform.GetChild(0).GetComponent<Text>().text}");
        if (!File.Exists(profilepath))
        {
            Debug.LogError("Profile not found");
            return;
        }

        // Read the json file
        string json = File.ReadAllText(profilepath);
        var dict = JsonConvert.DeserializeObject<Dictionary<string, (string, string)>>(json);

        foreach (var item in dict)
        {
            switch (item.Value.Item2)
            {
                case "f":
                    _sender.Send(item.Key, float.Parse(item.Value.Item1));
                    break;
                case "i":
                    _sender.Send(item.Key, int.Parse(item.Value.Item1));
                    break;
                case "T":
                    _sender.Send(item.Key, bool.Parse(item.Value.Item1));
                    break;
                default:
                    Debug.LogError("Unknown OSC Type");
                    break;
            }
        }
    }

    public void DeleteProfile(GameObject profile)
    {
        Text DeleteText = profile.transform.GetChild(2).GetChild(0).GetComponent<Text>();
        if (DeleteText.text == "Delete")
        {
            DeleteText.text = "Sure?";
            return;
        }
        string profilepath = Path.Combine(Application.persistentDataPath, $"Profiles/{currentAvatar}/{profile.transform.GetChild(0).GetComponent<Text>().text}");
        if (!File.Exists(profilepath))
        {
            Debug.LogError("Profile not found");
            return;
        }
        File.Delete(profilepath);
        profile.SetActive(false);
        DeleteText.text = "Delete";
        SetActiveProfiles();
    }

    public async void SaveProfile(InputField input)
    {
        if (string.IsNullOrWhiteSpace(currentAvatar))
        {
            Debug.LogError("No Avatar set");
            return;
        }
        if (string.IsNullOrWhiteSpace(input.text))
        {
            Debug.LogWarning("No Profile Name set");
            return;
        }
        if (QueryServiceProfile == null) {
            Debug.LogWarning("No QueryRoot found");
            return;
        }

        // Create a folder with currentAvatar as the name
        string folderPath = Path.Combine(Application.persistentDataPath, $"Profiles/{currentAvatar}");
        if (!Directory.Exists(folderPath))
        {
            Debug.Log("Creating Directory: " + folderPath);
            Directory.CreateDirectory(folderPath);
        }

        var tree = await Extensions.GetOSCTree(QueryServiceProfile.address, QueryServiceProfile.port);
        var node = tree.GetNodeWithPath("/avatar/parameters");

        var dict = await getJson(node);
        
        // Save the dict to a file
        var json = JsonConvert.SerializeObject(dict, Formatting.Indented);
        string filePath = Path.Combine(folderPath, $"{input.text}");
        File.WriteAllText(filePath, json);
        
        Debug.Log($"Saved Profile: {input.text}");

        _mainTheadDispatcher.Enqueue(() => {
            SetActiveProfiles();
        });

        input.text = "";
    }

    public async Task<Dictionary<string, (string value, string OscType)>> getJson(OSCQueryNode node) 
    {
        Dictionary<string, (string value, string OscType)> dict = new Dictionary<string, (string value, string OscType)>();
        foreach (var content in node.Contents)
        {
            var value = content.Value.Value;
            if (value is null)
            {
                var tree = await Extensions.GetOSCTree(QueryServiceProfile.address, QueryServiceProfile.port);
                var innernode = tree.GetNodeWithPath(content.Value.FullPath);
                var innerdict = await getJson(innernode);
                foreach (var innercontent in innerdict)
                {
                    dict.Add(innercontent.Key, innercontent.Value);
                }
            }
            else
            {
                dict.Add(content.Value.FullPath, (value[0].ToString(), content.Value.OscType));
            }
        }
        return dict;
    }

    public void CopyFromPreviousAvatar(Text buttontext)
    {
        if (buttontext.text == "Copy From Last") {
            buttontext.text = "Sure?";
            return;
        }
        if (string.IsNullOrWhiteSpace(previousAvatar))
        {
            Debug.LogError("No Previous Avatar set");
            return;
        }

        var previousfolderPath = Path.Combine(Application.persistentDataPath, $"Profiles/{previousAvatar}");
        var currentfolderPath = Path.Combine(Application.persistentDataPath, $"Profiles/{currentAvatar}");

        if (!Directory.Exists(previousfolderPath))
        {
            Debug.LogError("Previous Avatar Folder not found");
            return;
        }

        if (!Directory.Exists(currentfolderPath))
        {
            Debug.Log("Creating Directory: " + currentfolderPath);
            Directory.CreateDirectory(currentfolderPath);
        }

        string[] files = Directory.GetFiles(previousfolderPath, "*");
        for (int i = 0; i < files.Length; i++)
        {
            string fileName = Path.GetFileName(files[i]);
            string sourceFile = Path.Combine(previousfolderPath, fileName);
            string destFile = Path.Combine(currentfolderPath, fileName);
            File.Copy(sourceFile, destFile, true);
        }
        SetActiveProfiles();
        buttontext.text = "Copy From Last";
    }

    public void HandleInput(Button button)
    {
        var key = button.name;
        if (key == "BACK")
        {
            if (inputField.text.Length > 0)
            {
                inputField.text = inputField.text.Remove(inputField.text.Length - 1);
            }
            return;
        }
        else if (key == "SPACE")
        {
            inputField.text += " ";
            return;
        }
        else if (key == "SHIFT")
        {
            _mainTheadDispatcher.Enqueue(() => {
                ShiftKeys();
            });
            return;
        }
        key = button.GetComponentInChildren<Text>().text;
        inputField.text += key;
    }

    public void ShowKeyboard()
    {
        var active = keyboard.activeSelf;
        keyboard.SetActive(!active);
        profile_Container.SetActive(active);
        if (!active)
            inputField.Select();
        else
            inputField.DeactivateInputField();
    }

    public void ShiftKeys()
    {
        foreach (Transform child in keyboard.transform)
        {
            if (child.name == "Line1" || child.name == "Line2" || child.name == "Line3")
            {
                foreach (Transform button in child)
                {
                    if (button.GetComponentInChildren<Text>().text == button.GetComponentInChildren<Text>().text.ToUpper())
                    {
                        button.GetComponentInChildren<Text>().text = button.GetComponentInChildren<Text>().text.ToLower();
                    }
                    else
                    {
                        button.GetComponentInChildren<Text>().text = button.GetComponentInChildren<Text>().text.ToUpper();
                    }
                }
            }
        }
    }
}
