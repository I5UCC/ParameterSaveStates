using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Threading;
using Newtonsoft.Json;
using UnityEngine;
using UnityEngine.Networking;
using VRC.OSCQuery;

public class ProfileService
{
    private readonly OscService _oscService;
    private const string NameFile = "Name";
    private const string ProfilesRootFolderName = "Profiles";
    private const string AvatarMetadataFolderName = "Avatars";
    private const string AvatarMetadataFilePattern = "avtr_*.json";
    
    private List<string> AvailableProfiles { get; set; } = new List<string>();
    
    private int PageSize { get; }
    
    public int CurrentPage { get; private set; } = 0;
    
    public int TotalPages => AvailableProfiles.Count == 0 ? 0 : (int)Math.Ceiling((double)AvailableProfiles.Count / PageSize);

    public ProfileService(OscService oscService, int pageSize)
    {
        _oscService = oscService;
        PageSize = pageSize;
    }

    public void LoadProfiles(string avatarId)
    {
        AvailableProfiles.Clear();

        var folderPath = Path.Combine(GetProfilesRootPath(), avatarId);
        if (!Directory.Exists(folderPath))
        {
            CurrentPage = 0;
            return;
        }

        var sortedFiles = GetOrderedProfileEntries(folderPath)
            .Select(entry => entry.Path)
            .ToList();

        AvailableProfiles.AddRange(sortedFiles);

        var totalPages = TotalPages;
        if (CurrentPage >= totalPages)
        {
            CurrentPage = Math.Max(0, totalPages - 1);
        }
    }

    public List<(string path, string displayName)> GetCurrentPageProfiles()
    {
        var result = new List<(string, string)>();
        var startIndex = CurrentPage * PageSize;
        var endIndex = Math.Min(startIndex + PageSize, AvailableProfiles.Count);

        for (var i = startIndex; i < endIndex; i++)
        {
            var fileName = Path.GetFileName(AvailableProfiles[i]);
            var displayName = GetProfileDisplayName(fileName);
            result.Add((AvailableProfiles[i], displayName));
        }

        return result;
    }

    public List<string> GetAllProfileDisplayNames()
    {
        return AvailableProfiles
            .Select(path => GetProfileDisplayName(Path.GetFileName(path)))
            .ToList();
    }

    public bool NextPage()
    {
        if (CurrentPage < TotalPages - 1)
        {
            CurrentPage++;
            return true;
        }
        return false;
    }

    public bool PrevPage()
    {
        if (CurrentPage > 0)
        {
            CurrentPage--;
            return true;
        }
        return false;
    }

    public void ApplyProfile(string displayName)
    {
        var profilePath = AvailableProfiles.FirstOrDefault(p => GetProfileDisplayName(Path.GetFileName(p)) == displayName);

        if (string.IsNullOrEmpty(profilePath) || !File.Exists(profilePath))
        {
            Debug.LogError("Profile not found");
            return;
        }

        var json = File.ReadAllText(profilePath);
        var dict = JsonConvert.DeserializeObject<Dictionary<string, (string, string)>>(json);

        foreach (var item in dict)
        {
            switch (item.Value.Item2)
            {
                case "f":
                    _oscService.SendFloat(item.Key, float.Parse(item.Value.Item1));
                    break;
                case "i":
                    _oscService.SendInt(item.Key, int.Parse(item.Value.Item1));
                    break;
                case "T":
                    _oscService.SendBool(item.Key, bool.Parse(item.Value.Item1));
                    break;
                default:
                    Debug.LogError("Unknown OSC Type");
                    break;
            }
        }
    }

    public bool DeleteProfile(string displayName)
    {
        var profilePath = AvailableProfiles.FirstOrDefault(p => GetProfileDisplayName(Path.GetFileName(p)) == displayName);

        if (string.IsNullOrEmpty(profilePath) || !File.Exists(profilePath))
        {
            Debug.LogError("Profile not found");
            return false;
        }

        File.Delete(profilePath);
        return true;
    }

    public bool SaveProfile(string avatarId, string profileName)
    {
        if (string.IsNullOrWhiteSpace(avatarId))
            return false;

        var queryProfile = _oscService.QueryServiceProfile;
        if (queryProfile == null)
            return false;

        var nextIndex = GetNextProfileIndex();
        if (string.IsNullOrWhiteSpace(profileName))
        {
            profileName = "Profile " + nextIndex;
        }

        var folderPath = Path.Combine(GetProfilesRootPath(), avatarId);
        if (!Directory.Exists(folderPath))
        {
            Directory.CreateDirectory(folderPath);
        }

        // Check if a profile with this name already exists — override it instead of creating a duplicate
        var existingProfile = AvailableProfiles.FirstOrDefault(p =>
            GetProfileDisplayName(Path.GetFileName(p)) == profileName);

        using UnityWebRequest request =
            UnityWebRequest.Get($"http://{queryProfile.address}:{queryProfile.port}/");
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
        var dict = GetParametersJson(node, queryProfile);

        var json = JsonConvert.SerializeObject(dict, Formatting.Indented);

        string filePath;
        if (!string.IsNullOrEmpty(existingProfile) && File.Exists(existingProfile))
        {
            filePath = existingProfile;
        }
        else
        {
            var indexedFileName = $"{nextIndex}_{profileName}";
            filePath = Path.Combine(folderPath, indexedFileName);
        }

        File.WriteAllText(filePath, json);

        return true;
    }

    public void CopyProfilesFromAvatar(string sourceAvatarId, string targetAvatarId)
    {
        var previousFolderPath = Path.Combine(GetProfilesRootPath(), sourceAvatarId);
        var currentFolderPath = Path.Combine(GetProfilesRootPath(), targetAvatarId);

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
    }

    private Dictionary<string, (string value, string OscType)> GetParametersJson(OSCQueryNode node, OSCQueryServiceProfile queryProfile)
    {
        var dict = new Dictionary<string, (string value, string OscType)>();
        foreach (var content in node.Contents)
        {
            var value = content.Value.Value;
            if (value is null)
            {
                using var request =
                    UnityWebRequest.Get($"http://{queryProfile.address}:{queryProfile.port}/");
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
                var innerdict = GetParametersJson(innernode, queryProfile);
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

    private static int GetProfileIndex(string filePath)
    {
        var fileName = Path.GetFileName(filePath);
        var (_, index, _) = ParseProfileFileName(fileName);
        return index;
    }

    private static string GetProfileDisplayName(string fileName)
    {
        var (_, _, displayName) = ParseProfileFileName(fileName);
        return displayName;
    }

    private int GetNextProfileIndex()
    {
        if (AvailableProfiles.Count == 0) return 1;

        var maxIndex = AvailableProfiles
            .Select(GetProfileIndex)
            .Where(i => i != int.MaxValue)
            .DefaultIfEmpty(0)
            .Max();

        return maxIndex + 1;
    }
    
    private string GetAvatarNameFilePath(string avatarId)
    {
        var folderPath = Path.Combine(GetProfilesRootPath(), avatarId);
        return Path.Combine(folderPath, NameFile);
    }

    public string LoadAvatarName(string avatarId)
    {
        if (string.IsNullOrWhiteSpace(avatarId))
        {
            return null;
        }

        var savedAvatarName = ReadSavedAvatarName(avatarId);
        var metadataAvatarName = FindAvatarNameInVrChatOscCache(avatarId);
        if (string.IsNullOrWhiteSpace(metadataAvatarName))
        {
            return savedAvatarName;
        }

        if (string.Equals(savedAvatarName, metadataAvatarName, StringComparison.Ordinal))
        {
            return savedAvatarName;
        }

        SaveAvatarName(avatarId, metadataAvatarName);
        return metadataAvatarName;
    }

    public void SaveAvatarName(string avatarId, string avatarName)
    {
        if (string.IsNullOrWhiteSpace(avatarId) || string.IsNullOrWhiteSpace(avatarName))
        {
            return;
        }

        var folderPath = Path.Combine(GetProfilesRootPath(), avatarId);
        if (!Directory.Exists(folderPath))
        {
            Directory.CreateDirectory(folderPath);
        }
        var filePath = GetAvatarNameFilePath(avatarId);
        File.WriteAllText(filePath, avatarName.Trim());
    }

    private string ReadSavedAvatarName(string avatarId)
    {
        var filePath = GetAvatarNameFilePath(avatarId);
        if (!File.Exists(filePath))
        {
            return null;
        }

        var value = File.ReadAllText(filePath).Trim();
        return string.IsNullOrWhiteSpace(value) ? null : value;
    }

    private static string FindAvatarNameInVrChatOscCache(string avatarId)
    {
        var oscRootPath = GetVrChatOscRootPath();
        if (string.IsNullOrWhiteSpace(oscRootPath) || !Directory.Exists(oscRootPath))
        {
            return null;
        }

        var userDirectories = Directory.GetDirectories(oscRootPath, "usr_*");
        foreach (var userDirectory in userDirectories)
        {
            var avatarsDirectory = Path.Combine(userDirectory, AvatarMetadataFolderName);
            if (!Directory.Exists(avatarsDirectory))
            {
                continue;
            }

            var exactPath = Path.Combine(avatarsDirectory, $"{avatarId}.json");
            if (File.Exists(exactPath) && TryReadAvatarNameFromMetadataFile(exactPath, avatarId, out var exactAvatarName))
            {
                return exactAvatarName;
            }

            var metadataFiles = Directory.GetFiles(avatarsDirectory, AvatarMetadataFilePattern);
            foreach (var metadataFile in metadataFiles)
            {
                if (string.Equals(metadataFile, exactPath, StringComparison.OrdinalIgnoreCase))
                {
                    continue;
                }

                if (TryReadAvatarNameFromMetadataFile(metadataFile, avatarId, out var avatarName))
                {
                    return avatarName;
                }
            }
        }

        return null;
    }

    private static bool TryReadAvatarNameFromMetadataFile(string metadataFilePath, string avatarId, out string avatarName)
    {
        avatarName = null;

        string json;
        try
        {
            json = File.ReadAllText(metadataFilePath);
        }
        catch (IOException ex)
        {
            Debug.LogWarning($"Failed to read avatar metadata file '{metadataFilePath}': {ex.Message}");
            return false;
        }
        catch (UnauthorizedAccessException ex)
        {
            Debug.LogWarning($"Access denied reading avatar metadata file '{metadataFilePath}': {ex.Message}");
            return false;
        }

        VrChatAvatarMetadata metadata;
        try
        {
            metadata = JsonConvert.DeserializeObject<VrChatAvatarMetadata>(json);
        }
        catch (JsonException ex)
        {
            Debug.LogWarning($"Failed to parse avatar metadata file '{metadataFilePath}': {ex.Message}");
            return false;
        }

        if (metadata == null || !string.Equals(metadata.id, avatarId, StringComparison.Ordinal))
        {
            return false;
        }

        if (string.IsNullOrWhiteSpace(metadata.name))
        {
            return false;
        }

        avatarName = metadata.name.Trim();
        return true;
    }

    private static string GetVrChatOscRootPath()
    {
#if UNITY_STANDALONE_WIN || UNITY_EDITOR_WIN
        var localAppDataPath = Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData);
        if (string.IsNullOrWhiteSpace(localAppDataPath))
        {
            return null;
        }

        var localLowPath = Path.GetFullPath(Path.Combine(localAppDataPath, "..", "LocalLow"));
        return Path.Combine(localLowPath, "VRChat", "VRChat", "OSC");
#else
        return null;
#endif
    }

    private sealed class VrChatAvatarMetadata
    {
        public string id;
        public string name;
    }

    public List<(string avatarId, string avatarName)> GetAvatarsWithSavedProfiles()
    {
        var result = new List<(string avatarId, string avatarName)>();
        var profilesRootPath = GetProfilesRootPath();
        if (!Directory.Exists(profilesRootPath))
        {
            return result;
        }

        var avatarDirectories = Directory.GetDirectories(profilesRootPath);
        foreach (var avatarDirectory in avatarDirectories)
        {
            var avatarId = Path.GetFileName(avatarDirectory);
            if (string.IsNullOrWhiteSpace(avatarId))
            {
                continue;
            }

            var hasProfiles = Directory.GetFiles(avatarDirectory, "*")
                .Any(file => !string.Equals(Path.GetFileName(file), NameFile, StringComparison.Ordinal));
            if (!hasProfiles)
            {
                continue;
            }

            result.Add((avatarId, LoadAvatarName(avatarId)));
        }

        return result
            .OrderBy(entry => string.IsNullOrWhiteSpace(entry.avatarName) ? entry.avatarId : entry.avatarName, StringComparer.OrdinalIgnoreCase)
            .ToList();
    }

    public bool DeleteAvatarProfiles(string avatarId)
    {
        if (string.IsNullOrWhiteSpace(avatarId))
        {
            return false;
        }

        var avatarFolderPath = Path.Combine(GetProfilesRootPath(), avatarId);
        if (!Directory.Exists(avatarFolderPath))
        {
            return false;
        }

        Directory.Delete(avatarFolderPath, true);
        return true;
    }

    private static string GetProfilesRootPath()
    {
        return Path.Combine(Application.persistentDataPath, ProfilesRootFolderName);
    }

    public bool RenameProfile(string displayName, string nameOverride)
    {
        var profilePath = AvailableProfiles.FirstOrDefault(p => GetProfileDisplayName(Path.GetFileName(p)) == displayName);

        if (profilePath == null || !File.Exists(profilePath))
            return false;

        var currentFileName = Path.GetFileName(profilePath);
        var (hasIndex, index, _) = ParseProfileFileName(currentFileName);
        var directory = Path.GetDirectoryName(profilePath);
        var newFileName = hasIndex ? $"{index}_{nameOverride}" : nameOverride;
        var newPath = Path.Combine(directory, newFileName);
        if (File.Exists(newPath))
            return false;

        File.Move(profilePath, newPath);
        return true;
    }

    public bool OverrideProfile(string avatarId, string displayName)
    {
        return SaveProfile(avatarId, displayName);
    }

    public bool MoveProfile(string avatarId, string displayName, bool moveUp)
    {
        if (string.IsNullOrWhiteSpace(avatarId) || string.IsNullOrWhiteSpace(displayName))
        {
            return false;
        }

        var folderPath = Path.Combine(GetProfilesRootPath(), avatarId);
        if (!Directory.Exists(folderPath))
        {
            return false;
        }

        var orderedProfiles = GetOrderedProfileEntries(folderPath);
        var currentIndex = orderedProfiles.FindIndex(profile =>
            string.Equals(profile.DisplayName, displayName, StringComparison.Ordinal));
        if (currentIndex < 0)
        {
            return false;
        }

        var targetIndex = moveUp ? currentIndex - 1 : currentIndex + 1;
        if (targetIndex < 0 || targetIndex >= orderedProfiles.Count)
        {
            return false;
        }

        (orderedProfiles[currentIndex], orderedProfiles[targetIndex]) =
            (orderedProfiles[targetIndex], orderedProfiles[currentIndex]);

        ReindexProfiles(folderPath, orderedProfiles);
        return true;
    }

    private sealed class ProfileFileEntry
    {
        public string Path;
        public string DisplayName;
        public bool HasIndex;
        public int Index;
        public string FileName;
    }

    private static List<ProfileFileEntry> GetOrderedProfileEntries(string folderPath)
    {
        return Directory.GetFiles(folderPath, "*")
            .Where(path => !string.Equals(Path.GetFileName(path), NameFile, StringComparison.Ordinal))
            .Select(path =>
            {
                var fileName = Path.GetFileName(path);
                var (hasIndex, index, displayName) = ParseProfileFileName(fileName);
                return new ProfileFileEntry
                {
                    Path = path,
                    DisplayName = displayName,
                    HasIndex = hasIndex,
                    Index = index,
                    FileName = fileName
                };
            })
            .OrderBy(entry => entry.HasIndex ? 0 : 1)
            .ThenBy(entry => entry.Index)
            .ThenBy(entry => entry.DisplayName, StringComparer.OrdinalIgnoreCase)
            .ThenBy(entry => entry.FileName, StringComparer.OrdinalIgnoreCase)
            .ToList();
    }

    private static void ReindexProfiles(string folderPath, List<ProfileFileEntry> orderedProfiles)
    {
        var tempMoves = new List<(string tempPath, string targetPath)>(orderedProfiles.Count);
        for (var i = 0; i < orderedProfiles.Count; i++)
        {
            var tempPath = Path.Combine(folderPath, $"__reorder_tmp_{Guid.NewGuid():N}");
            File.Move(orderedProfiles[i].Path, tempPath);
            var targetFileName = $"{i + 1}_{orderedProfiles[i].DisplayName}";
            var targetPath = Path.Combine(folderPath, targetFileName);
            tempMoves.Add((tempPath, targetPath));
        }

        foreach (var (tempPath, targetPath) in tempMoves)
        {
            File.Move(tempPath, targetPath);
        }
    }

    private static (bool hasIndex, int index, string displayName) ParseProfileFileName(string fileName)
    {
        if (string.IsNullOrEmpty(fileName))
        {
            return (false, int.MaxValue, fileName ?? string.Empty);
        }

        var underscoreIndex = fileName.IndexOf('_');
        if (underscoreIndex > 0 && int.TryParse(fileName.Substring(0, underscoreIndex), out var index))
        {
            return (true, index, fileName.Substring(underscoreIndex + 1));
        }

        return (false, int.MaxValue, fileName);
    }
}
