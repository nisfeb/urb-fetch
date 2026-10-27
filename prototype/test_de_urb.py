"""de_urb against the cases in lattice's tests/lib/lattice-urls.hoon.
Run: LIBMESA=... LIBMESA_PY=... python3 -I test_de_urb.py (see the README)"""
import os, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from urb_fetch import de_urb, looks_html, UrbError

ok = {
    "urb://~zod": ["index"],
    "urb://~zod/": ["index"],
    "urb://~zod/site/": ["site"],
    "urb://~zod/notes/2026/intro": ["notes", "2026", "intro"],
    "urb://~zod/n/notes": ["notes"],
}
for url, rel in ok.items():
    assert de_urb(url) == ("~zod", rel), (url, de_urb(url))
for url in ("https://example.com", "urb://notaship/x", "urb://~notaship/x", "urb://~zod/x/foo",
            "urb://~zod/p/counter", "urb://~zod/t/apps/x", "urb://~zod/k/user/prefs"):
    try:
        de_urb(url)
        raise AssertionError("accepted " + url)
    except UrbError:
        pass
assert looks_html("  \n<!doctype html>") and not looks_html("# heading\n<b>")
print("ok")
