import sys

import gi

gi.require_version("Gtk", "3.0")
gi.require_version("WebKit2", "4.1")
from gi.repository import GLib, Gtk, WebKit2

view = WebKit2.WebView()
window = Gtk.Window()
window.add(view)
window.show_all()
print("webkit2gtk %d.%d.%d" % (WebKit2.get_major_version(), WebKit2.get_minor_version(), WebKit2.get_micro_version()), flush=True)


def changed(*_):
    if view.get_title() == "probe-done":
        GLib.timeout_add(500, Gtk.main_quit)


view.connect("notify::title", changed)
view.load_uri(sys.argv[1])
GLib.timeout_add_seconds(40, Gtk.main_quit)
Gtk.main()
