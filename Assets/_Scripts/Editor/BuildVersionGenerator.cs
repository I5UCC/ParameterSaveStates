#if UNITY_EDITOR
using System;
using System.IO;
using System.Text;
using UnityEditor;
using UnityEditor.Build;
using UnityEditor.Build.Reporting;
using UnityEngine;

/// <summary>
/// Keeps Resources/BuildVersion.txt synced with pipeline-provided version value.
/// </summary>
public sealed class BuildVersionGenerator : IPreprocessBuildWithReport
{
    private const string RelativeVersionFilePath = "Assets/Resources/BuildVersion.txt";
    private const string FallbackVersion = "dev";
    private const string VersionEnvVar = "PSS_BUILD_VERSION";

    public int callbackOrder => 0;

    public void OnPreprocessBuild(BuildReport report)
    {
        var projectRoot = Directory.GetParent(Application.dataPath)?.FullName;
        if (string.IsNullOrWhiteSpace(projectRoot))
        {
            return;
        }

        var versionFile = Path.Combine(projectRoot, RelativeVersionFilePath.Replace('/', Path.DirectorySeparatorChar));
        var existing = File.Exists(versionFile) ? File.ReadAllText(versionFile).Trim() : string.Empty;
        var version = ResolveVersion();

        if (string.Equals(existing, version, StringComparison.Ordinal))
        {
            return;
        }

        Directory.CreateDirectory(Path.GetDirectoryName(versionFile) ?? string.Empty);
        File.WriteAllText(versionFile, version + Environment.NewLine, Encoding.UTF8);
        AssetDatabase.ImportAsset(RelativeVersionFilePath, ImportAssetOptions.ForceUpdate);
        UnityEngine.Debug.Log($"Build version synced from pipeline value: {version}");
    }

    private static string ResolveVersion()
    {
        var value = Environment.GetEnvironmentVariable(VersionEnvVar);
        if (!string.IsNullOrWhiteSpace(value))
        {
            return value.Trim();
        }

        return FallbackVersion;
    }
}
#endif
