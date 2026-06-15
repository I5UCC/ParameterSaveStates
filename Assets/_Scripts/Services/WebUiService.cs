using System;
using System.Collections.Generic;
using System.IO;
using System.IO.Compression;
using System.Net;
using System.Text;
using System.Threading;
using System.Threading.Tasks;
using Newtonsoft.Json;
using Newtonsoft.Json.Linq;
using PimDeWitte.UnityMainThreadDispatcher;
using UnityEngine;
using CompressionLevel = System.IO.Compression.CompressionLevel;

public sealed class WebUiService : IDisposable
{
    private sealed class ApiResponse
    {
        public bool ok;
        public object data;
        public string error;
    }

    private readonly ProfileService _profileService;
    private readonly OscService _oscService;
    private readonly UnityMainThreadDispatcher _dispatcher;
    private readonly Func<string> _getCurrentAvatar;
    private readonly Func<string> _getPreviousAvatar;
    private readonly string _indexFilePath;
    private readonly int _port;

    private const string WebUiSettingsFileName = "WebUiSettings.json";
    private const string DefaultTheme = "dark";

    private readonly HttpListener _listener = new HttpListener();
    private readonly CancellationTokenSource _cts = new CancellationTokenSource();
    private Thread _listenerThread;
    private bool _started;

    public int Port => _port;
    public string BaseUrl => $"http://127.0.0.1:{_port}/";
    public bool IsRunning => _started;

    public event Action OnStateChanged;

    public void NotifyStateChanged()
    {
        OnStateChanged?.Invoke();
    }

    public WebUiService(
        ProfileService profileService,
        OscService oscService,
        UnityMainThreadDispatcher dispatcher,
        Func<string> getCurrentAvatar,
        Func<string> getPreviousAvatar,
        int port,
        string indexFilePath)
    {
        _profileService = profileService;
        _oscService = oscService;
        _dispatcher = dispatcher;
        _getCurrentAvatar = getCurrentAvatar;
        _getPreviousAvatar = getPreviousAvatar;
        _port = port;
        _indexFilePath = indexFilePath;
    }

    public void Start()
    {
        if (_started) return;

        try
        {
            _listener.Prefixes.Add($"http://127.0.0.1:{_port}/");
            _listener.Prefixes.Add($"http://localhost:{_port}/");
            _listener.Start();
        }
        catch (Exception ex)
        {
            Debug.LogError($"Failed to start Web UI server on port {_port}: {ex}");
            return;
        }

        _listenerThread = new Thread(ListenLoop)
        {
            IsBackground = true,
            Name = "WebUiService"
        };
        _listenerThread.Start();
        _started = true;
        Debug.Log($"Web UI running at {BaseUrl}");
    }

    public void Dispose()
    {
        Stop();
    }

    public void Stop()
    {
        if (!_started) return;

        _cts.Cancel();
        try
        {
            _listener.Stop();
            _listener.Close();
        }
        catch (Exception ex)
        {
            Debug.LogWarning($"Failed to stop Web UI server cleanly: {ex}");
        }

        try { _cts.Dispose(); } catch { /* ignore */ }
        _started = false;
    }

    private void ListenLoop()
    {
        while (!_cts.IsCancellationRequested)
        {
            HttpListenerContext context;
            try
            {
                context = _listener.GetContext();
            }
            catch (HttpListenerException)
            {
                if (_cts.IsCancellationRequested) break;
                continue;
            }
            catch (ObjectDisposedException)
            {
                break;
            }

            ThreadPool.QueueUserWorkItem(_ => HandleContext(context));
        }
    }

    private void HandleContext(HttpListenerContext context)
    {
        try
        {
            var request = context.Request;
            var path = request.Url?.AbsolutePath ?? "/";

            if (path == "/" || path.Equals("/index.html", StringComparison.OrdinalIgnoreCase))
            {
                ServeIndex(context.Response);
                return;
            }

            if (path.Equals("/api/events", StringComparison.OrdinalIgnoreCase))
            {
                HandleEvents(context);
                return;
            }

            if (path.StartsWith("/api/", StringComparison.OrdinalIgnoreCase))
            {
                HandleApi(context);
                return;
            }

            WriteJson(context.Response, new ApiResponse
            {
                ok = false,
                error = "Not found"
            }, 404);
        }
        catch (Exception ex)
        {
            Debug.LogError($"Web UI request failed: {ex}");
            WriteJson(context.Response, new ApiResponse
            {
                ok = false,
                error = "Internal server error"
            }, 500);
        }
        finally
        {
            try
            {
                context.Response.OutputStream.Close();
            }
            catch (Exception ex)
            {
                Debug.Log($"Web UI response stream close failed (usually harmless): {ex.Message}");
            }
        }
    }

    private void ServeIndex(HttpListenerResponse response)
    {
        byte[] bytes;
        if (File.Exists(_indexFilePath))
        {
            bytes = File.ReadAllBytes(_indexFilePath);
        }
        else
        {
            bytes = Encoding.UTF8.GetBytes(FallbackIndexHtml);
        }

        WriteBytes(response, bytes, "text/html; charset=utf-8", 200);
    }

    private void HandleEvents(HttpListenerContext context)
    {
        context.Response.ContentType = "text/event-stream";
        context.Response.Headers.Add("Cache-Control", "no-cache");
        context.Response.Headers.Add("Connection", "keep-alive");

        Action handler = null;
        try
        {
            using var writer = new StreamWriter(context.Response.OutputStream, new UTF8Encoding(false)) { AutoFlush = true };
            writer.Write(":\n\n");

            var mre = new ManualResetEventSlim(false);
            handler = () => mre.Set();
            OnStateChanged += handler;

            while (!_cts.IsCancellationRequested)
            {
                if (mre.Wait(15000, _cts.Token))
                {
                    writer.Write("data: update\n\n");
                    mre.Reset();
                }
                else
                {
                    writer.Write(":\n\n");
                }
            }
        }
        catch (Exception ex)
        {
            // Client disconnected or server stopping — log unexpected errors
            if (!_cts.IsCancellationRequested)
                Debug.Log($"Web UI event stream ended: {ex.Message}");
        }
        finally
        {
            if (handler != null)
            {
                OnStateChanged -= handler;
            }
        }
    }

    private void HandleApi(HttpListenerContext context)
    {
        try
        {
            var request = context.Request;
            var path = request.Url?.AbsolutePath ?? string.Empty;
            var method = request.HttpMethod;

            if (method == "GET" && path.Equals("/api/status", StringComparison.OrdinalIgnoreCase))
            {
                WriteOk(context.Response, RunOnMainThread(BuildStatus));
                return;
            }

            if (method == "GET" && path.Equals("/api/profiles", StringComparison.OrdinalIgnoreCase))
            {
                WriteOk(context.Response, RunOnMainThread(ListProfiles));
                return;
            }

            if (method == "POST" && path.Equals("/api/profiles/save", StringComparison.OrdinalIgnoreCase))
            {
                var body = ReadJsonBody(request);
                var name = body.Value<string>("name") ?? string.Empty;
                WriteOk(context.Response, RunOnMainThread(() => SaveProfile(name)));
                return;
            }

            if (method == "POST" && path.Equals("/api/profiles/apply", StringComparison.OrdinalIgnoreCase))
            {
                var body = ReadJsonBody(request);
                var name = body.Value<string>("name");
                WriteOk(context.Response, RunOnMainThread(() => ApplyProfile(name)));
                return;
            }

            if (method == "POST" && path.Equals("/api/profiles/override", StringComparison.OrdinalIgnoreCase))
            {
                var body = ReadJsonBody(request);
                var name = body.Value<string>("name");
                WriteOk(context.Response, RunOnMainThread(() => OverrideProfile(name)));
                return;
            }

            if (method == "POST" && path.Equals("/api/profiles/delete", StringComparison.OrdinalIgnoreCase))
            {
                var body = ReadJsonBody(request);
                var name = body.Value<string>("name");
                WriteOk(context.Response, RunOnMainThread(() => DeleteProfile(name)));
                return;
            }

            if (method == "POST" && path.Equals("/api/profiles/rename", StringComparison.OrdinalIgnoreCase))
            {
                var body = ReadJsonBody(request);
                var name = body.Value<string>("name");
                var newName = body.Value<string>("newName");
                WriteOk(context.Response, RunOnMainThread(() => RenameProfile(name, newName)));
                return;
            }

            if (method == "POST" && path.Equals("/api/profiles/move", StringComparison.OrdinalIgnoreCase))
            {
                var body = ReadJsonBody(request);
                var name = body.Value<string>("name");
                var direction = body.Value<string>("direction");
                WriteOk(context.Response, RunOnMainThread(() => MoveProfile(name, direction)));
                return;
            }

            if (method == "GET" && path.Equals("/api/avatars/with-profiles", StringComparison.OrdinalIgnoreCase))
            {
                WriteOk(context.Response, RunOnMainThread(ListAvatarsWithSavedProfiles));
                return;
            }

            if (method == "POST" && path.Equals("/api/profiles/copy-from-avatar", StringComparison.OrdinalIgnoreCase))
            {
                var body = ReadJsonBody(request);
                var avatarId = body.Value<string>("avatarId");
                WriteOk(context.Response, RunOnMainThread(() => CopyFromAvatar(avatarId)));
                return;
            }

            if (method == "POST" && path.Equals("/api/profiles/delete-avatar-folder", StringComparison.OrdinalIgnoreCase))
            {
                var body = ReadJsonBody(request);
                var avatarId = body.Value<string>("avatarId");
                WriteOk(context.Response, RunOnMainThread(() => DeleteAvatarFolder(avatarId)));
                return;
            }

            if (method == "GET" && path.Equals("/api/profiles/export", StringComparison.OrdinalIgnoreCase))
            {
                var archive = RunOnMainThread(ExportProfilesArchive);
                var fileName = $"profiles-export-{DateTime.UtcNow:yyyyMMdd-HHmmss}.zip";
                WriteFile(context.Response, archive, fileName, "application/zip", 200);
                return;
            }

            if (method == "POST" && path.Equals("/api/profiles/import", StringComparison.OrdinalIgnoreCase))
            {
                var archiveBytes = ReadBinaryBody(request);
                WriteOk(context.Response, RunOnMainThread(() => ImportProfilesArchive(archiveBytes)));
                return;
            }

            if (method == "GET" && path.Equals("/api/theme", StringComparison.OrdinalIgnoreCase))
            {
                WriteOk(context.Response, new { theme = LoadTheme() });
                return;
            }

            if (method == "POST" && path.Equals("/api/theme", StringComparison.OrdinalIgnoreCase))
            {
                var body = ReadJsonBody(request);
                var theme = body.Value<string>("theme");
                WriteOk(context.Response, new { theme = SaveTheme(theme) });
                return;
            }

            if (method == "GET" && path.Equals("/api/profile-apply-filters", StringComparison.OrdinalIgnoreCase))
            {
                WriteOk(context.Response, RunOnMainThread(GetProfileApplyFilters));
                return;
            }

            if (method == "POST" && path.Equals("/api/profile-apply-filters", StringComparison.OrdinalIgnoreCase))
            {
                var body = ReadJsonBody(request);
                var parameterNames = body["excludedParameterNames"]?.ToObject<List<string>>() ?? new List<string>();
                WriteOk(context.Response, RunOnMainThread(() => SaveProfileApplyFilters(parameterNames)));
                return;
            }

            if (method == "GET" && path.Equals("/api/avatar-parameter-auto-sync", StringComparison.OrdinalIgnoreCase))
            {
                var avatarId = GetQueryParam(request, "avatarId");
                if (string.IsNullOrWhiteSpace(avatarId))
                    throw new InvalidOperationException("avatarId is required");
                WriteOk(context.Response, RunOnMainThread(() => GetAvatarAutoSync(avatarId)));
                return;
            }

            if (method == "POST" && path.Equals("/api/avatar-parameter-auto-sync", StringComparison.OrdinalIgnoreCase))
            {
                var body = ReadJsonBody(request);
                var avatarId = body.Value<string>("avatarId");
                var parameterNames = body["parameterNames"]?.ToObject<List<string>>() ?? new List<string>();
                WriteOk(context.Response, RunOnMainThread(() => SaveAvatarAutoSync(avatarId, parameterNames)));
                return;
            }

            if (method == "GET" && path.Equals("/api/avatar-profile-apply-filters", StringComparison.OrdinalIgnoreCase))
            {
                var avatarId = GetQueryParam(request, "avatarId");
                if (string.IsNullOrWhiteSpace(avatarId))
                    throw new InvalidOperationException("avatarId is required");
                WriteOk(context.Response, RunOnMainThread(() => GetAvatarProfileFilters(avatarId)));
                return;
            }

            if (method == "POST" && path.Equals("/api/avatar-profile-apply-filters", StringComparison.OrdinalIgnoreCase))
            {
                var body = ReadJsonBody(request);
                var avatarId = body.Value<string>("avatarId");
                var parameterNames = body["parameterNames"]?.ToObject<List<string>>() ?? new List<string>();
                WriteOk(context.Response, RunOnMainThread(() => SaveAvatarProfileFilters(avatarId, parameterNames)));
                return;
            }

            WriteJson(context.Response, new ApiResponse
            {
                ok = false,
                error = "Not found"
            }, 404);
        }
        catch (Exception ex)
        {
            var statusCode = ex is InvalidOperationException ? 400 : 500;
            Debug.LogWarning($"Web UI API error: {ex.Message}");
            WriteJson(context.Response, new ApiResponse
            {
                ok = false,
                error = ex.Message
            }, statusCode);
        }
    }

    private object BuildStatus()
    {
        var currentAvatar = _getCurrentAvatar?.Invoke();
        var previousAvatar = _getPreviousAvatar?.Invoke();
        var connected = _oscService.QueryServiceProfile != null;
        var avatarName = string.IsNullOrWhiteSpace(currentAvatar) ? null : _profileService.LoadAvatarName(currentAvatar);
        var previousAvatarName = string.IsNullOrWhiteSpace(previousAvatar) ? null : _profileService.LoadAvatarName(previousAvatar);
        var profiles = string.IsNullOrWhiteSpace(currentAvatar) ? new List<string>() : GetProfileNames(currentAvatar);

        return new
        {
            connected,
            currentAvatar,
            previousAvatar,
            avatarName,
            previousAvatarName,
            profiles,
            baseUrl = BaseUrl,
            version = AppConstants.CurrentVersion
        };
    }

    private object ListProfiles()
    {
        var currentAvatar = _getCurrentAvatar?.Invoke();
        if (string.IsNullOrWhiteSpace(currentAvatar))
        {
            return new { profiles = new List<string>() };
        }

        return new { profiles = GetProfileNames(currentAvatar) };
    }

    private object GetProfileApplyFilters()
    {
        return new
        {
            excludedParameterNames = _profileService.GetExcludedParameterNames()
        };
    }

    private object SaveProfileApplyFilters(List<string> parameterNames)
    {
        var savedNames = _profileService.SaveExcludedParameterNames(parameterNames ?? new List<string>());
        return new
        {
            excludedParameterNames = savedNames
        };
    }

    private object GetAvatarAutoSync(string avatarId)
    {
        return new { avatarId, parameterNames = _profileService.GetAvatarAutoSyncParameterNames(avatarId) };
    }

    private object SaveAvatarAutoSync(string avatarId, List<string> parameterNames)
    {
        if (string.IsNullOrWhiteSpace(avatarId))
            throw new InvalidOperationException("avatarId is required");
        var saved = _profileService.SaveAvatarAutoSyncParameterNames(avatarId, parameterNames ?? new List<string>());
        return new { avatarId, parameterNames = saved };
    }

    private object GetAvatarProfileFilters(string avatarId)
    {
        return new { avatarId, parameterNames = _profileService.GetAvatarExcludedParameterNames(avatarId) };
    }

    private object SaveAvatarProfileFilters(string avatarId, List<string> parameterNames)
    {
        if (string.IsNullOrWhiteSpace(avatarId))
            throw new InvalidOperationException("avatarId is required");
        var saved = _profileService.SaveAvatarExcludedParameterNames(avatarId, parameterNames ?? new List<string>());
        return new { avatarId, parameterNames = saved };
    }

    private object SaveProfile(string name)
    {
        var currentAvatar = _getCurrentAvatar?.Invoke();
        if (string.IsNullOrWhiteSpace(currentAvatar))
        {
            throw new InvalidOperationException("No current avatar detected");
        }

        _profileService.LoadProfiles(currentAvatar);
        var saved = _profileService.SaveProfile(currentAvatar, name ?? string.Empty);
        if (!saved)
        {
            throw new InvalidOperationException("Unable to save profile. Is VRChat connected?");
        }

        return new { profiles = GetProfileNames(currentAvatar) };
    }

    private object ApplyProfile(string name)
    {
        var currentAvatar = _getCurrentAvatar?.Invoke();
        if (string.IsNullOrWhiteSpace(currentAvatar))
        {
            throw new InvalidOperationException("No current avatar detected");
        }

        if (string.IsNullOrWhiteSpace(name))
        {
            throw new InvalidOperationException("Profile name is required");
        }

        _profileService.LoadProfiles(currentAvatar);
        var profiles = _profileService.GetAllProfileDisplayNames();
        if (!profiles.Contains(name))
        {
            throw new InvalidOperationException("Profile not found");
        }

        _profileService.ApplyProfile(name);
        return new { profiles };
    }

    private object OverrideProfile(string name)
    {
        var currentAvatar = _getCurrentAvatar?.Invoke();
        if (string.IsNullOrWhiteSpace(currentAvatar))
        {
            throw new InvalidOperationException("No current avatar detected");
        }

        if (string.IsNullOrWhiteSpace(name))
        {
            throw new InvalidOperationException("Profile name is required");
        }

        _profileService.LoadProfiles(currentAvatar);
        var success = _profileService.OverrideProfile(currentAvatar, name);
        if (!success)
        {
            throw new InvalidOperationException("Unable to override profile. Is VRChat connected?");
        }

        return new { profiles = GetProfileNames(currentAvatar) };
    }

    private object DeleteProfile(string name)
    {
        var currentAvatar = _getCurrentAvatar?.Invoke();
        if (string.IsNullOrWhiteSpace(currentAvatar))
        {
            throw new InvalidOperationException("No current avatar detected");
        }

        if (string.IsNullOrWhiteSpace(name))
        {
            throw new InvalidOperationException("Profile name is required");
        }

        _profileService.LoadProfiles(currentAvatar);
        var success = _profileService.DeleteProfile(name);
        if (!success)
        {
            throw new InvalidOperationException("Profile not found");
        }

        return new { profiles = GetProfileNames(currentAvatar) };
    }

    private object RenameProfile(string name, string newName)
    {
        var currentAvatar = _getCurrentAvatar?.Invoke();
        if (string.IsNullOrWhiteSpace(currentAvatar))
        {
            throw new InvalidOperationException("No current avatar detected");
        }

        if (string.IsNullOrWhiteSpace(name) || string.IsNullOrWhiteSpace(newName))
        {
            throw new InvalidOperationException("Both current and new profile names are required");
        }

        _profileService.LoadProfiles(currentAvatar);
        var success = _profileService.RenameProfile(name, newName);
        if (!success)
        {
            throw new InvalidOperationException("Profile not found");
        }

        return new { profiles = GetProfileNames(currentAvatar) };
    }

    private object MoveProfile(string name, string direction)
    {
        var currentAvatar = _getCurrentAvatar?.Invoke();
        if (string.IsNullOrWhiteSpace(currentAvatar))
        {
            throw new InvalidOperationException("No current avatar detected");
        }

        if (string.IsNullOrWhiteSpace(name))
        {
            throw new InvalidOperationException("Profile name is required");
        }

        var moveUp = direction switch
        {
            "up" => true,
            "down" => false,
            _ => throw new InvalidOperationException("Direction must be 'up' or 'down'")
        };

        _profileService.LoadProfiles(currentAvatar);
        var success = _profileService.MoveProfile(currentAvatar, name, moveUp);
        if (!success)
        {
            throw new InvalidOperationException("Unable to move profile");
        }

        return new { profiles = GetProfileNames(currentAvatar) };
    }

    private object ListAvatarsWithSavedProfiles()
    {
        var avatars = _profileService.GetAvatarsWithSavedProfiles();
        return new
        {
            avatars = avatars.ConvertAll(avatar => new
            {
                avatarId = avatar.avatarId,
                avatarName = avatar.avatarName
            })
        };
    }

    private object CopyFromAvatar(string sourceAvatarId)
    {
        var currentAvatar = _getCurrentAvatar?.Invoke();
        if (string.IsNullOrWhiteSpace(currentAvatar))
        {
            throw new InvalidOperationException("No current avatar detected");
        }

        if (string.IsNullOrWhiteSpace(sourceAvatarId))
        {
            throw new InvalidOperationException("Source avatar is required");
        }

        var avatarsWithProfiles = _profileService.GetAvatarsWithSavedProfiles();
        var hasProfiles = avatarsWithProfiles.Exists(avatar => avatar.avatarId == sourceAvatarId);
        if (!hasProfiles)
        {
            throw new InvalidOperationException("Selected avatar has no saved profiles");
        }

        _profileService.CopyProfilesFromAvatar(sourceAvatarId, currentAvatar);
        return new { profiles = GetProfileNames(currentAvatar) };
    }

    private object DeleteAvatarFolder(string avatarId)
    {
        if (string.IsNullOrWhiteSpace(avatarId))
        {
            throw new InvalidOperationException("Avatar is required");
        }

        var avatarsWithProfiles = _profileService.GetAvatarsWithSavedProfiles();
        var hasProfiles = avatarsWithProfiles.Exists(avatar => avatar.avatarId == avatarId);
        if (!hasProfiles)
        {
            throw new InvalidOperationException("Selected avatar has no saved profiles");
        }

        var success = _profileService.DeleteAvatarProfiles(avatarId);
        if (!success)
        {
            throw new InvalidOperationException("Avatar profiles folder not found");
        }

        var currentAvatar = _getCurrentAvatar?.Invoke();
        return new
        {
            profiles = string.IsNullOrWhiteSpace(currentAvatar) ? new List<string>() : GetProfileNames(currentAvatar)
        };
    }

    private byte[] ExportProfilesArchive()
    {
        var profilesRootPath = Path.Combine(Application.persistentDataPath, AppConstants.ProfilesRootFolderName);
        if (!Directory.Exists(profilesRootPath))
        {
            throw new InvalidOperationException("Profiles folder not found");
        }

        var files = Directory.GetFiles(profilesRootPath, "*", SearchOption.AllDirectories);
        if (files.Length == 0)
        {
            throw new InvalidOperationException("Profiles folder is empty");
        }

        using var stream = new MemoryStream();
        using (var archive = new ZipArchive(stream, ZipArchiveMode.Create, true))
        {
            foreach (var filePath in files)
            {
                var relativePath = GetRelativePath(profilesRootPath, filePath)
                    .Replace(Path.DirectorySeparatorChar, '/');
                archive.CreateEntryFromFile(filePath, relativePath, CompressionLevel.Optimal);
            }
        }

        return stream.ToArray();
    }

    private object ImportProfilesArchive(byte[] archiveBytes)
    {
        if (archiveBytes == null || archiveBytes.Length == 0)
        {
            throw new InvalidOperationException("Archive body is empty");
        }

        var profilesRootPath = Path.Combine(Application.persistentDataPath, AppConstants.ProfilesRootFolderName);
        if (!Directory.Exists(profilesRootPath))
        {
            Directory.CreateDirectory(profilesRootPath);
        }

        var tempRoot = Path.Combine(Application.temporaryCachePath, $"profiles-import-{Guid.NewGuid():N}");
        Directory.CreateDirectory(tempRoot);

        try
        {
            using (var stream = new MemoryStream(archiveBytes, false))
            using (var archive = new ZipArchive(stream, ZipArchiveMode.Read, false))
            {
                if (archive.Entries.Count == 0)
                {
                    throw new InvalidOperationException("Archive has no files");
                }

                foreach (var entry in archive.Entries)
                {
                    if (string.IsNullOrEmpty(entry.Name))
                    {
                        continue;
                    }

                    var normalizedEntryPath = entry.FullName.Replace('\\', '/');
                    var destinationPath = Path.GetFullPath(Path.Combine(tempRoot, normalizedEntryPath));
                    if (!destinationPath.StartsWith(tempRoot, StringComparison.OrdinalIgnoreCase))
                    {
                        throw new InvalidOperationException("Archive contains invalid file path");
                    }

                    var destinationDir = Path.GetDirectoryName(destinationPath);
                    if (!string.IsNullOrEmpty(destinationDir) && !Directory.Exists(destinationDir))
                    {
                        Directory.CreateDirectory(destinationDir);
                    }

                    using var sourceStream = entry.Open();
                    using var destinationStream = File.Create(destinationPath);
                    sourceStream.CopyTo(destinationStream);
                }
            }

            var extractedFiles = Directory.GetFiles(tempRoot, "*", SearchOption.AllDirectories);
            foreach (var sourcePath in extractedFiles)
            {
                var relativePath = GetRelativePath(tempRoot, sourcePath);
                var targetPath = Path.Combine(profilesRootPath, relativePath);
                var targetDir = Path.GetDirectoryName(targetPath);
                if (!string.IsNullOrEmpty(targetDir) && !Directory.Exists(targetDir))
                {
                    Directory.CreateDirectory(targetDir);
                }

                File.Copy(sourcePath, targetPath, true);
            }

            NotifyStateChanged();
            return new
            {
                importedFiles = extractedFiles.Length
            };
        }
        catch (InvalidDataException)
        {
            throw new InvalidOperationException("Invalid zip archive");
        }
        finally
        {
            try
            {
                if (Directory.Exists(tempRoot))
                {
                    Directory.Delete(tempRoot, true);
                }
            }
            catch (Exception ex)
            {
                Debug.LogWarning($"Failed to clean temporary import folder: {ex.Message}");
            }
        }
    }

    private string LoadTheme()
    {
        var settingsPath = GetWebUiSettingsFilePath();
        if (!File.Exists(settingsPath))
        {
            return DefaultTheme;
        }

        try
        {
            var json = File.ReadAllText(settingsPath);
            if (string.IsNullOrWhiteSpace(json))
            {
                return DefaultTheme;
            }

            var settings = JObject.Parse(json);
            var theme = settings.Value<string>("theme");
            if (string.Equals(theme, "light", StringComparison.OrdinalIgnoreCase))
            {
                return "light";
            }

            if (string.Equals(theme, "dark", StringComparison.OrdinalIgnoreCase))
            {
                return "dark";
            }

            Debug.LogWarning("Web UI theme setting is invalid. Falling back to dark.");
            return DefaultTheme;
        }
        catch (Exception ex)
        {
            Debug.LogWarning($"Failed to load Web UI theme setting: {ex.Message}");
            return DefaultTheme;
        }
    }

    private string SaveTheme(string theme)
    {
        if (string.IsNullOrWhiteSpace(theme))
        {
            throw new InvalidOperationException("Theme is required");
        }

        var normalized = theme.Trim().ToLowerInvariant();
        if (normalized != "light" && normalized != "dark")
        {
            throw new InvalidOperationException("Theme must be 'light' or 'dark'");
        }

        var settings = new JObject
        {
            ["theme"] = normalized
        };

        var settingsPath = GetWebUiSettingsFilePath();
        File.WriteAllText(settingsPath, settings.ToString(Formatting.Indented));
        return normalized;
    }

    private static string GetWebUiSettingsFilePath()
    {
        var profilesRootPath = Path.Combine(Application.persistentDataPath, AppConstants.ProfilesRootFolderName);
        if (!Directory.Exists(profilesRootPath))
        {
            Directory.CreateDirectory(profilesRootPath);
        }

        return Path.Combine(profilesRootPath, WebUiSettingsFileName);
    }

    private static string GetQueryParam(HttpListenerRequest request, string paramName)
    {
        var query = request.Url?.Query;
        if (string.IsNullOrEmpty(query)) return null;
        query = query.TrimStart('?');
        foreach (var part in query.Split('&'))
        {
            var kv = part.Split(new[] { '=' }, 2);
            if (kv.Length == 2 && string.Equals(Uri.UnescapeDataString(kv[0]), paramName, StringComparison.OrdinalIgnoreCase))
                return Uri.UnescapeDataString(kv[1]);
        }
        return null;
    }

    private List<string> GetProfileNames(string avatarId)
    {
        _profileService.LoadProfiles(avatarId);
        return _profileService.GetAllProfileDisplayNames();
    }

    private T RunOnMainThread<T>(Func<T> func)
    {
        var tcs = new TaskCompletionSource<T>();
        _dispatcher.Enqueue(() =>
        {
            try
            {
                tcs.SetResult(func());
            }
            catch (Exception ex)
            {
                tcs.SetException(ex);
            }
        });
        return tcs.Task.GetAwaiter().GetResult();
    }

    private static JObject ReadJsonBody(HttpListenerRequest request)
    {
        if (!request.HasEntityBody) return new JObject();

        using var reader = new StreamReader(request.InputStream, request.ContentEncoding ?? Encoding.UTF8);
        var body = reader.ReadToEnd();
        if (string.IsNullOrWhiteSpace(body)) return new JObject();

        return JObject.Parse(body);
    }

    private static byte[] ReadBinaryBody(HttpListenerRequest request)
    {
        if (!request.HasEntityBody)
        {
            return Array.Empty<byte>();
        }

        using var stream = new MemoryStream();
        request.InputStream.CopyTo(stream);
        return stream.ToArray();
    }

    private static string GetRelativePath(string rootPath, string fullPath)
    {
        var normalizedRoot = Path.GetFullPath(rootPath)
            .TrimEnd(Path.DirectorySeparatorChar, Path.AltDirectorySeparatorChar)
            + Path.DirectorySeparatorChar;
        var normalizedFull = Path.GetFullPath(fullPath);
        if (!normalizedFull.StartsWith(normalizedRoot, StringComparison.OrdinalIgnoreCase))
        {
            throw new InvalidOperationException("Path is outside expected root");
        }

        return normalizedFull.Substring(normalizedRoot.Length);
    }

    private void WriteOk(HttpListenerResponse response, object data)
    {
        WriteJson(response, new ApiResponse
        {
            ok = true,
            data = data
        }, 200);
    }

    private static void WriteJson(HttpListenerResponse response, ApiResponse payload, int statusCode)
    {
        var json = JsonConvert.SerializeObject(payload);
        var bytes = Encoding.UTF8.GetBytes(json);
        WriteBytes(response, bytes, "application/json; charset=utf-8", statusCode);
    }

    private static void WriteBytes(HttpListenerResponse response, byte[] bytes, string contentType, int statusCode)
    {
        response.StatusCode = statusCode;
        response.ContentType = contentType;
        response.ContentLength64 = bytes.Length;
        response.OutputStream.Write(bytes, 0, bytes.Length);
    }

    private static void WriteFile(HttpListenerResponse response, byte[] bytes, string fileName, string contentType, int statusCode)
    {
        response.Headers["Content-Disposition"] = $"attachment; filename=\"{fileName}\"";
        WriteBytes(response, bytes, contentType, statusCode);
    }

    private const string FallbackIndexHtml = @"<!doctype html>
<html lang=""en"">
<head>
  <meta charset=""utf-8"" />
  <title>Parameter Save States</title>
</head>
<body>
  <h1>Web UI file missing</h1>
  <p>StreamingAssets/WebUi/index.html was not found.</p>
</body>
</html>";
}
