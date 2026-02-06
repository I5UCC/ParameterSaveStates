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
    private List<string> _availableProfiles = new List<string>();
    private int _currentPage = 0;
    private const int PageSize = 20;

    public List<string> AvailableProfiles => _availableProfiles;
    public int CurrentPage => _currentPage;
    public int TotalPages => _availableProfiles.Count == 0 ? 0 : (int)Math.Ceiling((double)_availableProfiles.Count / PageSize);
    
    private const string NameFile = "Name";

    public ProfileService(OscService oscService)
    {
        _oscService = oscService;
    }

    public void LoadProfiles(string avatarId)
    {
        _availableProfiles.Clear();

        var folderPath = Path.Combine(Application.persistentDataPath, $"Profiles/{avatarId}");
        if (!Directory.Exists(folderPath))
        {
            _currentPage = 0;
            return;
        }

        var files = Directory.GetFiles(folderPath, "*");
        var sortedFiles = files
            .Where(f => Path.GetFileName(f) != NameFile)
            .Select(f => new { Path = f, Index = GetProfileIndex(f) })
            .OrderBy(f => f.Index)
            .Select(f => f.Path)
            .ToList();

        _availableProfiles.AddRange(sortedFiles);

        var totalPages = TotalPages;
        if (_currentPage >= totalPages)
        {
            _currentPage = Math.Max(0, totalPages - 1);
        }
    }

    public List<(string path, string displayName)> GetCurrentPageProfiles()
    {
        var result = new List<(string, string)>();
        var startIndex = _currentPage * PageSize;
        var endIndex = Math.Min(startIndex + PageSize, _availableProfiles.Count);

        for (var i = startIndex; i < endIndex; i++)
        {
            var fileName = Path.GetFileName(_availableProfiles[i]);
            var displayName = GetProfileDisplayName(fileName);
            result.Add((_availableProfiles[i], displayName));
        }

        return result;
    }

    public bool NextPage()
    {
        if (_currentPage < TotalPages - 1)
        {
            _currentPage++;
            return true;
        }
        return false;
    }

    public bool PrevPage()
    {
        if (_currentPage > 0)
        {
            _currentPage--;
            return true;
        }
        return false;
    }

    public void ApplyProfile(string displayName)
    {
        var profilePath = _availableProfiles.FirstOrDefault(p => GetProfileDisplayName(Path.GetFileName(p)) == displayName);

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
        var profilePath = _availableProfiles.FirstOrDefault(p => GetProfileDisplayName(Path.GetFileName(p)) == displayName);

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

        var folderPath = Path.Combine(Application.persistentDataPath, $"Profiles/{avatarId}");
        if (!Directory.Exists(folderPath))
        {
            Directory.CreateDirectory(folderPath);
        }

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
        var indexedFileName = $"{nextIndex}_{profileName}";
        var filePath = Path.Combine(folderPath, indexedFileName);
        File.WriteAllText(filePath, json);

        return true;
    }

    public void CopyProfilesFromAvatar(string sourceAvatarId, string targetAvatarId)
    {
        var previousFolderPath = Path.Combine(Application.persistentDataPath, $"Profiles/{sourceAvatarId}");
        var currentFolderPath = Path.Combine(Application.persistentDataPath, $"Profiles/{targetAvatarId}");

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
        var underscoreIndex = fileName.IndexOf('_');
        if (underscoreIndex > 0 && int.TryParse(fileName.Substring(0, underscoreIndex), out var index))
        {
            return index;
        }
        return int.MaxValue;
    }

    private static string GetProfileDisplayName(string fileName)
    {
        var underscoreIndex = fileName.IndexOf('_');
        if (underscoreIndex > 0 && int.TryParse(fileName.Substring(0, underscoreIndex), out _))
        {
            return fileName.Substring(underscoreIndex + 1);
        }
        return fileName;
    }

    private int GetNextProfileIndex()
    {
        if (_availableProfiles.Count == 0) return 1;

        var maxIndex = _availableProfiles
            .Select(GetProfileIndex)
            .Where(i => i != int.MaxValue)
            .DefaultIfEmpty(0)
            .Max();

        return maxIndex + 1;
    }
    
    private string GetAvatarNameFilePath(string avatarId)
    {
        var folderPath = Path.Combine(Application.persistentDataPath, $"Profiles/{avatarId}");
        return Path.Combine(folderPath, NameFile);
    }

    public string LoadAvatarName(string avatarId)
    {
        var filePath = GetAvatarNameFilePath(avatarId);
        if (File.Exists(filePath))
        {
            return File.ReadAllText(filePath).Trim();
        }
        return null;
    }

    public void SaveAvatarName(string avatarId, string avatarName)
    {
        var folderPath = Path.Combine(Application.persistentDataPath, $"Profiles/{avatarId}");
        if (!Directory.Exists(folderPath))
        {
            Directory.CreateDirectory(folderPath);
        }
        var filePath = GetAvatarNameFilePath(avatarId);
        File.WriteAllText(filePath, avatarName);
    }
}
