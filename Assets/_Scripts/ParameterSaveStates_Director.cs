using System;
using System.IO;
using UnityEngine;
using UnityEngine.UI;
using Valve.VR;
using PimDeWitte.UnityMainThreadDispatcher;
using UnityEngine.Serialization;

public class ParameterSaveStates_Director : MonoBehaviour
{
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
    public Button setNameButton;
    
    #endregion

    #region Private Fields

    private readonly string _manifestFilePath = Path.GetFullPath("app.vrmanifest");
    
    private string _previousAvatar;
    private string _currentAvatar;
    private UnityMainThreadDispatcher _mainThreadDispatcher;
    private bool _steamVRKeyboardOpen;
    private string _profileText = string.Empty;
    private bool _initialized;
    private bool _isSettingName;

    private OscService _oscService;
    private ProfileService _profileService;

    #endregion

    #region Handler Methods

    private void Start()
    {
        _mainThreadDispatcher = UnityMainThreadDispatcher.Instance();
        
        _oscService = new OscService();
        _profileService = new ProfileService(_oscService);
        
        SetStatusText("Waiting for VRChat to connect...");

        currentAvatarText.gameObject.SetActive(false);
        cancelButton.gameObject.SetActive(false);
        newButton.gameObject.SetActive(false);
        copyFromPreviousButton.gameObject.SetActive(false);

        _oscService.OnVRChatConnected += OnVRChatConnected;
        _oscService.OnAvatarChanged += OnAvatarChanged;
        _oscService.Initialize();

        if (menuOverlay != null && menuOverlay.overlay != null)
        {
            menuOverlay.overlay.onKeyboardDone += OnKeyboardDone;
        }
    }
    
    public void OnApplicationQuit()
    {
        _oscService?.Dispose();
    }

    public void OnSteamVRConnect()
    {
        if (!File.Exists(_manifestFilePath))
        {
            var manifest = @"{
	""source"": ""builtin"",
	""applications"": [
		{
			""app_key"": ""i5ucc.parametersavestates"",
			""launch_type"": ""binary"",
			""binary_path_windows"": ""./ParameterSaveStates.exe"",
			""is_dashboard_overlay"": true,
			""strings"": {
				""en_us"": {
					""name"": ""ParameterSaveStates"",
					""description"": ""ParameterSaveStates for VRChat Avatars""
				}
			}
		}
	]
}";
            File.WriteAllText(_manifestFilePath, manifest);
            Debug.Log($"Created missing VR manifest at: {_manifestFilePath}");
        }
        
        OpenVR.Applications.AddApplicationManifest(_manifestFilePath, false);
        Debug.Log("Added VR manifest to SteamVR");
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
        setNameButton.gameObject.SetActive(true);
    }

    public void OnDashBoardClose()
    {
        return;
    }

    #endregion

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

    public void SetProfile(GameObject profile)
    {
        var displayName = profile.transform.GetChild(0).GetComponent<Text>().text;
        _profileService.ApplyProfile(displayName);
    }

    public void DeleteProfile(GameObject profile)
    {
        var deleteText = profile.transform.GetChild(2).GetChild(0).GetComponent<Text>();
        if (deleteText.text == "Delete")
        {
            deleteText.text = "Sure?";
            return;
        }

        var displayName = profile.transform.GetChild(0).GetComponent<Text>().text;
        if (_profileService.DeleteProfile(displayName))
        {
            profile.SetActive(false);
            deleteText.text = "Delete";
            RefreshProfiles();
        }
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
        _isSettingName = false;
        SetStatusText();
        currentAvatarText.gameObject.SetActive(true);
        cancelButton.gameObject.SetActive(false);
        newButton.gameObject.SetActive(true);
        copyFromPreviousButton.gameObject.SetActive(true);
    }

    public void NewProfile()
    {
        _isSettingName = false;
        ShowKeyboard("Enter Profile Name");
    }

    public void SetAvatarName()
    {
        _isSettingName = true;
        var currentName = _profileService.LoadAvatarName(_currentAvatar) ?? "";
        ShowKeyboard("Enter Avatar Name", currentName);
    }
    
    private System.Collections.IEnumerator SaveProfileDelayed()
    {
        yield return null;
        SaveProfile();
    }

    private void SaveProfile()
    {
        try
        {
            if (_profileService.SaveProfile(_currentAvatar, _profileText))
            {
                _mainThreadDispatcher.Enqueue(() => { RefreshProfiles(); });
            }

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
            ? $"Current Avatar: {customName}" 
            : $"Current Avatar: {_currentAvatar}";
    }

    private void SetStatusText(string text = "")
    {
        var active = !string.IsNullOrWhiteSpace(text);
        statusText.text = text;
        statusText.gameObject.SetActive(active);
        profileContainer.SetActive(!active);
        pagingContainer.SetActive(!active);
        setNameButton.gameObject.SetActive(!active);
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

        var profiles = _profileService.GetCurrentPageProfiles();
        for (var i = 0; i < profiles.Count; i++)
        {
            var profile = profileContainer.transform.GetChild(i).gameObject;
            profile.SetActive(true);
            profile.transform.GetChild(0).GetComponent<Text>().text = profiles[i].displayName;
        }

        UpdatePagingButtons();
    }

    private void UpdatePagingButtons()
    {
        var totalPages = _profileService.TotalPages;
        var hasMultiplePages = totalPages > 1;
        
        pageNumberText.text = totalPages.ToString();
        pagingContainer.SetActive(hasMultiplePages);
        
        if (hasMultiplePages)
        {
            prevPageButton.interactable = _profileService.CurrentPage > 0;
            nextPageButton.interactable = _profileService.CurrentPage < totalPages - 1;
        }
    }
    
    #endregion

    #region Keyboard events and methods

    private void OnKeyboardDone()
    {
        _steamVRKeyboardOpen = false;
        var finalText = new System.Text.StringBuilder(256);
        OpenVR.Overlay.GetKeyboardText(finalText, 256);
        _profileText = finalText.ToString();

        if (_isSettingName)
        {
            _isSettingName = false;
            if (!string.IsNullOrWhiteSpace(_profileText))
            {
                _profileService.SaveAvatarName(_currentAvatar, _profileText);
                UpdateCurrentAvatarDisplay();
            }
            _profileText = string.Empty;
            SetStatusText();
            currentAvatarText.gameObject.SetActive(true);
            cancelButton.gameObject.SetActive(false);
            newButton.gameObject.SetActive(true);
            copyFromPreviousButton.gameObject.SetActive(true);
        }
        else
        {
            SetStatusText("Saving Profile...");
            StartCoroutine(SaveProfileDelayed());
        }
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
