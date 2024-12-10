from cx_Freeze import setup, Executable

packages = ["pythonosc", "zeroconf", "requests", "flask", "jinja2", "dataclasses"]
exclude = []
includes = ["flask", "jinja2.ext", "jinja2"]
file_include = ['templates']
bin_excludes = ["_bz2.pyd", "_decimal.pyd", "_hashlib.pyd", "_lzma.pyd", "_queue.pyd", "_ssl.pyd", "libcrypto-1_1.dll", "libssl-1_1.dll", "ucrtbase.dll", "VCRUNTIME140.dll"]

build_exe_options = {"packages": packages, "excludes": exclude, "include_files": file_include, "bin_excludes": bin_excludes, "includes": includes}

setup(
    name="ParameterSaveStates",
    version="1.0",
    description="ParameterSaveStates",
    options={"build_exe": build_exe_options},
    executables=[Executable("app.py", target_name="ParameterSaveStates.exe", base=False), Executable("app.py", target_name="ParameterSaveStates_NoConsole.exe", base="Win32GUI")],
)