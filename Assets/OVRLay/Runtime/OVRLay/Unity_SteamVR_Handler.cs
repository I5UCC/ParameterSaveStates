using System.Collections;
using System.Collections.Generic;
using UnityEngine;
using UnityEngine.Events;

using Valve.VR;

public class Unity_SteamVR_Handler : MonoBehaviour 
{
	public float steamVRPollTime = 0.5f;

	public bool connectedToSteam = false;

	[Space(10)]

	public GameObject hmdObject;
	public GameObject rightTrackerObj;
	public GameObject leftTrackerObj;

	[Space(10)]

	public bool autoUpdate = true;

	[Space(10)]

	public bool debugLog = false;

	[Space(10)]
	[Header("Performance")]
	[Space(10)]
	public int activeTargetFrameRate = 30;
	public int idleTargetFrameRate = 1;
	public bool throttleWhenDashboardClosed = true;

	[Space(10)]

	public UnityEvent onSteamVRConnect = new UnityEvent();
	public UnityEvent onSteamVRDisconnect = new UnityEvent();

	[Space(10)]

	public UnityEvent onDashboardOpen = new UnityEvent();
	public UnityEvent onDashboardClose = new UnityEvent();

	public UnityEvent<string> onKeyboardInput = new UnityEvent<string>();


	[HideInInspector] public OVR_Handler ovrHandler = OVR_Handler.instance;

	[HideInInspector] public OVR_Overlay_Handler overlayHandler { get { return ovrHandler.overlayHandler; } }
	[HideInInspector] public OVR_Pose_Handler poseHandler { get { return ovrHandler.poseHandler; } }

	private float lastSteamVRPollTime = 0f;
	private bool isDashboardOpen = true;

	void Start()
	{
		// Will always do a check on start first, then use timer for polling
		lastSteamVRPollTime = steamVRPollTime + 1f;
		ovrHandler.onOpenVRChange += OnOpenVRChange;

		UpdateTargetFrameRate();

		ovrHandler.onVREvent += VREventHandler;
	}

	void OnOpenVRChange(bool connected) 
	{
		connectedToSteam = connected;
		UpdateTargetFrameRate();

		if(!connected)
		{
			onSteamVRDisconnect.Invoke();
			ovrHandler.ShutDownOpenVR();
		}
			
	}

	void OnDashboardChange(bool open)
	{
		isDashboardOpen = open;
		UpdateTargetFrameRate();
		if(open)
			onDashboardOpen.Invoke();
		else
			onDashboardClose.Invoke();
	}

	void OnKeyboardInput(string input)
	{
		onKeyboardInput.Invoke(input);
	}

	void Update() 
	{
		if(autoUpdate)
			UpdateHandler();
	}

	public void UpdateHandler()
	{
		if(!SteamVRStartup())
			return;

		ovrHandler.UpdateAll();

		if(hmdObject)
			poseHandler.SetTransformToTrackedDevice(hmdObject.transform, poseHandler.hmdIndex);

		if(poseHandler.rightActive && rightTrackerObj)
		{
			rightTrackerObj.SetActive(true);
			poseHandler.SetTransformToTrackedDevice(rightTrackerObj.transform, poseHandler.rightIndex);
		}
		else if(rightTrackerObj)
			rightTrackerObj.SetActive(false);
		
		if(poseHandler.leftActive && leftTrackerObj)
		{
			leftTrackerObj.SetActive(true);
			poseHandler.SetTransformToTrackedDevice(leftTrackerObj.transform, poseHandler.leftIndex);
		}
		else if(leftTrackerObj)
			leftTrackerObj.SetActive(false);
	}

	public void VREventHandler(VREvent_t e)
	{
		if(debugLog)
			Debug.Log("VR Event: " + e);
	}

	bool SteamVRStartup()
	{
		lastSteamVRPollTime += Time.deltaTime;

		if(ovrHandler.OpenVRConnected)
			return true;
		else if(lastSteamVRPollTime >= steamVRPollTime)
		{
			lastSteamVRPollTime = 0f;

			if (debugLog)
				Debug.Log("Checking to see if SteamVR Is Running...");
			if(System.Diagnostics.Process.GetProcessesByName("vrserver").Length <= 0)
			{
				if (debugLog)
					Debug.Log("VRServer not Running!");
				return false;
			}

			if (debugLog)
				Debug.Log("Starting Up SteamVR Connection...");

			if( !ovrHandler.StartupOpenVR() )
			{
				if (debugLog)
					Debug.Log("Connection Failed :( !");
				return false;
			}
			else
			{
				if (debugLog)
					Debug.Log("Connected to SteamVR!");
				
				onSteamVRConnect.Invoke();
				ovrHandler.onDashboardChange += OnDashboardChange;
				ovrHandler.onKeyboardInput += OnKeyboardInput;

				return true;
			}		
		}
		else
			return false;
	}
	void OnApplicationQuit()
	{
		if(ovrHandler.OpenVRConnected)
			ovrHandler.ShutDownOpenVR();
	}

	void UpdateTargetFrameRate()
	{
		if(!connectedToSteam)
		{
			Application.targetFrameRate = idleTargetFrameRate;
			return;
		}

		if(!throttleWhenDashboardClosed)
		{
			Application.targetFrameRate = activeTargetFrameRate;
			return;
		}

		Application.targetFrameRate = isDashboardOpen ? activeTargetFrameRate : idleTargetFrameRate;
	}
}
