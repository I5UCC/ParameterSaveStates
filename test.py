import time
from tinyoscquery.queryservice import OSCQueryService
from tinyoscquery.utility import get_open_tcp_port, get_open_udp_port, check_if_tcp_port_open, check_if_udp_port_open
from tinyoscquery.query import OSCQueryBrowser, OSCQueryClient
import requests
from pythonosc import udp_client, dispatcher, osc_server
from threading import Thread
import logging
import sys
import os
from parameter import Parameter
import json
import dataclasses

def get_absolute_path(relative_path) -> str:
    """
    Gets absolute path relative to the executable.
    Parameters:
        relative_path (str): Relative path
    Returns:
        str: Absolute path
    """
    base_path = getattr(sys, '_MEIPASS', os.path.dirname(os.path.abspath(__file__)))
    return os.path.join(base_path, relative_path)

logging.basicConfig(level=logging.DEBUG if len(sys.argv) > 1 else logging.INFO, format='%(asctime)s - %(levelname)s - %(message)s', datefmt='%d-%b-%y %H:%M:%S', handlers=[logging.StreamHandler(), logging.FileHandler(get_absolute_path("log.log"))])

AVATAR_PARAMETERS_PREFIX = "/avatar/parameters/"
AVATAR_CHANGE_PARAMETER = "/avatar/change"

current_avatar = None
osc_client = udp_client.SimpleUDPClient("127.0.0.1", "9000")
http_port = get_open_tcp_port()
server_port = get_open_udp_port()

def osc_server_serve():
    server.serve_forever(2)

def avatar_changed(address, *args):
    global current_avatar
    if current_avatar == args[0]:
        return
    current_avatar = args[0]

    logging.info(f"Avatar changed to {current_avatar}!")

service_info = None
logging.info("Waiting for VRChat to be discovered.")
while service_info is None:
    browser = OSCQueryBrowser()
    time.sleep(2) # Wait for discovery
    service_info = browser.find_service_by_name("VRChat")
logging.info("VRChat discovered!")
client = OSCQueryClient(service_info)
logging.info("Waiting for VRChat to be ready.")
while client.query_node(AVATAR_CHANGE_PARAMETER) is None:
    time.sleep(2)
logging.info("VRChat ready!")

current_avatar = client.query_node(AVATAR_CHANGE_PARAMETER).value[0]
logging.info(f"Current avatar: {current_avatar}")

disp = dispatcher.Dispatcher()
disp.map(AVATAR_CHANGE_PARAMETER, avatar_changed)

server = osc_server.ThreadingOSCUDPServer(("127.0.0.1", server_port), disp)

server_thread = Thread(target=osc_server_serve, daemon=True)
server_thread.start()
oscqs = OSCQueryService("test", http_port, server_port)
oscqs.advertise_endpoint(AVATAR_CHANGE_PARAMETER, access="readwrite")

root = client._get_query_root()
logging.info("Root:" + root)

def get_params():
    global root
    params = requests.get(root).json()["CONTENTS"]["avatar"]["CONTENTS"]["parameters"]["CONTENTS"]
    # filter res items to only have readwrite access
    # ACCESS 3 is readwrite
    # ACCESS 2 is write
    # ACCESS 1 is read
    # Access 0 is none
    return {key: value for key, value in params.items() if value["ACCESS"] == 3}

def save():
    global root, current_avatar

    params = get_params()
    paramlist = [Parameter(key, value["VALUE"][0]) for key, value in params.items()]

    for p in paramlist:
        print(p)

    logging.info("Saving parameters...")
    # save parameters to file
    with open(get_absolute_path(f"saves/{current_avatar}"), "w") as f:
        json.dump([dataclasses.asdict(p) for p in paramlist], f, indent=4)
    logging.info("Parameters saved!")

def load(load_from):
    global root, current_avatar

    if load_from == current_avatar:
        copy_to_current(load_from)

    load_from_path = get_absolute_path(f"saves/{load_from}")

    logging.info(f"Loading parameters from {load_from}...")
    with open(load_from_path, "r") as f:
        # load parameters from file into Paramter objects
        paramlist = [Parameter(**p) for p in json.load(f)]
    
    for p in paramlist:
        osc_client.send_message(f"{AVATAR_PARAMETERS_PREFIX}{p.name}", p.value)

    logging.info("Parameters loaded!")

def copy_to_current(copy_from):
    global current_avatar

    copy_from_path = get_absolute_path(f"saves/{copy_from}")
    current_avatar_path = get_absolute_path(f"saves/{current_avatar}")

    logging.info(f"Copying parameters from {copy_from} to {current_avatar}...")
    with open(copy_from_path, "r") as f:
        data = json.load(f)

    with open(current_avatar_path, "w") as f:
        json.dump(data, f, indent=4)
    logging.info("Parameters copied!")
