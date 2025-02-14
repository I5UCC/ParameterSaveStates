using System.Collections;
using UnityEngine;
using Valve.VR;

public partial class OVR_Handler 
{
    public OpenVRChange onOpenVRChange = delegate(bool connected){};
    public StandbyChange onStandbyChange = delegate(bool inStandbyMode){};
    public DashboardChange onDashboardChange = delegate(bool open){};
    public ChaperoneChange onChaperoneChange = delegate(){};
    public KeyboardInput onKeyboardInput = delegate(string input){};


    private bool PollNextEvent(ref VREvent_t pEvent)
    {
        if(VRSystem == null)
            return false;

		var size = (uint)System.Runtime.InteropServices.Marshal.SizeOf(typeof(Valve.VR.VREvent_t));
		return VRSystem.PollNextEvent(ref pEvent, size);
    }

    public delegate void OpenVRChange(bool connected);
    public delegate void StandbyChange(bool inStandbyMode);
    public delegate void DashboardChange(bool open);

    public delegate void KeyboardInput(string input);

    public delegate void ChaperoneChange();

    private void DigestEvent(VREvent_t pEvent) 
    {
        EVREventType type = (EVREventType) pEvent.eventType;
        switch(type)
        {
            case EVREventType.VREvent_Quit:
                Debug.Log("VR - QUIT - EVENT");
                onOpenVRChange(false);
            break;
            
            case EVREventType.VREvent_DashboardActivated:
                onDashboardChange(true);
            break;
            case EVREventType.VREvent_DashboardDeactivated:
                onDashboardChange(false);
            break;

            case EVREventType.VREvent_EnterStandbyMode:
                onStandbyChange(true);
            break;
            case EVREventType.VREvent_LeaveStandbyMode:
                onStandbyChange(false);
            break;

            case EVREventType.VREvent_KeyboardCharInput:
                string txt = "";
                var kd = pEvent.data.keyboard;
                byte[] bytes = new byte[]
                {
                    kd.cNewInput0,
                    kd.cNewInput1,
                    kd.cNewInput2,
                    kd.cNewInput3,
                    kd.cNewInput4,
                    kd.cNewInput5,
                    kd.cNewInput6,
                    kd.cNewInput7,
                };
                int len = 0;
                while(bytes[len++] != 0 && len < 7);
                string input = System.Text.Encoding.UTF8.GetString(bytes, 0, len);

                System.Text.StringBuilder txtB = new System.Text.StringBuilder(1024);
                Overlay.GetKeyboardText(txtB, 1024);
                txt = txtB.ToString();

                onKeyboardInput(txt);
            break;

            case EVREventType.VREvent_ChaperoneSettingsHaveChanged:
                onChaperoneChange();
            break;
        }

        onVREvent.Invoke(pEvent);
    }
}