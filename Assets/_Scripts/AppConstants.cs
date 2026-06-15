using UnityEngine;

/// <summary>
/// Shared application-wide constants. Single source of truth — do not duplicate in individual classes.
/// </summary>
public static class AppConstants
{
    /// <summary>Root folder name under <c>Application.persistentDataPath</c> where all profile data is stored.</summary>
    public const string ProfilesRootFolderName = "Profiles";

    /// <summary>
    /// Window/tray title for the Web UI window. Used by <c>WindowController</c> for Win32 window-handle search
    /// and by <c>TrayForm</c> for the system-tray tooltip — both must match.
    /// </summary>
    public const string WebUiWindowTitle = "Parameter Save States";

    private const string DefaultVersion = "dev";

    /// <summary>
    /// Version string loaded from Resources/BuildVersion.txt, then fallback.
    /// </summary>
    public static string CurrentVersion
    {
        get
        {
            var fromResources = Resources.Load<TextAsset>("BuildVersion");
            if (fromResources != null)
            {
                var value = (fromResources.text ?? string.Empty).Trim();
                if (!string.IsNullOrEmpty(value))
                {
                    return value;
                }
            }

            return DefaultVersion;
        }
    }
}

