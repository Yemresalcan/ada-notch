"""Embeds the application icon and version information into a Windows .exe.

The GNU toolchain has no `windres`, so instead of compiling a resource script
this uses the Win32 resource-update API (BeginUpdateResource / UpdateResource /
EndUpdateResource) on the finished executable. The version comes from
Cargo.toml next to this script's parent directory.

Usage: python tools/set_icon.py <exe> <ico>
"""
import ctypes
import re
import struct
import sys
from ctypes import wintypes
from pathlib import Path

RT_ICON = 3
RT_GROUP_ICON = 14
RT_VERSION = 16
LANG_NEUTRAL = 0
LANG_EN_US = 0x0409
CODEPAGE_UNICODE = 0x04B0

PRODUCT = "Ada"
COMPANY = "Yunus Emre Salcan"
DESCRIPTION = "Ada - Dynamic Island-style notch for Windows"
COPYRIGHT = "Copyright 2026 Yunus Emre Salcan. Licensed under the Apache License 2.0."

kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
kernel32.BeginUpdateResourceW.argtypes = [wintypes.LPCWSTR, wintypes.BOOL]
kernel32.BeginUpdateResourceW.restype = wintypes.HANDLE
kernel32.UpdateResourceW.argtypes = [wintypes.HANDLE, wintypes.LPVOID, wintypes.LPVOID, wintypes.WORD, wintypes.LPVOID, wintypes.DWORD]
kernel32.UpdateResourceW.restype = wintypes.BOOL
kernel32.EndUpdateResourceW.argtypes = [wintypes.HANDLE, wintypes.BOOL]
kernel32.EndUpdateResourceW.restype = wintypes.BOOL


def read_ico(path):
    data = open(path, "rb").read()
    _reserved, kind, count = struct.unpack_from("<HHH", data, 0)
    if kind != 1:
        raise ValueError("not an icon file")
    entries = []
    for i in range(count):
        w, h, colors, _r, planes, bpp, size, offset = struct.unpack_from("<BBBBHHII", data, 6 + 16 * i)
        entries.append((w, h, colors, planes, bpp, data[offset:offset + size]))
    return entries


def cargo_version():
    toml = (Path(__file__).resolve().parent.parent / "Cargo.toml").read_text(encoding="utf-8")
    match = re.search(r'^version\s*=\s*"(\d+)\.(\d+)\.(\d+)', toml, re.MULTILINE)
    if not match:
        raise ValueError("version not found in Cargo.toml")
    return tuple(int(x) for x in match.groups())


# ---- VS_VERSIONINFO (https://learn.microsoft.com/windows/win32/menurc/vs-versioninfo) ----

def _pad4(data):
    return data + b"\0" * (-len(data) % 4)


def _node(key, value=b"", value_len=0, text=True, children=()):
    """One version-resource block: wLength, wValueLength, wType, key, value, children (DWORD-aligned)."""
    out = struct.pack("<HHH", 0, value_len, 1 if text else 0) + (key + "\0").encode("utf-16-le")
    out = _pad4(out) + value
    for child in children:
        out = _pad4(out) + child
    return struct.pack("<H", len(out)) + out[2:]


def _string(key, text):
    return _node(key, (text + "\0").encode("utf-16-le"), len(text) + 1)


def version_resource(major, minor, patch):
    ms, ls = (major << 16) | minor, patch << 16
    fixed = struct.pack(
        "<13I",
        0xFEEF04BD,  # signature
        0x00010000,  # struct version
        ms, ls,  # file version
        ms, ls,  # product version
        0x3F,  # flags mask
        0,  # flags
        0x00040004,  # VOS_NT_WINDOWS32
        0x1,  # VFT_APP
        0, 0, 0,  # subtype, date
    )
    version = f"{major}.{minor}.{patch}"
    strings = [
        _string("CompanyName", COMPANY),
        _string("FileDescription", DESCRIPTION),
        _string("FileVersion", f"{version}.0"),
        _string("InternalName", "ada"),
        _string("LegalCopyright", COPYRIGHT),
        _string("OriginalFilename", "ada.exe"),
        _string("ProductName", PRODUCT),
        _string("ProductVersion", version),
    ]
    table = _node(f"{LANG_EN_US:04X}{CODEPAGE_UNICODE:04X}", children=strings)
    string_info = _node("StringFileInfo", children=[table])
    translation = _node("Translation", struct.pack("<HH", LANG_EN_US, CODEPAGE_UNICODE), 4, text=False)
    var_info = _node("VarFileInfo", children=[translation])
    return _node("VS_VERSION_INFO", fixed, len(fixed), text=False, children=[string_info, var_info])


def update(handle, rtype, rid, payload, lang=LANG_NEUTRAL):
    buf = ctypes.create_string_buffer(payload, len(payload))
    ok = kernel32.UpdateResourceW(handle, ctypes.c_void_p(rtype), ctypes.c_void_p(rid), lang, buf, len(payload))
    if not ok:
        raise ctypes.WinError(ctypes.get_last_error())


def main(exe, ico):
    entries = read_ico(ico)
    major, minor, patch = cargo_version()
    handle = kernel32.BeginUpdateResourceW(exe, False)
    if not handle:
        raise ctypes.WinError(ctypes.get_last_error())
    try:
        group = struct.pack("<HHH", 0, 1, len(entries))
        for i, (w, h, colors, planes, bpp, image) in enumerate(entries, start=1):
            update(handle, RT_ICON, i, image)
            group += struct.pack("<BBBBHHIH", w, h, colors, 0, planes, bpp, len(image), i)
        # Group id 1: the icon Explorer shows for the exe (and LoadIcon(hinst, 1)).
        update(handle, RT_GROUP_ICON, 1, group)
        update(handle, RT_VERSION, 1, version_resource(major, minor, patch), LANG_EN_US)
    except Exception:
        kernel32.EndUpdateResourceW(handle, True)
        raise
    if not kernel32.EndUpdateResourceW(handle, False):
        raise ctypes.WinError(ctypes.get_last_error())
    print(f"icon ({len(entries)} images) and version {major}.{minor}.{patch} embedded -> {exe}")


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    main(sys.argv[1], sys.argv[2])
