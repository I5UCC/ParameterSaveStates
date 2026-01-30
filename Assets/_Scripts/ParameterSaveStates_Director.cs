using System;
using System.IO;
using UnityEngine;
using UnityEngine.UI;
using Valve.VR;
using VRC.OSCQuery;
using OscCore;
using BlobHandles;
using System.Net;
using System.Net.Sockets;
using PimDeWitte.UnityMainThreadDispatcher;
using System.Collections.Generic;
using System.Linq;
using Newtonsoft.Json;
using UnityEngine.Networking;
using System.Threading;

public class ParameterSaveStates_Director : MonoBehaviour
{
    private readonly string _manifestFilePath = Path.GetFullPath("app.vrmanifest");

    public Unity_Overlay menuOverlay;
    
    [Space(10)] 
    
    public GameObject profileContainer;
    
    [Space(10)] 
    
    public Text statusText;
    public Text currentAvatarText;

    [Space(10)] 
    
    public Button copyFromPreviousButton;
    public Button newButton;
    public Button cancelButton;
    
    [Space(10)] 
    
    public GameObject pagingContainer;
    public Button NextPageButton;
    public Button PrevPageButton;

    private string _previousAvatar;
    private string _currentAvatar;
    private OSCQueryService _oscQuery;
    private int _tcpPort;
    private int _udpPort;
    private OscServer _receiver;
    private OscClient _sender;
    private UnityMainThreadDispatcher _mainTheadDispatcher;
    private OSCQueryServiceProfile _queryServiceProfile;
    private bool _steamVRKeyboardOpen;
    private string _profileText = string.Empty;
    private const int PageSize = 20;
    private int _currentPage = 0;
    private List<string> _availableProfiles = new List<string>();

    private bool _initialized;

    private void Start()
    {
        _mainTheadDispatcher = UnityMainThreadDispatcher.Instance();
        _tcpPort = Extensions.GetAvailableTcpPort();
        _udpPort = Extensions.GetAvailableUdpPort();
        SetStatusText("Waiting for VRChat to connect...");

        currentAvatarText.gameObject.SetActive(false);
        cancelButton.gameObject.SetActive(false);
        newButton.gameObject.SetActive(false);
        copyFromPreviousButton.gameObject.SetActive(false);

        Start_OSC();

        if (menuOverlay != null && menuOverlay.overlay != null)
        {
            menuOverlay.overlay.onKeyboardDone += OnKeyboardDone;
        }
    }

    private void Start_OSC()
    {
        _sender = new OscClient("127.0.0.1", 9000);
        VRC.OSCQuery.IDiscovery discovery = new MeaModDiscovery();
        _receiver = OscServer.GetOrCreate(_udpPort);

        // Listen to all incoming messages
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
        _oscQuery.OnOscQueryServiceAdded += OnOscQueryServiceAdded;
        _oscQuery.AddEndpoint<string>("/avatar/change", Attributes.AccessValues.WriteOnly);
    }

    private void OnOscQueryServiceAdded(OSCQueryServiceProfile profile)
    {
        Debug.Log($"\nfound service {profile.name} at {profile.port} on {profile.address}");
        if (!profile.name.Contains("VRChat")) return;
        _queryServiceProfile = profile;
        Debug.Log("QueryRoot: " + _queryServiceProfile.address + ":" + _queryServiceProfile.port);
        var test = Extensions.GetOSCTree(_queryServiceProfile.address, _queryServiceProfile.port);
        test.Wait();
        var tree = test.Result;
        var node = tree.GetNodeWithPath("/avatar/change");
        _currentAvatar = node.Value[0].ToString();
        Debug.Log("Current Avatar: " + _currentAvatar);
        _mainTheadDispatcher.Enqueue(() =>
        {
            SetStatusText();
            currentAvatarText.text = "Current Avatar: " + _currentAvatar;
            SetActiveProfiles();
        });
        _oscQuery.OnOscQueryServiceAdded -= OnOscQueryServiceAdded;
        _initialized = true;
    }

    private void SetStatusText(string text = "")
    {
        var active = !string.IsNullOrWhiteSpace(text);
        statusText.text = text;
        statusText.gameObject.SetActive(active);
        profileContainer.SetActive(!active);
        pagingContainer.SetActive(!active);
    }

    private void OnMessageReceived(BlobString address, OscMessageValues values)
    {
        var addressString = address.ToString();

        if (addressString != "/avatar/change") return;

        var temp = values.ReadStringElement(0);
        if (temp == _currentAvatar)
        {
            return;
        }

        _previousAvatar = _currentAvatar;
        _currentAvatar = values.ReadStringElement(0);
        _mainTheadDispatcher.Enqueue(() =>
        {
            currentAvatarText.text = "Current Avatar: " + _currentAvatar;
            SetActiveProfiles();
        });
        return;
    }

    private static IPAddress GetLocalIPAddress()
    {
        // Android can always serve on the non-loopback address
#if UNITY_ANDROID
        return GetLocalIPAddressNonLoopback();
#else
        // Windows can only serve TCP on the loopback address, but can serve UDP on the non-loopback address
        return IPAddress.Loopback;
#endif
    }

    private static IPAddress GetLocalIPAddressNonLoopback()
    {
        // Get the host name of the local machine
        var hostName = Dns.GetHostName();

        // Get the IP address of the first IPv4 network interface found on the local machine
        return Dns.GetHostEntry(hostName).AddressList
            .FirstOrDefault(ip => ip.AddressFamily == AddressFamily.InterNetwork);
    }

    public void OnApplicationQuit()
    {
        _receiver.Dispose();
        _oscQuery.Dispose();
    }

    public void OnSteamVRConnect()
    {
        if (File.Exists(_manifestFilePath))
        {
            OpenVR.Applications.AddApplicationManifest(_manifestFilePath, false);
        }
    }

    public void OnSteamVRDisconnect()
    {
        Debug.Log("Quitting!");
        Application.Quit();
    }

    public void OnDashBoardOpen()
    {
        if (!_initialized) return;

        _steamVRKeyboardOpen = false;
        currentAvatarText.gameObject.SetActive(true);
        cancelButton.gameObject.SetActive(false);
        newButton.gameObject.SetActive(true);
        copyFromPreviousButton.gameObject.SetActive(true);
    }

    public void OnDashBoardClose()
    {
        return;
    }

    private void SetActiveProfiles()
    {
        profileContainer.SetActive(true);
        foreach (Transform child in profileContainer.transform)
        {
            child.gameObject.SetActive(false);
        }

        var folderPath = Path.Combine(Application.persistentDataPath, $"Profiles/{_currentAvatar}");
        if (!Directory.Exists(folderPath))
        {
            _availableProfiles.Clear();
            _currentPage = 0;
            UpdatePagingButtons();
            return;
        }

        var files = Directory.GetFiles(folderPath, "*");
        _availableProfiles.Clear();
        _availableProfiles.AddRange(files);

        // Ensure current page is valid
        var totalPages = GetTotalPages();
        if (_currentPage >= totalPages)
        {
            _currentPage = Math.Max(0, totalPages - 1);
        }

        DisplayCurrentPage();

        currentAvatarText.gameObject.SetActive(true);
        cancelButton.gameObject.SetActive(false);
        newButton.gameObject.SetActive(true);
        copyFromPreviousButton.gameObject.SetActive(true);
    }

    private void DisplayCurrentPage()
    {
        foreach (Transform child in profileContainer.transform)
        {
            child.gameObject.SetActive(false);
        }

        var startIndex = _currentPage * PageSize;
        var endIndex = Math.Min(startIndex + PageSize, _availableProfiles.Count);

        for (var i = startIndex; i < endIndex; i++)
        {
            var childIndex = i - startIndex;
            var fileName = Path.GetFileName(_availableProfiles[i]);
            var profile = profileContainer.transform.GetChild(childIndex).gameObject;
            profile.SetActive(true);
            profile.transform.GetChild(0).GetComponent<Text>().text = fileName;
        }

        UpdatePagingButtons();
    }

    private int GetTotalPages()
    {
        if (_availableProfiles.Count == 0) return 0;
        return (int)Math.Ceiling((double)_availableProfiles.Count / PageSize);
    }

    private void UpdatePagingButtons()
    {
        var totalPages = GetTotalPages();
        var hasMultiplePages = totalPages > 1;

        pagingContainer.SetActive(hasMultiplePages);
        
        if (hasMultiplePages)
        {
            PrevPageButton.interactable = _currentPage > 0;
            NextPageButton.interactable = _currentPage < totalPages - 1;
        }
    }

    public void NextPage()
    {
        var totalPages = GetTotalPages();
        if (_currentPage < totalPages - 1)
        {
            _currentPage++;
            DisplayCurrentPage();
        }
    }

    public void PrevPage()
    {
        if (_currentPage > 0)
        {
            _currentPage--;
            DisplayCurrentPage();
        }
    }

    public void SetProfile(GameObject profile)
    {
        var profilepath = Path.Combine(Application.persistentDataPath,
            $"Profiles/{_currentAvatar}/{profile.transform.GetChild(0).GetComponent<Text>().text}");
        if (!File.Exists(profilepath))
        {
            Debug.LogError("Profile not found");
            return;
        }

        // Read the json file
        var json = File.ReadAllText(profilepath);
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
        var deleteText = profile.transform.GetChild(2).GetChild(0).GetComponent<Text>();
        if (deleteText.text == "Delete")
        {
            deleteText.text = "Sure?";
            return;
        }

        var profilePath = Path.Combine(Application.persistentDataPath,
            $"Profiles/{_currentAvatar}/{profile.transform.GetChild(0).GetComponent<Text>().text}");
        if (!File.Exists(profilePath))
        {
            Debug.LogError("Profile not found");
            return;
        }

        File.Delete(profilePath);
        profile.SetActive(false);
        deleteText.text = "Delete";
        SetActiveProfiles();
    }

    private System.Collections.IEnumerator SaveProfileDelayed()
    {
        yield return null;
        SaveProfile();
    }

    private void SaveProfile()
    {
        if (string.IsNullOrWhiteSpace(_currentAvatar))
        {
            return;
        }

        if (string.IsNullOrWhiteSpace(_profileText))
        {
            var profileIndex = 1;
            if (_availableProfiles.Count > 0)
            {
                profileIndex = _availableProfiles.Count + 1;
            }

            _profileText = "Profile " + profileIndex;
        }

        if (_queryServiceProfile == null)
        {
            return;
        }

        var folderPath = Path.Combine(Application.persistentDataPath, $"Profiles/{_currentAvatar}");
        if (!Directory.Exists(folderPath))
        {
            Directory.CreateDirectory(folderPath);
        }

        try
        {
            using UnityWebRequest request =
                UnityWebRequest.Get($"http://{_queryServiceProfile.address}:{_queryServiceProfile.port}/");
            var operation = request.SendWebRequest();
            while (!operation.isDone)
                Thread.Sleep(50);

            if (request.result != UnityWebRequest.Result.Success)
            {
                throw new Exception($"Request error: {request.error}");
            }

            var req = request.downloadHandler.text;
            var tree = OSCQueryRootNode.FromString(req);
            var node = tree.GetNodeWithPath("/avatar/parameters");
            var dict = getJson(node);

            var json = JsonConvert.SerializeObject(dict, Formatting.Indented);
            var filePath = Path.Combine(folderPath, $"{_profileText}");
            File.WriteAllText(filePath, json);

            _mainTheadDispatcher.Enqueue(() => { SetActiveProfiles(); });

            _profileText = string.Empty;
            currentAvatarText.gameObject.SetActive(true);
            cancelButton.gameObject.SetActive(false);
            newButton.gameObject.SetActive(true);
            copyFromPreviousButton.gameObject.SetActive(true);
            SetStatusText();
        }
        catch (Exception e)
        {
            Debug.LogError(e);
            Debug.LogWarning("Lost Connection to VRChat, trying to reconnect...");
            SetStatusText("Lost Connection to VRChat, trying to reconnect...");
            _queryServiceProfile = null;
            _oscQuery.OnOscQueryServiceAdded += OnOscQueryServiceAdded;
            return;
        }
    }

    private Dictionary<string, (string value, string OscType)> getJson(OSCQueryNode node)
    {
        var dict = new Dictionary<string, (string value, string OscType)>();
        foreach (var content in node.Contents)
        {
            var value = content.Value.Value;
            if (value is null)
            {
                using var request =
                    UnityWebRequest.Get($"http://{_queryServiceProfile.address}:{_queryServiceProfile.port}/");
                var operation = request.SendWebRequest();
                while (!operation.isDone)
                    Thread.Sleep(50);

                if (request.result != UnityWebRequest.Result.Success)
                {
                    throw new Exception($"Request error: {request.error}");
                }

                var req = request.downloadHandler.text;
                var tree = OSCQueryRootNode.FromString(req);
                var innernode = tree.GetNodeWithPath(content.Value.FullPath);
                var innerdict = getJson(innernode);
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

    private void OnKeyboardDone()
    {
        _steamVRKeyboardOpen = false;
        var finalText = new System.Text.StringBuilder(256);
        OpenVR.Overlay.GetKeyboardText(finalText, 256);
        _profileText = finalText.ToString();
        SetStatusText("Saving Profile...");

        StartCoroutine(SaveProfileDelayed());
    }

    public void CopyFromPreviousAvatar(Text buttontext)
    {
        if (buttontext.text == "Copy From Last")
        {
            buttontext.text = "Sure?";
            return;
        }

        if (string.IsNullOrWhiteSpace(_previousAvatar))
        {
            Debug.LogError("No Previous Avatar set");
            return;
        }

        var previousFolderPath = Path.Combine(Application.persistentDataPath, $"Profiles/{_previousAvatar}");
        var currentFolderPath = Path.Combine(Application.persistentDataPath, $"Profiles/{_currentAvatar}");

        if (!Directory.Exists(previousFolderPath))
        {
            Debug.LogError("Previous Avatar Folder not found");
            return;
        }

        if (!Directory.Exists(currentFolderPath))
        {
            Debug.Log("Creating Directory: " + currentFolderPath);
            Directory.CreateDirectory(currentFolderPath);
        }

        var files = Directory.GetFiles(previousFolderPath, "*");
        foreach (var t in files)
        {
            var fileName = Path.GetFileName(t);
            var sourceFile = Path.Combine(previousFolderPath, fileName);
            var destFile = Path.Combine(currentFolderPath, fileName);
            File.Copy(sourceFile, destFile, true);
        }

        SetActiveProfiles();
        buttontext.text = "Copy From Last";
    }

    public void Cancel()
    {
        if (!_steamVRKeyboardOpen) return;

        OpenVR.Overlay.HideKeyboard();
        _steamVRKeyboardOpen = false;
        SetStatusText();
        currentAvatarText.gameObject.SetActive(true);
        cancelButton.gameObject.SetActive(false);
        newButton.gameObject.SetActive(true);
        copyFromPreviousButton.gameObject.SetActive(true);
    }

    public void NewProfile()
    {
        ShowKeyboard();
    } 

    private void ShowKeyboard()
    {
        if (OpenVR.Overlay == null || menuOverlay == null || menuOverlay.overlay == null) return;

        var overlayHandle = menuOverlay.overlay.overlayHandle;

        if (overlayHandle == OpenVR.k_ulOverlayHandleInvalid)
        {
            Debug.LogError("Overlay handle is invalid - menuOverlay may not be initialized");
            return;
        }

        Debug.Log($"Using overlay handle: {overlayHandle}");

        var error = OpenVR.Overlay.ShowKeyboardForOverlay(
            overlayHandle,
            (int)EGamepadTextInputMode.k_EGamepadTextInputModeNormal,
            (int)EGamepadTextInputLineMode.k_EGamepadTextInputLineModeSingleLine,
            0,
            "Enter Profile Name",
            255,
            _profileText,
            0
        );

        if (error != EVROverlayError.None)
        {
            Debug.LogError($"Failed to show SteamVR keyboard: {error}");
            return;
        }

        currentAvatarText.gameObject.SetActive(true);
        cancelButton.gameObject.SetActive(true);
        newButton.gameObject.SetActive(false);
        copyFromPreviousButton.gameObject.SetActive(false);

        SetStatusText("Enter Profile Name");
        Debug.Log("SteamVR keyboard opened successfully");
        _steamVRKeyboardOpen = true;
    }
}
