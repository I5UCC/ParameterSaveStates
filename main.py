# load "C:\Users\ROOT\AppData\LocalLow\VRChat\VRChat\LocalAvatarData\usr_8f47b443-8066-45ab-ad12-3e559a40d8ab\avtr_e8315b74-9edb-4a30-8905-26d873c98c44" as json and beautify the output
import json
import os
from pythonosc import udp_client

AVATAR_PARAMETERS_PREFIX = "/avatar/parameters/"
AVATAR_CHANGE_PARAMETER = "/avatar/change"

# osc client
osc = udp_client.SimpleUDPClient("127.0.0.1", 9000)

path = os.path.expanduser("~") + "\\AppData\\LocalLow\\VRChat\\VRChat\\LocalAvatarData\\usr_8f47b443-8066-45ab-ad12-3e559a40d8ab\\avtr_e8315b74-9edb-4a30-8905-26d873c98c44"
with open(path) as f:
    data = json.load(f)["animationParameters"]
    print(json.dumps(data, indent=4))
    for param in data:
        value = param["value"]
        parameter = AVATAR_PARAMETERS_PREFIX + param["name"]
        
        if value == 0:
            value = 0.0
        value = round(value, 2)

        print(parameter + ":\t\t\t" + str(value))
        # send over osc
        osc.send_message(parameter, value)
