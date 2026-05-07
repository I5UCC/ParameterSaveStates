using UnityEngine;
using UnityEngine.UI;

public static class ProfileObjectExtensions
{
    private const string EditContainerName = "Edit_Container";
    private const string MoveButtonContainerName = "Move_Button_Container";
    private const string EditButtonName = "Edit";

    private const int EDIT_CONTAINER_FALLBACK_INDEX = 3;
    private const int EDIT_BUTTON_FALLBACK_INDEX = 2;
    
    private const int CONTAINER_DELETE_TEXT_INDEX = 0;
    private const int CONTAINER_OVERRIDE_TEXT_INDEX = 1;
    private const int CONTAINER_RENAME_TEXT_INDEX = 2;
    
    private const int TEXT_INDEX = 0;
    private const int DISPLAY_TEXT_INDEX = 0;

    public static Transform GetEditContainer(this GameObject gameObject)
    {
        return GetNamedChildOrFallback(gameObject.transform, EditContainerName, EDIT_CONTAINER_FALLBACK_INDEX);
    }

    public static Transform GetMoveButtonContainer(this GameObject gameObject)
    {
        return gameObject.transform.Find(MoveButtonContainerName);
    }
    
    public static Text GetDeleteText(this GameObject gameObject)
    {
        return gameObject
            .GetEditContainer()
            .GetChild(CONTAINER_DELETE_TEXT_INDEX)
            .GetButtonTextComponent();
    }
    
    public static Text GetOverrideProfileText(this GameObject gameObject)
    {
        return gameObject
            .GetEditContainer()
            .GetChild(CONTAINER_OVERRIDE_TEXT_INDEX)
            .GetButtonTextComponent();
    }
    
    public static Text GetRenameText(this GameObject gameObject)
    {
        return gameObject
            .GetEditContainer()
            .GetChild(CONTAINER_RENAME_TEXT_INDEX)
            .GetButtonTextComponent();
    }
    
    public static Text GetDisplayNameText(this GameObject gameObject)
    {
        return gameObject.transform.GetChild(DISPLAY_TEXT_INDEX).GetComponent<Text>();
    }

    public static Transform GetEditButton(this GameObject gameObject)
    {
        return GetNamedChildOrFallback(gameObject.transform, EditButtonName, EDIT_BUTTON_FALLBACK_INDEX);
    }

    public static Text GetEditButtonText(this GameObject gameObject)
    {
        return gameObject.GetEditButton().GetButtonTextComponent();
    }
    
    public static Text GetButtonTextComponent(this Transform transform)
    {
        return transform.GetChild(TEXT_INDEX).GetComponent<Text>();
    }
    
    public static Image GetImageComponent(this Transform transform)
    {
        return transform.GetComponent<Image>();
    }

    public static void ResetEditButtons(this GameObject gameObject)
    {
        var deleteButtonText = gameObject.GetDeleteText();
        deleteButtonText.text = "Delete";
        
        var overrideButtonText = gameObject.GetOverrideProfileText();
        overrideButtonText.text = "Override";
        
        var renameButtonText = gameObject.GetRenameText();
        renameButtonText.text = "Rename";
    }

    private static Transform GetNamedChildOrFallback(Transform parent, string childName, int fallbackIndex = -1)
    {
        var namedChild = parent.Find(childName);
        if (namedChild != null)
        {
            return namedChild;
        }

        if (fallbackIndex >= 0 && fallbackIndex < parent.childCount)
        {
            return parent.GetChild(fallbackIndex);
        }

        throw new MissingReferenceException($"Unable to find child '{childName}' under '{parent.name}'.");
    }
}
