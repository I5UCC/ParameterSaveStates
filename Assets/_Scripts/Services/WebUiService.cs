using System;
using System.Collections.Generic;
using System.IO;
using System.Net;
using System.Text;
using System.Threading;
using System.Threading.Tasks;
using Newtonsoft.Json;
using Newtonsoft.Json.Linq;
using PimDeWitte.UnityMainThreadDispatcher;
using UnityEngine;

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

    private const string ProfilesRootFolderName = "Profiles";
    private const string WebUiSettingsFileName = "WebUiSettings.json";
    private const string DefaultTheme = "dark";

    private readonly HttpListener _listener = new HttpListener();
    private readonly CancellationTokenSource _cts = new CancellationTokenSource();
    private Thread _listenerThread;
    private bool _started;

    public int Port => _port;
    public string BaseUrl => $"http://127.0.0.1:{_port}/";
    public bool IsRunning => _started;

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

            HandleContext(context);
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
            catch
            {
                // Ignore close failures
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

            if (method == "POST" && path.Equals("/api/profiles/copy-from-previous", StringComparison.OrdinalIgnoreCase))
            {
                WriteOk(context.Response, RunOnMainThread(CopyFromPrevious));
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

            if (method == "POST" && path.Equals("/api/avatar/name", StringComparison.OrdinalIgnoreCase))
            {
                var body = ReadJsonBody(request);
                var name = body.Value<string>("name");
                WriteOk(context.Response, RunOnMainThread(() => SetAvatarName(name)));
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
            baseUrl = BaseUrl
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

    private object CopyFromPrevious()
    {
        var currentAvatar = _getCurrentAvatar?.Invoke();
        var previousAvatar = _getPreviousAvatar?.Invoke();
        if (string.IsNullOrWhiteSpace(currentAvatar))
        {
            throw new InvalidOperationException("No current avatar detected");
        }

        if (string.IsNullOrWhiteSpace(previousAvatar))
        {
            throw new InvalidOperationException("No previous avatar detected");
        }

        _profileService.CopyProfilesFromAvatar(previousAvatar, currentAvatar);
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

    private object SetAvatarName(string name)
    {
        var currentAvatar = _getCurrentAvatar?.Invoke();
        if (string.IsNullOrWhiteSpace(currentAvatar))
        {
            throw new InvalidOperationException("No current avatar detected");
        }

        if (string.IsNullOrWhiteSpace(name))
        {
            throw new InvalidOperationException("Avatar name is required");
        }

        _profileService.SaveAvatarName(currentAvatar, name);
        return new { avatarName = name };
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
        var profilesRootPath = Path.Combine(Application.persistentDataPath, ProfilesRootFolderName);
        if (!Directory.Exists(profilesRootPath))
        {
            Directory.CreateDirectory(profilesRootPath);
        }

        return Path.Combine(profilesRootPath, WebUiSettingsFileName);
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
