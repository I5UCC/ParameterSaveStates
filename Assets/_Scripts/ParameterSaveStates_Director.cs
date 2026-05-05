using System;
using System.IO;
using UnityEngine;
using UnityEngine.UI;
using Valve.VR;
using PimDeWitte.UnityMainThreadDispatcher;

public class ParameterSaveStates_Director : MonoBehaviour
{
    private enum KeyboardMode
    {
        None,
        NewProfile,
        RenameProfile
    }
    
    #region Unity Inspector Fields
    
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
    public Button nextPageButton;
    public Button prevPageButton;
    public Text pageNumberText;
    
    [Space(10)] 

    [Header("Web UI")]
    [SerializeField] private bool enableWebUi = true;
    [SerializeField] private int webUiPort = 17663;
    [SerializeField] private WindowController windowController;
    
    #endregion

    #region Private Fields

    private readonly string _manifestFilePath = Path.GetFullPath("app.vrmanifest");
    
    private string _previousAvatar;
    private string _currentAvatar;
    private UnityMainThreadDispatcher _mainThreadDispatcher;
    private bool _steamVRKeyboardOpen;
    private string _profileText = string.Empty;
    private bool _initialized;
    private KeyboardMode _keyboardMode = KeyboardMode.None;
    
    private string _profileToRename;

    private OscService _oscService;
    private ProfileService _profileService;
    private ProfileService _webProfileService;
    private WebUiService _webUiService;

    #endregion

    #region Handler Methods

    private void Start()
    {
        _mainThreadDispatcher = UnityMainThreadDispatcher.Instance();
        
        _oscService = new OscService();
        _profileService = new ProfileService(_oscService, profileContainer.transform.childCount);
        
        SetStatusText("Waiting for VRChat to connect...");
        
        pagingContainer.SetActive(false);
        currentAvatarText.gameObject.SetActive(false);
        cancelButton.gameObject.SetActive(false);
        newButton.gameObject.SetActive(false);
        copyFromPreviousButton.gameObject.SetActive(false);

        if (menuOverlay != null && menuOverlay.cameraForTexture != null)
            menuOverlay.cameraForTexture.enabled = false;

        _oscService.OnVRChatConnected += OnVRChatConnected;
        _oscService.OnAvatarChanged += OnAvatarChanged;
        _oscService.Initialize();

        if (enableWebUi)
        {
            if (webUiPort <= 0)
            {
                Debug.LogWarning("Web UI port is invalid. Web UI will not start.");
            }
            else
            {
                var indexPath = Path.Combine(Application.streamingAssetsPath, "WebUi", "index.html");
                _webProfileService = new ProfileService(_oscService, profileContainer.transform.childCount);
                _webUiService = new WebUiService(
                    _webProfileService,
                    _oscService,
                    _mainThreadDispatcher,
                    () => _currentAvatar,
                    () => _previousAvatar,
                    webUiPort,
                    indexPath);
                _webUiService.Start();
                ConfigureTrayMenu();

                if (!IsSteamVrRunning())
                {
                    OpenWebUi();
                }
            }
        }

        if (menuOverlay != null && menuOverlay.overlay != null)
        {
            menuOverlay.overlay.onKeyboardDone += OnKeyboardDone;
            menuOverlay.overlay.onKeyboardClosed += OnKeyboardCancel;
        }
    }
    
    public void OnApplicationQuit()
    {
        _webUiService?.Dispose();
        _oscService?.Dispose();
    }

    public void OnSteamVRConnect()
    {
        if (!File.Exists(_manifestFilePath)) return;
        
        OpenVR.Applications.AddApplicationManifest(_manifestFilePath, false);
        Debug.Log("Added VR manifest to SteamVR");
        
        if (menuOverlay != null && menuOverlay.cameraForTexture != null)
            menuOverlay.cameraForTexture.enabled = false;
    }

    public void OnSteamVRDisconnect()
    {
        Debug.Log("SteamVR disconnected");
        if (menuOverlay != null && menuOverlay.cameraForTexture != null)
            menuOverlay.cameraForTexture.enabled = false;

        if (enableWebUi && _webUiService != null && _webUiService.IsRunning)
        {
            OpenWebUi();
            return;
        }
        Application.Quit();
    }

    public void OnDashBoardOpen()
    {
        if (!_initialized) return;

        if (menuOverlay != null && menuOverlay.cameraForTexture != null)
            menuOverlay.cameraForTexture.enabled = true;

        _steamVRKeyboardOpen = false;
        currentAvatarText.gameObject.SetActive(true);
        cancelButton.gameObject.SetActive(false);
        newButton.gameObject.SetActive(true);
        copyFromPreviousButton.gameObject.SetActive(true);
        copyFromPreviousButton.gameObject.GetDisplayNameText().text = "Copy From Last";
        RefreshProfiles();
    }

    public void OnDashBoardClose()
    {
        OpenVR.Overlay.HideKeyboard();
        _steamVRKeyboardOpen = false;
        
        if (menuOverlay != null && menuOverlay.cameraForTexture != null)
            menuOverlay.cameraForTexture.enabled = false;
    }

    #endregion

    private void ConfigureTrayMenu()
    {
        if (windowController == null)
        {
            windowController = FindAnyObjectByType<WindowController>();
        }

        if (windowController != null)
        {
            windowController.SetOpenWebUiAction(OpenWebUi);
            windowController.SetOpenWebUiBrowserAction(OpenWebUiBrowser);
        }
    }

    private void OpenWebUiBrowser()
    {
        if (!enableWebUi || _webUiService == null || !_webUiService.IsRunning)
            return;

        Application.OpenURL(_webUiService.BaseUrl);
    }

    private void OpenWebUi()
    {
        if (!enableWebUi || _webUiService == null || !_webUiService.IsRunning)
            return;

        void Open()
        {
            if (windowController != null
                && windowController.TryShowWebUiWindow(_webUiService.BaseUrl))
            {
                return;
            }

            OpenWebUiBrowser();
        }

        if (_mainThreadDispatcher != null)
        {
            _mainThreadDispatcher.Enqueue(Open);
        }
        else
        {
            Open();
        }
    }

    private static bool IsSteamVrRunning()
    {
#if UNITY_STANDALONE_WIN
        return System.Diagnostics.Process.GetProcessesByName("vrserver").Length > 0
               || System.Diagnostics.Process.GetProcessesByName("vrmonitor").Length > 0
               || System.Diagnostics.Process.GetProcessesByName("steamvr").Length > 0;
#else
        return true;
#endif
    }

    #region Button Handlers
    
    public void NextPage()
    {
        if (_profileService.NextPage())
        {
            DisplayCurrentPage();
        }
    }

    public void PrevPage()
    {
        if (_profileService.PrevPage())
        {
            DisplayCurrentPage();
        }
    }

    public void ApplyProfile(GameObject profile)
    {
        var displayName = profile.GetDisplayNameText();
        _profileService.ApplyProfile(displayName.text);
    }

    public void EditProfile(GameObject profile)
    {
        var editButtonText = profile.GetEditButtonText().text;
        EnableEditButtons(profile, editButtonText == "Edit");
    }

    public void DeleteProfile(GameObject profile)
    {
        var deleteText = profile.GetDeleteText();
        if (deleteText.text == "Delete")
        {
            deleteText.text = "U Sure?";
            return;
        }
        EnableEditButtons(profile, false);

        var displayName = profile.GetDisplayNameText().text;
        if (_profileService.DeleteProfile(displayName))
        {
            profile.SetActive(false);
            deleteText.text = "Delete";
            RefreshProfiles();
        }
    }

    public void OverrideProfile(GameObject profile)
    {
        var overrideProfileText = profile.GetOverrideProfileText();
        if (overrideProfileText.text == "Override")
        {
            overrideProfileText.text = "U Sure?";
            return;
        }
        EnableEditButtons(profile, false);

        _profileService.OverrideProfile(_currentAvatar, profile.GetDisplayNameText().text);
    }

    public void RenameProfile(GameObject profile)
    {
        var renameText = profile.GetRenameText();
        if (renameText.text == "Rename")
        {
            renameText.text = "U Sure?";
            return;
        }
        EnableEditButtons(profile, false);

        _keyboardMode = KeyboardMode.RenameProfile;
        _profileToRename = profile.GetDisplayNameText().text;
        ShowKeyboard("Enter New Profile Name", _profileToRename);
    }

    public void MoveProfileUp(GameObject profile)
    {
        MoveProfile(profile, true);
    }

    public void MoveProfileDown(GameObject profile)
    {
        MoveProfile(profile, false);
    }

    private void EnableEditButtons(GameObject profile, bool enable)
    {
        var editButton = profile.GetEditButton();
        var editText = editButton.GetButtonTextComponent();
        var editContainer = profile.GetEditContainer();
        var moveButtonContainer = profile.GetMoveButtonContainer();

        editButton.GetImageComponent().color = enable ? new Color(0.020f, 0.765f, 0f) : new Color(0.631f, 0.380f, 0f);
        editText.text = enable ? "Done" : "Edit";
        editContainer.gameObject.SetActive(enable);
        if (moveButtonContainer != null)
        {
            moveButtonContainer.gameObject.SetActive(enable);
        }
        if (!enable)
        {
            profile.ResetEditButtons();
        }
    }

    private void MoveProfile(GameObject profile, bool moveUp)
    {
        if (profile == null)
        {
            return;
        }

        var displayName = profile.GetDisplayNameText().text;
        if (!_profileService.MoveProfile(_currentAvatar, displayName, moveUp))
        {
            Debug.LogWarning("Unable to move profile.");
            return;
        }

        RefreshProfiles();
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

        _profileService.CopyProfilesFromAvatar(_previousAvatar, _currentAvatar);
        RefreshProfiles();
        buttontext.text = "Copy From Last";
    }

    public void Cancel()
    {
        if (!_steamVRKeyboardOpen) return;

        OpenVR.Overlay.HideKeyboard();
        _steamVRKeyboardOpen = false;
        _keyboardMode = KeyboardMode.None;
        SetStatusText();
    }

    public void NewProfile()
    {
        _keyboardMode = KeyboardMode.NewProfile;
        ShowKeyboard("Enter Profile Name");
        cancelButton.gameObject.SetActive(true);
        newButton.gameObject.SetActive(false);
        copyFromPreviousButton.gameObject.SetActive(false);
        pagingContainer.SetActive(false);
    }

    private void SaveProfile()
    {
        try
        {
            _profileService.SaveProfile(_currentAvatar, _profileText);
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
            _oscService.ReconnectToVRChat();
        }
    }
    #endregion

    #region Event Handlers

    private void OnVRChatConnected(VRC.OSCQuery.OSCQueryServiceProfile profile)
    {
        _initialized = true;
        _mainThreadDispatcher.Enqueue(() =>
        {
            SetStatusText();
            DisplayCurrentPage();
        });
    }

    private void OnAvatarChanged(string previousAvatar, string newAvatar)
    {
        if (newAvatar == _currentAvatar) return;

        _previousAvatar = _currentAvatar;
        _currentAvatar = newAvatar;
        
        _mainThreadDispatcher.Enqueue(() =>
        {
            SetStatusText();
            UpdateCurrentAvatarDisplay();
            RefreshProfiles();
        });
    }

    #endregion

    #region Overlay Methods

    private void UpdateCurrentAvatarDisplay()
    {
        var customName = _profileService.LoadAvatarName(_currentAvatar);
        currentAvatarText.text = !string.IsNullOrWhiteSpace(customName) 
            ? customName
            : _currentAvatar;
    }

    private void SetStatusText(string text = "")
    {
        var active = !string.IsNullOrWhiteSpace(text);
        statusText.text = text;
        statusText.gameObject.SetActive(active);
        profileContainer.SetActive(!active);
        currentAvatarText.gameObject.SetActive(true);
        cancelButton.gameObject.SetActive(active);
        newButton.gameObject.SetActive(!active);
        copyFromPreviousButton.gameObject.SetActive(!active);
        if (!active) DisplayCurrentPage();
    }

    private void RefreshProfiles()
    {
        profileContainer.SetActive(true);
        foreach (Transform child in profileContainer.transform)
        {
            child.gameObject.SetActive(false);
        }

        _profileService.LoadProfiles(_currentAvatar);
        DisplayCurrentPage();
    }

    private void DisplayCurrentPage()
    {
        foreach (Transform child in profileContainer.transform)
        {
            child.gameObject.SetActive(false);
        }

        var profiles = _profileService.GetCurrentPageProfiles();
        var allProfileNames = _profileService.GetAllProfileDisplayNames();
        for (var i = 0; i < profiles.Count; i++)
        {
            var profile = profileContainer.transform.GetChild(i).gameObject;
            profile.SetActive(true);
            profile.GetDisplayNameText().text = profiles[i].displayName;
            UpdateMoveButtonState(profile, profiles[i].displayName, allProfileNames);
        }

        UpdatePagingButtons();
    }

    private static void UpdateMoveButtonState(GameObject profile, string displayName, System.Collections.Generic.List<string> allProfileNames)
    {
        var moveContainer = profile.GetMoveButtonContainer();
        if (moveContainer == null)
        {
            return;
        }

        var upButtonTransform = moveContainer.Find("Up");
        var downButtonTransform = moveContainer.Find("Down");
        if (upButtonTransform == null || downButtonTransform == null)
        {
            return;
        }

        var upButton = upButtonTransform.GetComponent<Button>();
        var downButton = downButtonTransform.GetComponent<Button>();
        if (upButton == null || downButton == null)
        {
            return;
        }

        var index = allProfileNames.IndexOf(displayName);
        upButton.interactable = index > 0;
        downButton.interactable = index >= 0 && index < allProfileNames.Count - 1;
    }

    private void UpdatePagingButtons()
    {
        var totalPages = _profileService.TotalPages;
        var hasMultiplePages = totalPages > 1;

        pageNumberText.text = _profileService.CurrentPage + 1 + "/" + totalPages;
        pagingContainer.SetActive(hasMultiplePages);

        if (!hasMultiplePages) return;
        
        prevPageButton.interactable = _profileService.CurrentPage > 0;
        nextPageButton.interactable = _profileService.CurrentPage < totalPages - 1;
    }
    
    #endregion

    #region Keyboard events and methods

    private void OnKeyboardDone()
    {
        _steamVRKeyboardOpen = false;
        var finalText = new System.Text.StringBuilder(256);
        OpenVR.Overlay.GetKeyboardText(finalText, 256);
        _profileText = finalText.ToString();

        switch (_keyboardMode)
        {
            case KeyboardMode.NewProfile:
                SetStatusText("Saving Profile...");
                SaveProfile();
                RefreshProfiles();
                break;
            case KeyboardMode.RenameProfile:
                _profileService.RenameProfile(_profileToRename, _profileText);
                _profileToRename = string.Empty;
                RefreshProfiles();
                break;
            case KeyboardMode.None:
                SetStatusText();
                break;
            default:
                throw new ArgumentOutOfRangeException();
        }
        _keyboardMode = KeyboardMode.None;
    }
    
    private void OnKeyboardCancel()
    {
        _steamVRKeyboardOpen = false;
        SetStatusText();
    }

    private void ShowKeyboard(string description, string existingText = "")
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
            description,
            255,
            existingText,
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

        SetStatusText(description);
        Debug.Log("SteamVR keyboard opened successfully");
        _steamVRKeyboardOpen = true;
    }
    
    #endregion
}
