"""Embeds a .ico file into a Windows .exe as its application icon.

The GNU toolchain has no `windres`, so instead of compiling a resource script
this uses the Win32 resource-update API (BeginUpdateResource / UpdateResource /
EndUpdateResource) on the finished executable.

Usage: python tools/set_icon.py <exe> <ico>
"""
import ctypes
import struct
import sys
from ctypes import wintypes

RT_ICON = 3
RT_GROUP_ICON = 14
LANG_NEUTRAL = 0

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


def update(handle, rtype, rid, payload):
    buf = ctypes.create_string_buffer(payload, len(payload))
    ok = kernel32.UpdateResourceW(handle, ctypes.c_void_p(rtype), ctypes.c_void_p(rid), LANG_NEUTRAL, buf, len(payload))
    if not ok:
        raise ctypes.WinError(ctypes.get_last_error())


def main(exe, ico):
    entries = read_ico(ico)
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
    except Exception:
        kernel32.EndUpdateResourceW(handle, True)
        raise
    if not kernel32.EndUpdateResourceW(handle, False):
        raise ctypes.WinError(ctypes.get_last_error())
    print(f"icon embedded: {len(entries)} images -> {exe}")


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    main(sys.argv[1], sys.argv[2])
