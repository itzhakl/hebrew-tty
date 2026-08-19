#!/usr/bin/env python3
"""Take a screenshot through the desktop portal and save it where asked.

GNOME refuses org.gnome.Shell.Screenshot to ordinary callers, but the
xdg-desktop-portal Screenshot API answers non-interactively. This is the only
way to check on this machine what a terminal actually painted, which matters
because a description of RTL text cannot be trusted - reading it back reorders
it again. Crop to a single glyph and identify that instead.

    python3 tools/screenshot.py /tmp/shot.png
"""
import gi, secrets, sys, shutil, os
gi.require_version('Gio','2.0')
from gi.repository import Gio, GLib
def shot(dest):
    bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
    token = 'rtl' + secrets.token_hex(4)
    sender = bus.get_unique_name()[1:].replace('.','_')
    path = f"/org/freedesktop/portal/desktop/request/{sender}/{token}"
    loop = GLib.MainLoop(); res={}
    bus.signal_subscribe('org.freedesktop.portal.Desktop','org.freedesktop.portal.Request',
        'Response', path, None, Gio.DBusSignalFlags.NONE,
        lambda *a: (res.update(r=a[5].unpack()), loop.quit()))
    bus.call_sync('org.freedesktop.portal.Desktop','/org/freedesktop/portal/desktop',
        'org.freedesktop.portal.Screenshot','Screenshot',
        GLib.Variant('(sa{sv})', ('', {'handle_token': GLib.Variant('s', token),
                                       'interactive': GLib.Variant('b', False)})),
        None, Gio.DBusCallFlags.NONE, 8000, None)
    GLib.timeout_add_seconds(15, lambda: (loop.quit(), False)[1])
    loop.run()
    r = res.get('r')
    if not r or r[0] != 0: raise SystemExit(f"screenshot failed: {r}")
    src = r[1]['uri'].replace('file://','')
    shutil.copy(src, dest); os.remove(src)
    print(dest)
if __name__ == '__main__': shot(sys.argv[1])
