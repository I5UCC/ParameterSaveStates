using System;
using System.Runtime.InteropServices;
using System.IO;
using System.Drawing;
using System.Text;
using System.Windows.Forms;
using UnityEngine;

public class WindowController : MonoBehaviour
{
    public bool hasTrayIcon = true;
    public Texture2D trayIconTex;

    private const string ProfilesRootFolderName = "Profiles";
    private const string WebUiWindowSizeFileName = "WebUiWindowSize";
    private const string WebUiWindowTitle = "Parameter Save States";
    private const int DefaultWebUiWindowWidth = 1200;
    private const int DefaultWebUiWindowHeight = 500;
    private const int MinWebUiWindowWidth = 640;
    private const int MinWebUiWindowHeight = 480;
    private const float WebUiWindowSizeSampleInterval = 1f;
    private const float WebUiWindowApplyTimeout = 5f;
    private const int WebUiWindowSizeTolerance = 4;
    private const float WebUiWindowHandleSearchInterval = 0.5f;

    private const int GWL_EXSTYLE = -0x14;
    private const int WS_EX_TOOLWINDOW = 0x0080;
    private const int SWP_HIDEWINDOW = 0x0080;
    private const uint SW_RESTORE = 9;
    private const int SWP_NOMOVE = 0x0002;
    private const int SWP_NOZORDER = 0x0004;
    private const int SWP_NOACTIVATE = 0x0010;
    private const uint WM_CLOSE = 0x0010;

    private IntPtr winHandle;

    private TrayForm trayForm;
    private System.Diagnostics.Process webUiWindowProcess;
    private IntPtr webUiWindowHandle;
    private int pendingWebUiWindowWidth;
    private int pendingWebUiWindowHeight;
    private bool hasPendingWebUiWindowSize;
    private float pendingWebUiWindowSizeUntilTime;
    private float nextWebUiWindowSizeSampleTime;
    private float nextWebUiWindowHandleSearchTime;
    private int lastSavedWebUiWindowWidth;
    private int lastSavedWebUiWindowHeight;
    private bool hasLastSavedWebUiWindowSize;
    private bool quitRequestedFromTray;


#if UNITY_STANDALONE_WIN && !UNITY_EDITOR

    [DllImport("user32.dll", EntryPoint = "FindWindow", SetLastError = true)] static extern IntPtr FindWindowByCaption(IntPtr zeroOnly, string lpWindowName);
    [DllImport("user32.dll", EntryPoint = "SetWindowPos")] private static extern bool SetWindowPos(IntPtr hwnd, int hWndInsertAfter, int x, int Y, int cx, int cy, int wFlags);
    [DllImport("user32.dll")] [return: MarshalAs(UnmanagedType.Bool)] private static extern bool ShowWindow(IntPtr hWnd, uint nCmdShow);
    [DllImport("user32.dll")] static extern int SetWindowLong(IntPtr hWnd, int nIndex, int dwNewLong);
    [DllImport("user32.dll", SetLastError = true)] static extern int GetWindowLong(IntPtr hWnd, int nIndex);
    [DllImport("user32.dll")] [return: MarshalAs(UnmanagedType.Bool)] private static extern bool SetForegroundWindow(IntPtr hWnd);
    [DllImport("user32.dll")] [return: MarshalAs(UnmanagedType.Bool)] private static extern bool GetWindowRect(IntPtr hWnd, out WinRect lpRect);
    [DllImport("user32.dll")] [return: MarshalAs(UnmanagedType.Bool)] private static extern bool PostMessage(IntPtr hWnd, uint Msg, IntPtr wParam, IntPtr lParam);
    private delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);
    [DllImport("user32.dll")] [return: MarshalAs(UnmanagedType.Bool)] private static extern bool EnumWindows(EnumWindowsProc lpEnumFunc, IntPtr lParam);
    [DllImport("user32.dll")] private static extern int GetWindowText(IntPtr hWnd, StringBuilder lpString, int nMaxCount);
    [DllImport("user32.dll")] private static extern int GetWindowTextLength(IntPtr hWnd);
    [DllImport("user32.dll")] [return: MarshalAs(UnmanagedType.Bool)] private static extern bool IsWindowVisible(IntPtr hWnd);

#else

    static IntPtr FindWindowByCaption(IntPtr zeroOnly, string lpWindowName) { return IntPtr.Zero; }
    static int SetWindowLong(IntPtr hWnd, int nIndex, int dwNewLong) { return 0; }
    static int GetWindowLong(IntPtr hWnd, int nIndex) { return 0; }
    private static bool SetWindowPos(IntPtr hwnd, int hWndInsertAfter, int x, int Y, int cx, int cy, int wFlags) { return true; }
    private static bool ShowWindow(IntPtr hWnd, uint nCmdShow) { return true; }
    private static bool SetForegroundWindow(IntPtr hWnd) { return true; }
    private static bool GetWindowRect(IntPtr hWnd, out WinRect lpRect) { lpRect = default; return false; }
    private static bool PostMessage(IntPtr hWnd, uint Msg, IntPtr wParam, IntPtr lParam) { return false; }
    private delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);
    private static bool EnumWindows(EnumWindowsProc lpEnumFunc, IntPtr lParam) { return false; }
    private static int GetWindowText(IntPtr hWnd, StringBuilder lpString, int nMaxCount) { return 0; }
    private static int GetWindowTextLength(IntPtr hWnd) { return 0; }
    private static bool IsWindowVisible(IntPtr hWnd) { return false; }

#endif
    [StructLayout(LayoutKind.Sequential)]
    private struct WinRect
    {
        public int Left;
        public int Top;
        public int Right;
        public int Bottom;
    }

    void Start()
    {
        winHandle = FindWindowByCaption(IntPtr.Zero, UnityEngine.Application.productName);

        if (hasTrayIcon)
            CreateTray();

        HideUnityWindow();
        ShowTrayIcon();
    }

    void OnEnable()
    {
        if (hasTrayIcon)
            CreateTray();
    }

    void Update()
    {
        if (quitRequestedFromTray)
        {
            quitRequestedFromTray = false;
            DestroyTray();
            UnityEngine.Application.Quit();
            return;
        }

#if UNITY_STANDALONE_WIN && !UNITY_EDITOR
        if (webUiWindowHandle == IntPtr.Zero && (hasPendingWebUiWindowSize || webUiWindowProcess != null))
        {
            if (Time.unscaledTime >= nextWebUiWindowHandleSearchTime)
            {
                nextWebUiWindowHandleSearchTime = Time.unscaledTime + WebUiWindowHandleSearchInterval;
                if (TryFindWebUiWindowHandle(out var foundHandle))
                {
                    webUiWindowHandle = foundHandle;
                }
            }
        }

        if (hasPendingWebUiWindowSize)
        {
            if (webUiWindowHandle != IntPtr.Zero)
            {
                ApplyWebUiWindowSize(webUiWindowHandle, pendingWebUiWindowWidth, pendingWebUiWindowHeight);
                if (TryGetWebUiWindowSize(webUiWindowHandle, out var appliedWidth, out var appliedHeight))
                {
                    if (Mathf.Abs(appliedWidth - pendingWebUiWindowWidth) <= WebUiWindowSizeTolerance
                        && Mathf.Abs(appliedHeight - pendingWebUiWindowHeight) <= WebUiWindowSizeTolerance)
                    {
                        hasPendingWebUiWindowSize = false;
                    }
                }
            }

            if (hasPendingWebUiWindowSize && Time.unscaledTime >= pendingWebUiWindowSizeUntilTime)
            {
                hasPendingWebUiWindowSize = false;
            }
        }

        if (Time.unscaledTime < nextWebUiWindowSizeSampleTime)
            return;

        nextWebUiWindowSizeSampleTime = Time.unscaledTime + WebUiWindowSizeSampleInterval;

        if (webUiWindowHandle != IntPtr.Zero && TryGetWebUiWindowSize(webUiWindowHandle, out var width, out var height))
        {
            if (!hasLastSavedWebUiWindowSize || width != lastSavedWebUiWindowWidth || height != lastSavedWebUiWindowHeight)
            {
                SaveWebUiWindowSize(width, height);
                lastSavedWebUiWindowWidth = width;
                lastSavedWebUiWindowHeight = height;
                hasLastSavedWebUiWindowSize = true;
            }
        }
#endif
    }

    void OnDestroy()
    {
        DestroyTray();
    }

    void OnApplicationQuit()
    {
        DestroyTray();
    }

    void OnDisable()
    {
        DestroyTray();
    }

    public void CreateTray()
    {
#if !UNITY_EDITOR
        if(trayForm == null)
        {
            trayForm = new TrayForm(trayIconTex); //CreateIcon(trayIconTex));

            trayForm.onExitCallback += OnExit;
        }
#endif
    }

    public void SetOpenWebUiAction(Action action)
    {
        if (!hasTrayIcon)
            return;

        if (trayForm == null)
            CreateTray();

        if (trayForm != null)
            trayForm.SetOpenWebUiCallback(action);
    }

    public void SetOpenWebUiBrowserAction(Action action)
    {
        if (!hasTrayIcon)
            return;

        if (trayForm == null)
            CreateTray();

        if (trayForm != null)
            trayForm.SetOpenWebUiBrowserCallback(action);
    }

    public void DestroyTray()
    {
        if (webUiWindowProcess != null)
        {
            try
            {
                if (!webUiWindowProcess.HasExited)
                {
                    if (TryGetWebUiWindowSize(webUiWindowProcess, out var width, out var height))
                    {
                        SaveWebUiWindowSize(width, height);
                    }
                    webUiWindowProcess.CloseMainWindow();
                    if (!webUiWindowProcess.WaitForExit(1500))
                    {
                        webUiWindowProcess.Kill();
                        webUiWindowProcess.WaitForExit(1500);
                    }
                }
            }
            catch (Exception ex)
            {
                Debug.LogWarning($"Failed to close Web UI app window: {ex.Message}");
            }
            finally
            {
                webUiWindowProcess.Dispose();
                webUiWindowProcess = null;
            }
        }

        if (webUiWindowHandle != IntPtr.Zero)
        {
            PostMessage(webUiWindowHandle, WM_CLOSE, IntPtr.Zero, IntPtr.Zero);
            webUiWindowHandle = IntPtr.Zero;
        }

        if (trayForm != null)
        {
            trayForm.Dispose();
            trayForm = null;
        }
    }

    public void OnExit()
    {
        quitRequestedFromTray = true;
    }

    public void ShowTrayIcon()
    {
        if (trayForm != null)
            trayForm.ShowTray();
    }

    public bool TryShowWebUiWindow(string url)
    {
#if UNITY_STANDALONE_WIN && !UNITY_EDITOR
        if (string.IsNullOrWhiteSpace(url))
            return false;

        if (webUiWindowProcess != null)
        {
            try
            {
                if (!webUiWindowProcess.HasExited)
                {
                    webUiWindowProcess.Refresh();
                    var handle = webUiWindowProcess.MainWindowHandle;
                    if (handle != IntPtr.Zero)
                    {
                        GetSavedWebUiWindowSize(out var savedWidth, out var savedHeight);
                        SetPendingWebUiWindowSize(savedWidth, savedHeight);
                        ApplyWebUiWindowSize(handle, savedWidth, savedHeight);
                        ShowWindow(handle, SW_RESTORE);
                        SetForegroundWindow(handle);
                    }
                    return true;
                }
            }
            catch (Exception ex)
            {
                Debug.LogWarning($"Failed to focus existing Web UI window: {ex.Message}");
            }
            finally
            {
                webUiWindowProcess.Dispose();
                webUiWindowProcess = null;
            }
        }

        if (TryFindWebUiWindowHandle(out var existingHandle))
        {
            webUiWindowHandle = existingHandle;
            GetSavedWebUiWindowSize(out var savedWidth, out var savedHeight);
            SetPendingWebUiWindowSize(savedWidth, savedHeight);
            ApplyWebUiWindowSize(existingHandle, savedWidth, savedHeight);
            ShowWindow(existingHandle, SW_RESTORE);
            SetForegroundWindow(existingHandle);
            return true;
        }

        GetSavedWebUiWindowSize(out var width, out var height);
        SetPendingWebUiWindowSize(width, height);

        if (TryLaunchBrowserAppWindow("msedge.exe", url, width, height, out var edgeProcess))
        {
            webUiWindowProcess = edgeProcess;
            webUiWindowHandle = IntPtr.Zero;
            return true;
        }

        if (TryLaunchBrowserAppWindow("chrome.exe", url, width, height, out var chromeProcess))
        {
            webUiWindowProcess = chromeProcess;
            webUiWindowHandle = IntPtr.Zero;
            return true;
        }

        return false;
#else
        return false;
#endif
    }

    private static bool TryLaunchBrowserAppWindow(string browserExe, string url, int width, int height, out System.Diagnostics.Process process)
    {
        process = null;
        try
        {
            var startInfo = new System.Diagnostics.ProcessStartInfo
            {
                FileName = browserExe,
                Arguments = $"--app=\"{url}\" --window-size={width},{height}",
                UseShellExecute = true
            };

            process = System.Diagnostics.Process.Start(startInfo);
            return process != null;
        }
        catch (Exception ex)
        {
            Debug.LogWarning($"Failed to start {browserExe} as app window: {ex.Message}");
            process?.Dispose();
            process = null;
            return false;
        }
    }

    private static void GetSavedWebUiWindowSize(out int width, out int height)
    {
        width = DefaultWebUiWindowWidth;
        height = DefaultWebUiWindowHeight;

        var sizeFilePath = GetWebUiWindowSizeFilePath();
        if (!File.Exists(sizeFilePath))
            return;

        try
        {
            var text = File.ReadAllText(sizeFilePath).Trim();
            if (string.IsNullOrWhiteSpace(text))
                return;

            var parts = text.Split(',');
            if (parts.Length != 2)
                return;

            if (!int.TryParse(parts[0], out var parsedWidth) || !int.TryParse(parts[1], out var parsedHeight))
                return;

            width = Mathf.Max(MinWebUiWindowWidth, parsedWidth);
            height = Mathf.Max(MinWebUiWindowHeight, parsedHeight);
        }
        catch (Exception ex)
        {
            Debug.LogWarning($"Failed to load Web UI window size: {ex.Message}");
        }
    }

    private static void SaveWebUiWindowSize(System.Diagnostics.Process process)
    {
        if (TryGetWebUiWindowSize(process, out var width, out var height))
        {
            SaveWebUiWindowSize(width, height);
        }
    }

    private static string GetWebUiWindowSizeFilePath()
    {
        var profilesRootPath = Path.Combine(UnityEngine.Application.persistentDataPath, ProfilesRootFolderName);
        return Path.Combine(profilesRootPath, WebUiWindowSizeFileName);
    }

    private static void SaveWebUiWindowSize(int width, int height)
    {
        if (width < MinWebUiWindowWidth || height < MinWebUiWindowHeight)
            return;

        var profilesRootPath = Path.Combine(UnityEngine.Application.persistentDataPath, ProfilesRootFolderName);
        if (!Directory.Exists(profilesRootPath))
        {
            Directory.CreateDirectory(profilesRootPath);
        }

        var sizeFilePath = GetWebUiWindowSizeFilePath();
        File.WriteAllText(sizeFilePath, $"{width},{height}");
    }

    private static bool TryGetWebUiWindowSize(System.Diagnostics.Process process, out int width, out int height)
    {
        width = 0;
        height = 0;
        process.Refresh();
        var handle = process.MainWindowHandle;
        if (handle == IntPtr.Zero)
            return false;

        return TryGetWebUiWindowSize(handle, out width, out height);
    }

    private static bool TryGetWebUiWindowSize(IntPtr handle, out int width, out int height)
    {
        width = 0;
        height = 0;

        if (!GetWindowRect(handle, out var rect))
            return false;

        width = rect.Right - rect.Left;
        height = rect.Bottom - rect.Top;
        return width >= MinWebUiWindowWidth && height >= MinWebUiWindowHeight;
    }

    private static void ApplyWebUiWindowSize(IntPtr handle, int width, int height)
    {
        if (width < MinWebUiWindowWidth || height < MinWebUiWindowHeight)
            return;

        ShowWindow(handle, SW_RESTORE);
        SetWindowPos(handle, 0, 0, 0, width, height, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
    }

    private void SetPendingWebUiWindowSize(int width, int height)
    {
        pendingWebUiWindowWidth = width;
        pendingWebUiWindowHeight = height;
        pendingWebUiWindowSizeUntilTime = Time.unscaledTime + WebUiWindowApplyTimeout;
        hasPendingWebUiWindowSize = true;
        nextWebUiWindowHandleSearchTime = 0f;
    }

    private static bool TryFindWebUiWindowHandle(out IntPtr handle)
    {
        handle = IntPtr.Zero;
        IntPtr foundHandle = IntPtr.Zero;
        EnumWindows((hWnd, lParam) =>
        {
            if (!IsWindowVisible(hWnd))
                return true;

            var length = GetWindowTextLength(hWnd);
            if (length <= 0)
                return true;

            var builder = new StringBuilder(length + 1);
            if (GetWindowText(hWnd, builder, builder.Capacity) <= 0)
                return true;

            if (builder.ToString().IndexOf(WebUiWindowTitle, StringComparison.OrdinalIgnoreCase) >= 0)
            {
                foundHandle = hWnd;
                return false;
            }

            return true;
        }, IntPtr.Zero);

        handle = foundHandle;
        return handle != IntPtr.Zero;
    }

    public bool HideTaskbarIcon()
    {
        bool res = false;

        res = ShowWindow(winHandle, (uint)0);
        SetWindowLong(winHandle, GWL_EXSTYLE, GetWindowLong(winHandle, GWL_EXSTYLE) | WS_EX_TOOLWINDOW);
        res = ShowWindow(winHandle, (uint)5);

        return res;
    }

    public bool MinimizeUnityWindow()
    {
        bool res = ShowWindow(winHandle, (uint)6);
        return res;
    }

    public bool HideUnityWindow()
    {
        bool res = false;

        res = MinimizeUnityWindow();
        res = HideTaskbarIcon();

        return res;
    }
}

public class TrayForm : System.Windows.Forms.Form
{
    public delegate void OnExitDel();
    public delegate void OnShowWindowDel();

    public OnExitDel onExitCallback;
    public OnShowWindowDel onShowWindow;
    public Action onOpenWebUi;
    public Action onOpenWebUiBrowser;

    private System.Windows.Forms.ContextMenuStrip trayMenu;
    private NotifyIcon trayIcon;
    private ToolStripMenuItem openWebUiItem;
    private ToolStripMenuItem openWebUiItemBrowser;

    public TrayForm(Texture2D tex = null)
    {
        trayMenu = new System.Windows.Forms.ContextMenuStrip();

        openWebUiItem = new ToolStripMenuItem("Open Web UI", CreateBitmap(Texture2D.whiteTexture));
        openWebUiItemBrowser = new ToolStripMenuItem("Open Web UI (Browser)", CreateBitmap(Texture2D.whiteTexture));
        openWebUiItem.Enabled = false;
        openWebUiItemBrowser.Enabled = false;
        openWebUiItem.Click += OnOpenWebUi;
        openWebUiItemBrowser.Click += OnOpenWebUiBrowser;
        trayMenu.Items.Add(openWebUiItem);
        trayMenu.Items.Add(openWebUiItemBrowser);
        trayMenu.Items.Add(new ToolStripSeparator());

        trayMenu.Items.Add("Exit", CreateBitmap(Texture2D.whiteTexture), OnExit);

        trayIcon = new NotifyIcon();
        trayIcon.Text = "Steameeter";
        trayIcon.MouseDoubleClick += OnTrayIconMouseDoubleClick;

        trayIcon.ContextMenuStrip = trayMenu;

        if (tex == null)
            trayIcon.Icon = new Icon(SystemIcons.Application, 40, 40);
        else
            trayIcon.Icon = Icon.FromHandle(CreateBitmap(tex).GetHicon());
    }

    public Bitmap CreateBitmap(Texture2D tex)
    {
        MemoryStream memS = new MemoryStream(tex.EncodeToPNG());
        memS.Seek(0, System.IO.SeekOrigin.Begin);

        Bitmap bitmap = new Bitmap(memS);

        return bitmap;
    }

    protected override void OnLoad(EventArgs e)
    {
        Visible = false;
        ShowInTaskbar = false;

        base.OnLoad(e);
    }

    ~TrayForm()
    {
        Dispose(true);
    }

    public void ShowTray()
    {
        trayIcon.Visible = true;
    }

    public void HideTray()
    {
        trayIcon.Visible = false;
    }

    protected void OnExit(object sender, EventArgs e)
    {
        onExitCallback.Invoke();
    }

    public void SetOpenWebUiCallback(Action callback)
    {
        onOpenWebUi = callback;

        if (openWebUiItem != null)
            openWebUiItem.Enabled = callback != null;
    }

    public void SetOpenWebUiBrowserCallback(Action callback)
    {
        onOpenWebUiBrowser = callback;

        if (openWebUiItemBrowser != null)
            openWebUiItemBrowser.Enabled = callback != null;
    }

    protected void OnOpenWebUi(object sender, EventArgs e)
    {
        onOpenWebUi?.Invoke();
    }

    protected void OnOpenWebUiBrowser(object sender, EventArgs e)
    {
        onOpenWebUiBrowser?.Invoke();
    }

    private void OnTrayIconMouseDoubleClick(object sender, MouseEventArgs e)
    {
        if (e.Button == MouseButtons.Left)
        {
            onOpenWebUi?.Invoke();
        }
    }

    protected override void Dispose(bool disposing)
    {
        if (disposing)
            trayIcon.Dispose();

        base.Dispose(disposing);
    }
}
